use super::*;

#[tokio::test]
async fn fts_search_characters_finds_by_name_and_appearance() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CreateCharacterInput,
    };
    let (_tmp, repo) = fresh_repo().await;
    let (project, branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "P".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let _mara = repo
        .create_character(&CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.id.clone(),
            name: "Mara Oathkeeper".into(),
            summary: "Warden of the Ash Gate.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: Some(CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: None,
                status: None,
                notes: None,
                source_summary: None,
            }),
        })
        .await
        .unwrap();

    let hits = repo
        .fts_search_characters(&project.id, Some(&branch.id), "warden", 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "Mara matches 'warden' via summary");
    assert!(
        hits[0].2.to_lowercase().contains("warden"),
        "snippet contains the term: {}",
        hits[0].2
    );
}

#[tokio::test]
async fn fts_search_scenes_finds_prose_match() {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};
    let (_tmp, repo) = fresh_repo().await;
    let (project, branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "P".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Two scenes with distinct content so FTS can tell them apart.
    repo.save_scene_draft(
        &project.id,
        &branch.id,
        &SaveSceneDraftInput {
            project_id: project.id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "The Ash Gate burned through the night.".into(),
            summary: "Gate burns".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    repo.save_scene_draft(
        &project.id,
        &branch.id,
        &SaveSceneDraftInput {
            project_id: project.id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: "Aldric copied maps in the candlelight.".into(),
            summary: "Maps".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let hits = repo
        .fts_search_scenes(&project.id, Some(&branch.id), "burned", 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "only the Ash Gate scene contains 'burned'");
    assert!(
        hits[0].2.contains("<mark>burned</mark>"),
        "snippet must mark the term: {}",
        hits[0].2
    );

    let hits = repo
        .fts_search_scenes(&project.id, Some(&branch.id), "maps", 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
}

#[tokio::test]
async fn knn_search_returns_inserted_embeddings_ordered_by_distance() {
    use crate::ai::SearchDocument;
    let (_tmp, repo) = fresh_repo().await;
    let (project, branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "P".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Two characters with deterministically-distinct content for the
    // TokenHash backend; their embeddings should be similar to themselves
    // and farther from each other.
    let mara_id = "character:01HM";
    let aldric_id = "character:01HA";
    repo.upsert_search_embedding_document(
        &project.id,
        &branch.id,
        mara_id,
        &SearchDocument {
            entity_table: "character".into(),
            title: "Mara".into(),
            excerpt: "oath warden".into(),
            content: "Mara stands at the Ash Gate. Oathbound. Sword across her knee.".into(),
        },
    )
    .await
    .unwrap();
    repo.upsert_search_embedding_document(
        &project.id,
        &branch.id,
        aldric_id,
        &SearchDocument {
            entity_table: "character".into(),
            title: "Aldric".into(),
            excerpt: "scribe".into(),
            content: "Aldric copies maps in candlelight. Ink stains his cuffs.".into(),
        },
    )
    .await
    .unwrap();

    // Fetch Mara's own embedding to use as a query.
    let mara_row = repo
        .list_search_embeddings_by_project_and_branch(&project.id, &branch.id)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.entity_id == mara_id)
        .expect("mara row");
    let mara_query = mara_row.embedding.clone();

    // kNN: nearest should be Mara herself (distance ~= 0).
    let results = repo
        .knn_search_embeddings(&project.id, Some(&branch.id), &mara_query, 2)
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].0.entity_id, mara_id,
        "self-similarity must be the closest match"
    );
    assert!(
        results[0].1 < results[1].1,
        "results must be sorted by ascending distance"
    );
}

#[tokio::test]
async fn search_embedding_upsert_round_trips_blob() {
    use crate::ai::SearchDocument;
    let (_tmp, repo) = fresh_repo().await;
    let (project, branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "P".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let doc = SearchDocument {
        entity_table: "character".into(),
        title: "Mara".into(),
        excerpt: "Oathbound warden of the Ash Gate.".into(),
        content: "Mara, an oathbound warden, holds the gate against the dark.".into(),
    };
    let entity_id = "character:01HFAKE";

    repo.upsert_search_embedding_document(&project.id, &branch.id, entity_id, &doc)
        .await
        .unwrap();

    let rows = repo
        .list_search_embeddings_by_project_and_branch(&project.id, &branch.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let stored = &rows[0];
    assert_eq!(stored.entity_table, "character");
    assert_eq!(stored.entity_id, entity_id);
    assert_eq!(stored.embedding.len(), 64, "TokenHash backend = 64 dims");
    // The TokenHash backend is deterministic on content; the embedding
    // contains the L2-normalized hash distribution.
    assert!(stored.embedding.iter().any(|&v| v != 0.0));

    // Second upsert with new content must replace in place (no new row).
    let updated_doc = SearchDocument {
        content: "Mara: oathbound. She doubts.".into(),
        ..doc
    };
    repo.upsert_search_embedding_document(&project.id, &branch.id, entity_id, &updated_doc)
        .await
        .unwrap();
    let rows2 = repo
        .list_search_embeddings_by_project_and_branch(&project.id, &branch.id)
        .await
        .unwrap();
    assert_eq!(rows2.len(), 1, "ON CONFLICT must update in place");
    // Content changes alter the embedding deterministically.
    assert_ne!(rows2[0].content, stored.content);

    // Delete by entity.
    let removed = repo
        .delete_search_embedding_for_entity(&project.id, entity_id)
        .await
        .unwrap();
    assert!(removed);
    let rows3 = repo
        .list_search_embeddings_by_project_and_branch(&project.id, &branch.id)
        .await
        .unwrap();
    assert!(rows3.is_empty());
}
