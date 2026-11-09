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
async fn same_spec_migrates_two_projects_independently() {
    let (_tmp, svc) = fresh_service_local().await;
    let (a, _seed_a, _) = project_with_scene(&svc, "Embedding Project A").await;
    let (b, _seed_b, _) = project_with_scene(&svc, "Embedding Project B").await;
    seed_documents(&svc, &a.project_id).await;
    seed_documents(&svc, &b.project_id).await;

    let spec = fixture_spec(384);
    let out_a = svc
        .rebuild_embedding_generation(rebuild_input(&a.project_id, 384))
        .await
        .unwrap();
    assert!(out_a.activated);
    assert_eq!(out_a.indexed_records, 2);
    // Project A now owns an active global registry row for this spec. Project B
    // must still build its own rows and move its own pointer.
    let out_b = svc
        .rebuild_embedding_generation(rebuild_input(&b.project_id, 384))
        .await
        .unwrap();
    assert!(
        out_b.activated,
        "second project must not short-circuit: {out_b:?}"
    );
    assert_eq!(out_b.indexed_records, 2);
    assert_eq!(out_b.generation_id, spec.generation_id());
    for project in [&a.project_id, &b.project_id] {
        let active = svc.active_embedding_generation(project).await.unwrap();
        assert_eq!(active.generation_id, spec.generation_id());
        assert_eq!(
            svc.repository()
                .embedding_row_count(project, &spec.generation_id())
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            semantic_first(&svc, project).await[0].entity_id,
            "character:veyra"
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
    let first = rows
        .iter()
        .find(|row| row.entity_id == "character:veyra")
        .expect("source row");
    let document = SearchDocument {
        entity_table: first.entity_table.clone(),
        title: first.title.clone(),
        excerpt: first.excerpt.clone(),
        content: first.content.clone(),
    };
    let active_branch_id = svc
        .repository()
        .active_branch_id_public(&project.project_id)
        .await
        .unwrap();
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

    // Edit the partially built entity through the active generation before the
    // restart. Resume must detect the changed content and re-embed, not keep the
    // stale target row.
    svc.repository()
        .upsert_search_embedding_document(
            &project.project_id,
            &active_branch_id,
            &first.entity_id,
            &SearchDocument {
                entity_table: first.entity_table.clone(),
                title: first.title.clone(),
                excerpt: "loyal border regiment updated".into(),
                content: "loyal border regiment updated cavalry garrison".into(),
            },
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
    let rebuilt = restarted
        .repository()
        .list_embeddings_for_generation(&project.project_id, &spec.generation_id())
        .await
        .unwrap();
    assert!(
        rebuilt.iter().any(|row| {
            row.entity_id == first.entity_id && row.content.contains("updated cavalry")
        }),
        "resume must refresh stale target content: {rebuilt:?}"
    );
    drop(tmp);
}

#[tokio::test]
async fn resume_is_branch_aware_for_the_same_entity_id() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, _seed, _) = project_with_scene(&svc, "Embedding Branches").await;
    seed_documents(&svc, &project.project_id).await;
    let parent = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let alternate = svc
        .create_branch(spindle_core::models::CreateBranchInput {
            project_id: project.project_id.clone(),
            name: "Alternate".into(),
            branch_type: "alternate".into(),
            description: None,
            parent_branch_id: Some(parent.id.clone()),
        })
        .await
        .unwrap();
    // Same entity id on another branch with different text. The uniqueness key
    // includes branch_id, so this row must not be skipped by a partial build on
    // the parent branch.
    svc.repository()
        .upsert_search_embedding_document(
            &project.project_id,
            &alternate.branch_id,
            "character:veyra",
            &SearchDocument {
                entity_table: "character".into(),
                title: "Veyra (alternate)".into(),
                excerpt: "ballroom".into(),
                content: "forbidden romance ballroom dance".into(),
            },
        )
        .await
        .unwrap();
    let active = svc
        .active_embedding_generation(&project.project_id)
        .await
        .unwrap();
    let spec = fixture_spec(384);
    let target = svc
        .repository()
        .create_embedding_generation(spec.clone())
        .await
        .unwrap();
    let rows = svc
        .repository()
        .list_embeddings_for_generation(&project.project_id, &active.generation_id)
        .await
        .unwrap();
    let parent_row = rows
        .iter()
        .find(|row| {
            row.branch_id.as_deref() == Some(parent.id.as_str())
                && row.entity_id == "character:veyra"
        })
        .expect("parent veyra row");
    let document = SearchDocument {
        entity_table: parent_row.entity_table.clone(),
        title: parent_row.title.clone(),
        excerpt: parent_row.excerpt.clone(),
        content: parent_row.content.clone(),
    };
    let vector = token_hash_embedding(&parent_row.content, 384);
    svc.repository()
        .upsert_embedding_document_for_generation(
            &project.project_id,
            parent_row.branch_id.as_deref(),
            &parent_row.entity_id,
            &document,
            &target,
            &vector,
        )
        .await
        .unwrap();
    let output = svc
        .rebuild_embedding_generation(rebuild_input(&project.project_id, 384))
        .await
        .unwrap();
    assert!(output.activated);
    assert_eq!(output.indexed_records, 3);
    let rebuilt = svc
        .repository()
        .list_embeddings_for_generation(&project.project_id, &spec.generation_id())
        .await
        .unwrap();
    assert!(
        rebuilt.iter().any(|row| {
            row.branch_id.as_deref() == Some(alternate.branch_id.as_str())
                && row.entity_id == "character:veyra"
                && row.content.contains("ballroom")
        }),
        "branch row must be rebuilt: {rebuilt:?}"
    );
}

#[tokio::test]
async fn live_source_edit_during_build_fails_activation_and_is_caught_up() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, _seed, _) = project_with_scene(&svc, "Embedding Live Edit").await;
    seed_documents(&svc, &project.project_id).await;
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let previous = svc
        .active_embedding_generation(&project.project_id)
        .await
        .unwrap();
    let spec = fixture_spec(384);
    let target = svc
        .repository()
        .create_embedding_generation(spec.clone())
        .await
        .unwrap();
    let source = svc
        .repository()
        .list_embeddings_for_generation(&project.project_id, &previous.generation_id)
        .await
        .unwrap();
    // Complete a target snapshot first.
    for row in &source {
        let document = SearchDocument {
            entity_table: row.entity_table.clone(),
            title: row.title.clone(),
            excerpt: row.excerpt.clone(),
            content: row.content.clone(),
        };
        let vector = token_hash_embedding(&row.content, 384);
        svc.repository()
            .upsert_embedding_document_for_generation(
                &project.project_id,
                row.branch_id.as_deref(),
                &row.entity_id,
                &document,
                &target,
                &vector,
            )
            .await
            .unwrap();
    }
    // A live edit bumps the source epoch but the target snapshot is stale.
    svc.repository()
        .upsert_search_embedding_document(
            &project.project_id,
            &branch.id,
            "character:veyra",
            &SearchDocument {
                entity_table: "character".into(),
                title: "Veyra".into(),
                excerpt: "updated".into(),
                content: "loyal border regiment updated live cavalry".into(),
            },
        )
        .await
        .unwrap();
    // A new source entity arrives after the target snapshot too.
    svc.repository()
        .upsert_search_embedding_document(
            &project.project_id,
            &branch.id,
            "character:new",
            &SearchDocument {
                entity_table: "character".into(),
                title: "New".into(),
                excerpt: "new source".into(),
                content: "a new source entity added during rebuild".into(),
            },
        )
        .await
        .unwrap();
    let epoch = svc
        .repository()
        .embedding_write_epoch(&project.project_id)
        .await
        .unwrap();
    assert!(epoch >= 2);
    // The activation-side epoch check refuses the stale snapshot.
    let error = svc
        .repository()
        .activate_embedding_generation(
            &project.project_id,
            &target.generation_id,
            &previous.generation_id,
            chrono::Utc::now().timestamp_micros() + 3_600_000_000,
            epoch - 1,
        )
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains(spindle_core::embeddings::EMBEDDING_WRITE_CONFLICT),
        "unexpected activation error: {error:#}"
    );
    assert_eq!(
        svc.active_embedding_generation(&project.project_id)
            .await
            .unwrap()
            .generation_id,
        previous.generation_id
    );
    // A retry sees the new epoch, refreshes the edited row, adds the new row,
    // and only then activates.
    let output = svc
        .rebuild_embedding_generation(rebuild_input(&project.project_id, 384))
        .await
        .unwrap();
    assert!(
        output.activated,
        "retry must catch up and activate: {output:?}"
    );
    assert_eq!(output.indexed_records, 3);
    let rebuilt = svc
        .repository()
        .list_embeddings_for_generation(&project.project_id, &spec.generation_id())
        .await
        .unwrap();
    assert_eq!(rebuilt.len(), 3);
    assert!(rebuilt.iter().any(|row| {
        row.entity_id == "character:veyra" && row.content.contains("updated live cavalry")
    }));
    assert!(rebuilt.iter().any(|row| row.entity_id == "character:new"));
    assert_eq!(
        semantic_first(&svc, &project.project_id).await[0].entity_id,
        "character:veyra"
    );
}

#[tokio::test]
async fn deletion_during_build_bumps_epoch_and_prunes_target() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, _seed, _) = project_with_scene(&svc, "Embedding Deletion").await;
    seed_documents(&svc, &project.project_id).await;
    let previous = svc
        .active_embedding_generation(&project.project_id)
        .await
        .unwrap();
    let (source, snapshot_epoch) = svc
        .repository()
        .embedding_source_snapshot(&project.project_id, &previous.generation_id)
        .await
        .unwrap();
    assert_eq!(source.len(), 2);
    let spec = fixture_spec(384);
    let target = svc
        .repository()
        .create_embedding_generation(spec.clone())
        .await
        .unwrap();
    // Complete a target snapshot, then delete one source entity through the
    // normal delete path.
    for row in &source {
        let document = SearchDocument {
            entity_table: row.entity_table.clone(),
            title: row.title.clone(),
            excerpt: row.excerpt.clone(),
            content: row.content.clone(),
        };
        let vector = token_hash_embedding(&row.content, 384);
        svc.repository()
            .upsert_embedding_document_for_generation(
                &project.project_id,
                row.branch_id.as_deref(),
                &row.entity_id,
                &document,
                &target,
                &vector,
            )
            .await
            .unwrap();
    }
    assert!(
        svc.repository()
            .delete_search_embedding_for_entity(&project.project_id, "character:poet")
            .await
            .unwrap()
    );
    let after_delete_epoch = svc
        .repository()
        .embedding_write_epoch(&project.project_id)
        .await
        .unwrap();
    assert!(
        after_delete_epoch > snapshot_epoch,
        "delete must bump the source epoch ({snapshot_epoch} -> {after_delete_epoch})"
    );
    // The stale target snapshot must not resurrect the deleted entity.
    let output = svc
        .rebuild_embedding_generation(rebuild_input(&project.project_id, 384))
        .await
        .unwrap();
    assert!(output.activated);
    assert_eq!(output.indexed_records, 1);
    let rebuilt = svc
        .repository()
        .list_embeddings_for_generation(&project.project_id, &spec.generation_id())
        .await
        .unwrap();
    assert_eq!(rebuilt.len(), 1);
    assert_eq!(rebuilt[0].entity_id, "character:veyra");
    assert_eq!(
        semantic_first(&svc, &project.project_id).await[0].entity_id,
        "character:veyra"
    );
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
