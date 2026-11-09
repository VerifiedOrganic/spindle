use super::*;
use crate::ai::SearchDocument;
use crate::sqlite::SqlitePool;
use spindle_core::embeddings::*;
use spindle_core::models::{SearchBibleInput, SearchBibleMode};

fn fixture_spec(dimension: usize) -> EmbeddingGenerationSpecV1 {
    EmbeddingGenerationSpecV1 {
        provider: EmbeddingProviderKindV1::TokenHash,
        model: "fixture-token-hash".into(),
        dimension,
        metric: EmbeddingMetricV1::Cosine,
        normalization: EmbeddingNormalizationV1::L2,
        version: format!("fixture-v1-{dimension}"),
    }
}

fn rebuild_input(project_id: &str, dimension: usize) -> RebuildEmbeddingGenerationInputV1 {
    RebuildEmbeddingGenerationInputV1 {
        project_id: project_id.to_string(),
        spec: fixture_spec(dimension),
        quality_probe: Some(EmbeddingQualityProbeV1 {
            query: "loyal border regiment".into(),
            expected_entity_id: "character:veyra".into(),
            max_rank: 1,
        }),
        rollback_window_seconds: 3_600,
    }
}

async fn seed_documents(svc: &SqliteSpindleService, project_id: &str) -> (String, String) {
    let branch = svc
        .repository()
        .get_active_branch(project_id)
        .await
        .unwrap();
    svc.repository()
        .upsert_search_embedding_document(
            project_id,
            &branch.id,
            "character:veyra",
            &SearchDocument {
                entity_table: "character".into(),
                title: "Veyra".into(),
                excerpt: "loyal border regiment".into(),
                content: "loyal border regiment military garrison commander".into(),
            },
        )
        .await
        .unwrap();
    svc.repository()
        .upsert_search_embedding_document(
            project_id,
            &branch.id,
            "character:poet",
            &SearchDocument {
                entity_table: "character".into(),
                title: "Poet".into(),
                excerpt: "forbidden romance ballroom".into(),
                content: "forbidden romance ballroom dance candlelight sonnet".into(),
            },
        )
        .await
        .unwrap();
    ("character:veyra".into(), "character:poet".into())
}

async fn semantic_first(
    svc: &SqliteSpindleService,
    project_id: &str,
) -> Vec<spindle_core::models::SearchBibleResultItem> {
    svc.search_bible(SearchBibleInput {
        project_id: project_id.to_string(),
        query: "loyal border regiment".into(),
        limit: Some(5),
        mode: Some(SearchBibleMode::Semantic),
        field: None,
        subject_table: None,
        format: None,
        budget_tokens: None,
    })
    .await
    .unwrap()
    .results
}

#[tokio::test]
async fn embedding_fixtures_64_384_1536_roundtrip_and_mixed_query_is_rejected() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, _seed, _) = project_with_scene(&svc, "Embedding Fixtures").await;
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    seed_documents(&svc, &project.project_id).await;

    for dimension in [64usize, 384, 1536] {
        let output = svc
            .rebuild_embedding_generation(rebuild_input(&project.project_id, dimension))
            .await
            .unwrap();
        assert!(output.activated, "dimension {dimension} must activate");
        assert!(output.quality_passed);
        assert_eq!(output.status, EmbeddingGenerationStatusV1::Active);
        assert_eq!(output.indexed_records, 2);
        let active = svc
            .active_embedding_generation(&project.project_id)
            .await
            .unwrap();
        assert_eq!(active.spec.dimension, dimension);
        assert_eq!(
            active.generation_id,
            fixture_spec(dimension).generation_id()
        );
        let hits = semantic_first(&svc, &project.project_id).await;
        assert_eq!(
            hits.first().map(|hit| hit.entity_id.as_str()),
            Some("character:veyra"),
            "dimension {dimension} retrieval quality: {hits:?}"
        );

        // A query vector from a different dimension is rejected before any
        // vec0 comparison, and a same-dimension vector tagged with another
        // generation is rejected by identity too.
        let wrong_dimension = if dimension == 64 { 384 } else { 64 };
        let wrong = EmbeddingQueryVectorV1::new(
            active.generation_id.clone(),
            wrong_dimension,
            vec![0.1; wrong_dimension],
        )
        .unwrap();
        let error = svc
            .repository()
            .knn_search_embeddings_for_generation(
                &project.project_id,
                Some(&branch.id),
                &active,
                &wrong,
                5,
            )
            .await
            .unwrap_err();
        assert!(
            format!("{error}").to_lowercase().contains("dimension"),
            "unexpected dimension error: {error}"
        );
        let mixed = EmbeddingQueryVectorV1::new(
            "embedding_generation:other",
            dimension,
            vec![0.1; dimension],
        )
        .unwrap();
        let error = svc
            .repository()
            .knn_search_embeddings_for_generation(
                &project.project_id,
                Some(&branch.id),
                &active,
                &mixed,
                5,
            )
            .await
            .unwrap_err();
        assert!(
            format!("{error}").to_lowercase().contains("generation"),
            "unexpected mixed-generation error: {error}"
        );
    }
}

#[tokio::test]
async fn interrupted_rebuild_restart_preserves_active_and_resumes() {
    let (tmp, svc) = fresh_service_local().await;
    let (project, _seed, _) = project_with_scene(&svc, "Embedding Restart").await;
    seed_documents(&svc, &project.project_id).await;
    let active_before = svc
        .active_embedding_generation(&project.project_id)
        .await
        .unwrap();
    assert_eq!(active_before.spec.dimension, 64);

    // Simulate an interrupted build: create the target generation and write
    // exactly one row, then leave it building.
    let spec = fixture_spec(384);
    let target = svc
        .repository()
        .create_embedding_generation(spec.clone())
        .await
        .unwrap();
    let rows = svc
        .repository()
        .list_embeddings_for_generation(&project.project_id, &active_before.generation_id)
        .await
        .unwrap();
    let first = rows.first().expect("source row");
    let document = SearchDocument {
        entity_table: first.entity_table.clone(),
        title: first.title.clone(),
        excerpt: first.excerpt.clone(),
        content: first.content.clone(),
    };
    let vector = token_hash_embedding(&first.content, 384);
    svc.repository()
        .upsert_embedding_document_for_generation(
            &project.project_id,
            first.branch_id.as_deref(),
            &first.entity_id,
            &document,
            &target,
            &vector,
        )
        .await
        .unwrap();

    // The serving index is still the old generation while the rebuild is
    // incomplete.
    let active_during = svc
        .active_embedding_generation(&project.project_id)
        .await
        .unwrap();
    assert_eq!(active_during.generation_id, active_before.generation_id);
    assert_eq!(
        semantic_first(&svc, &project.project_id).await[0].entity_id,
        "character:veyra"
    );

    // Restart the service against the same database and resume the same
    // generation. The already-written row is skipped, not duplicated.
    let db_path = svc.repository().pool().db_path().to_path_buf();
    let data_dir = svc.repository().data_dir().to_path_buf();
    drop(svc);
    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo = crate::sqlite::repository::Repository::with_model_router(
        pool,
        data_dir,
        crate::ai::ModelRouter::local_only(),
    );
    let restarted = SqliteSpindleService::new(repo);
    let output = restarted
        .rebuild_embedding_generation(rebuild_input(&project.project_id, 384))
        .await
        .unwrap();
    assert!(
        output.activated,
        "restart must resume and activate: {output:?}"
    );
    assert_eq!(output.generation_id, spec.generation_id());
    assert_eq!(output.indexed_records, 2);
    let active_after = restarted
        .active_embedding_generation(&project.project_id)
        .await
        .unwrap();
    assert_eq!(active_after.spec.dimension, 384);
    drop(tmp);
}

#[tokio::test]
async fn rollback_restores_previous_serving_generation() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, _seed, _) = project_with_scene(&svc, "Embedding Rollback").await;
    seed_documents(&svc, &project.project_id).await;
    let previous = svc
        .active_embedding_generation(&project.project_id)
        .await
        .unwrap();
    let output = svc
        .rebuild_embedding_generation(rebuild_input(&project.project_id, 64))
        .await
        .unwrap();
    assert!(output.activated);
    let rolled = svc
        .rollback_embedding_generation(&project.project_id)
        .await
        .unwrap();
    assert_eq!(rolled.active_generation_id, previous.generation_id);
    assert_eq!(
        svc.active_embedding_generation(&project.project_id)
            .await
            .unwrap()
            .generation_id,
        previous.generation_id
    );
    assert_eq!(
        semantic_first(&svc, &project.project_id).await[0].entity_id,
        "character:veyra"
    );
    // The window is one-shot: a second rollback has no previous generation.
    assert!(
        svc.rollback_embedding_generation(&project.project_id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn failed_http_generation_leaves_active_index_serving() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, _seed, _) = project_with_scene(&svc, "Embedding Failed HTTP").await;
    seed_documents(&svc, &project.project_id).await;
    let active_before = svc
        .active_embedding_generation(&project.project_id)
        .await
        .unwrap();
    let mut spec = fixture_spec(1536);
    spec.provider = EmbeddingProviderKindV1::Http;
    let error = svc
        .rebuild_embedding_generation(RebuildEmbeddingGenerationInputV1 {
            project_id: project.project_id.clone(),
            spec,
            quality_probe: None,
            rollback_window_seconds: 60,
        })
        .await
        .unwrap_err();
    assert!(
        format!("{error}").to_lowercase().contains("http"),
        "unexpected provider error: {error}"
    );
    assert_eq!(
        svc.active_embedding_generation(&project.project_id)
            .await
            .unwrap()
            .generation_id,
        active_before.generation_id
    );
    assert_eq!(
        semantic_first(&svc, &project.project_id).await[0].entity_id,
        "character:veyra"
    );
}
