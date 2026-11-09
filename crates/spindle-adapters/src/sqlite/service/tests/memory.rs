use super::*;
use spindle_core::memory::{MemoryLayerV1, MemorySummaryStatusV1, MemoryVisibilityPolicyV1};

async fn save_chapter_scene(
    svc: &SqliteSpindleService,
    project_id: &str,
    chapter: i32,
    summary: &str,
    text: &str,
) -> String {
    if chapter > 1 {
        let _ = svc
            .create_chapter(spindle_core::models::CreateChapterInput {
                project_id: project_id.to_string(),
                book_id: None,
                book_number: Some(1),
                chapter_number: Some(chapter),
                title: None,
            })
            .await;
    }
    svc.save_scene_draft(spindle_core::models::SaveSceneDraftInput {
        project_id: project_id.to_string(),
        book_number: 1,
        chapter_number: chapter,
        chapter_id: None,
        scene_order: 1,
        full_text: text.to_string(),
        summary: summary.to_string(),
        content_rating: spindle_core::models::ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap()
    .scene_id
}

#[tokio::test]
async fn rebuild_creates_four_layers_and_cursor_excludes_later_summaries() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, _scene_id, _) = project_with_scene(&svc, "Layered Memory").await;
    save_chapter_scene(
        &svc,
        &project.project_id,
        2,
        "CHAPTER TWO MEMORY",
        "Chapter two detail.",
    )
    .await;
    save_chapter_scene(
        &svc,
        &project.project_id,
        3,
        "CHAPTER THREE MEMORY",
        "Chapter three detail.",
    )
    .await;
    let report = svc
        .rebuild_layered_memory_for_project(&project.project_id)
        .await
        .unwrap();
    assert_eq!(report.scene_count, 3);
    assert_eq!(report.chapter_count, 3);
    assert_eq!(report.arc_count, 1);
    assert_eq!(report.book_count, 1);
    assert!(report.dependency_count >= 6);
    assert_eq!(report.stale_count, 0);

    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let summaries = svc
        .repository()
        .list_layered_memory(&project.project_id, &branch.id)
        .await
        .unwrap();
    assert_eq!(
        summaries
            .iter()
            .filter(|summary| summary.layer == MemoryLayerV1::Book)
            .count(),
        1
    );
    let selection = svc
        .layered_memory_selection(
            &project.project_id,
            &branch.id,
            &spindle_core::models::StoryPlacement {
                book_number: 1,
                chapter_number: 3,
                scene_order: Some(1),
                note: None,
            },
            &MemoryVisibilityPolicyV1::default(),
        )
        .await
        .unwrap();
    assert!(
        selection
            .items
            .iter()
            .all(|summary| summary.layer == MemoryLayerV1::Chapter),
        "book/arc summaries span chapter 3 and must fall back to chapters: {:?}",
        selection
            .items
            .iter()
            .map(|summary| (&summary.layer, summary.coverage.end.chapter_number))
            .collect::<Vec<_>>()
    );
    let chapters: Vec<i32> = selection
        .items
        .iter()
        .map(|summary| summary.coverage.start.chapter_number)
        .collect();
    assert_eq!(chapters, vec![1, 2]);

    let budgeted = svc
        .layered_memory_budgeted_context(
            &project.project_id,
            &branch.id,
            &spindle_core::models::StoryPlacement {
                book_number: 1,
                chapter_number: 3,
                scene_order: Some(1),
                note: None,
            },
            &MemoryVisibilityPolicyV1::default(),
            &spindle_core::memory::MemoryBudgetPolicyV1 {
                total_tokens: 120,
                hard_constraint_tokens: 20,
                layer_caps: std::collections::BTreeMap::from([
                    (MemoryLayerV1::Chapter, 60),
                    (MemoryLayerV1::Scene, 20),
                ]),
            },
        )
        .await
        .unwrap();
    assert!(budgeted.hard_constraints_preserved);
    assert!(!budgeted.budget_exceeded);
    assert!(budgeted.estimated_tokens <= 120);

    let story = svc
        .story_so_far(&project.project_id, &branch.id, 1, 3, 1)
        .await
        .unwrap();
    assert!(story.contains("opening scene") || story.contains("CHAPTER TWO MEMORY"));
    assert!(!story.contains("CHAPTER THREE MEMORY"));
}

#[tokio::test]
async fn scene_edit_marks_scene_and_ancestors_stale_and_rebuild_clears() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Layered Stale").await;
    save_chapter_scene(&svc, &project.project_id, 2, "chapter two", "detail").await;
    svc.rebuild_layered_memory_for_project(&project.project_id)
        .await
        .unwrap();
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();

    // Editing the first scene replaces its row, then marks its summary and
    // every ancestor stale through the dependency DAG.
    svc.save_scene_draft(spindle_core::models::SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Revised opening scene with a new detail.".into(),
        summary: "revised opening scene".into(),
        content_rating: spindle_core::models::ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    let _scene = svc.repository().get_scene(&scene_id).await.unwrap();
    let stale = svc
        .repository()
        .list_layered_memory(&project.project_id, &branch.id)
        .await
        .unwrap();
    assert!(stale.iter().any(|summary| {
        summary.layer == MemoryLayerV1::Scene
            && matches!(summary.status, MemorySummaryStatusV1::Stale { .. })
    }));
    assert!(stale.iter().any(|summary| {
        summary.layer == MemoryLayerV1::Book
            && matches!(summary.status, MemorySummaryStatusV1::Stale { .. })
    }));
    assert!(
        svc.repository()
            .has_pending_memory_recompute_jobs(&project.project_id, &branch.id)
            .await
            .unwrap()
    );

    let report = svc
        .recompute_layered_memory(&project.project_id, &branch.id)
        .await
        .unwrap();
    assert_eq!(report.stale_count, 0);
    assert!(
        !svc.repository()
            .has_pending_memory_recompute_jobs(&project.project_id, &branch.id)
            .await
            .unwrap()
    );
    let fresh = svc
        .repository()
        .list_layered_memory(&project.project_id, &branch.id)
        .await
        .unwrap();
    assert!(
        fresh
            .iter()
            .all(|summary| summary.status == MemorySummaryStatusV1::Fresh)
    );
}

#[tokio::test]
async fn story_so_far_keeps_completed_previous_book_when_cursor_is_later_book() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, _scene_id, _) = project_with_scene(&svc, "Multi Book Memory").await;
    // Book 1 chapter 1 has an existing summary; add a book 2 scene at ch 3.
    let _ = svc
        .create_chapter(spindle_core::models::CreateChapterInput {
            project_id: project.project_id.clone(),
            book_id: None,
            book_number: Some(2),
            chapter_number: Some(3),
            title: None,
        })
        .await;
    svc.save_scene_draft(spindle_core::models::SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: 2,
        chapter_number: 3,
        chapter_id: None,
        scene_order: 1,
        full_text: "Book two detail.".into(),
        summary: "BOOK TWO MEMORY".into(),
        content_rating: spindle_core::models::ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.rebuild_layered_memory_for_project(&project.project_id)
        .await
        .unwrap();
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let story = svc
        .story_so_far(&project.project_id, &branch.id, 2, 3, 1)
        .await
        .unwrap();
    assert!(
        story.contains("opening scene") || story.contains("opening"),
        "completed previous book must survive chapter-only selection: {story}"
    );
    assert!(!story.contains("BOOK TWO MEMORY"));
}

#[tokio::test]
async fn commit_scene_edit_invalidates_layered_memory() {
    use spindle_core::models::{
        CommitSceneContinuityPackage, CommitSceneInput, ContentRating, SceneAuthorship,
    };
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Layered Commit").await;
    svc.rebuild_layered_memory_for_project(&project.project_id)
        .await
        .unwrap();
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    svc.commit_scene(CommitSceneInput {
        project_id: project.project_id.clone(),
        branch_id: None,
        preparation_id: None,
        context_revision_token: None,
        dependency_tokens: None,
        scene_id: Some(scene_id),
        book_number: None,
        chapter_number: None,
        scene_order: None,
        expected_scene_version: None,
        full_text: "The atomic commit path rewrote this scene.".into(),
        summary: "atomic commit revision".into(),
        content_rating: ContentRating::General,
        location_id: None,
        tone: None,
        authorship: SceneAuthorship::Human,
        generation_receipt: None,
        continuity: CommitSceneContinuityPackage::default(),
        beat_annotations: Vec::new(),
        chapter_summary: None,
        run_id: None,
        expected_phase: None,
        dry_run: false,
        idempotency_key: None,
    })
    .await
    .unwrap();
    let stale = svc
        .repository()
        .list_layered_memory(&project.project_id, &branch.id)
        .await
        .unwrap();
    assert!(stale.iter().any(|summary| {
        summary.layer == MemoryLayerV1::Scene
            && matches!(summary.status, MemorySummaryStatusV1::Stale { .. })
    }));
    assert!(stale.iter().any(|summary| {
        summary.layer == MemoryLayerV1::Book
            && matches!(summary.status, MemorySummaryStatusV1::Stale { .. })
    }));
    assert!(
        svc.repository()
            .has_pending_memory_recompute_jobs(&project.project_id, &branch.id)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn secret_summary_is_excluded_by_visibility_policy() {
    use spindle_core::models::{RegisterCanonicalFactInput, SecrecyScope};
    let (_tmp, svc) = fresh_service_local().await;
    let (project, seed_scene_id, _) = project_with_scene(&svc, "Layered Secrets").await;
    let character_id = create_character_named(&svc, &project.project_id, "Veyra").await;
    let character_id_for_holder = character_id.clone();
    let fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(seed_scene_id),
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("secret_identity".into()),
            key: Some("passphrase".into()),
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(character_id),
            predicate: Some("passphrase".into()),
            value_kind: Some("text".into()),
            value_text: Some("obsidiancrown".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: Some(SecrecyScope {
                holder_ids: vec![character_id_for_holder.clone()],
                concealment_note: None,
            }),
        })
        .await
        .unwrap();
    svc.save_scene_draft(spindle_core::models::SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Veyra wrote quietly in her notebook.".into(),
        summary: "Veyra copied obsidiancrown into her notebook.".into(),
        content_rating: spindle_core::models::ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.rebuild_layered_memory_for_project(&project.project_id)
        .await
        .unwrap();
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let mut hidden = std::collections::BTreeSet::new();
    hidden.insert(fact.canonical_fact_id);
    let policy = MemoryVisibilityPolicyV1 {
        reader_visible_only: false,
        allowed_secret_fact_ids: std::collections::BTreeSet::new(),
        hidden_secret_fact_ids: hidden,
    };
    let selection = svc
        .layered_memory_selection(
            &project.project_id,
            &branch.id,
            &spindle_core::models::StoryPlacement {
                book_number: 1,
                chapter_number: 2,
                scene_order: Some(1),
                note: None,
            },
            &policy,
        )
        .await
        .unwrap();
    assert!(selection.secret_filtered_count >= 1);
}

#[tokio::test]
async fn secret_only_in_chapter_and_book_summary_text_is_labeled() {
    use spindle_core::models::{RegisterCanonicalFactInput, SaveSummaryInput, SecrecyScope};
    let (_tmp, svc) = fresh_service_local().await;
    let (project, seed_scene_id, _) = project_with_scene(&svc, "Summary Secrets").await;
    let holder = create_character_named(&svc, &project.project_id, "Veyra").await;
    let fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(seed_scene_id),
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("secret_identity".into()),
            key: Some("passphrase".into()),
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(holder.clone()),
            predicate: Some("passphrase".into()),
            value_kind: Some("text".into()),
            value_text: Some("obsidiancrown".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: Some(SecrecyScope {
                holder_ids: vec![holder],
                concealment_note: None,
            }),
        })
        .await
        .unwrap();
    svc.save_summary(SaveSummaryInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        entity_type: None,
        entity_id: None,
        summary: "The passphrase obsidiancrown appears only in this chapter summary.".into(),
        key_events: Vec::new(),
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    })
    .await
    .unwrap();
    svc.rebuild_layered_memory_for_project(&project.project_id)
        .await
        .unwrap();
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let summaries = svc
        .repository()
        .list_layered_memory(&project.project_id, &branch.id)
        .await
        .unwrap();
    for layer in [MemoryLayerV1::Chapter, MemoryLayerV1::Book] {
        assert!(
            summaries.iter().any(|summary| {
                summary.layer == layer && summary.secret_fact_ids.contains(&fact.canonical_fact_id)
            }),
            "{layer:?} summary text must label the secret: {summaries:?}"
        );
    }
    let mut hidden = std::collections::BTreeSet::new();
    hidden.insert(fact.canonical_fact_id.clone());
    let selection = svc
        .layered_memory_selection(
            &project.project_id,
            &branch.id,
            &spindle_core::models::StoryPlacement {
                book_number: 1,
                chapter_number: 2,
                scene_order: Some(1),
                note: None,
            },
            &MemoryVisibilityPolicyV1 {
                reader_visible_only: false,
                allowed_secret_fact_ids: std::collections::BTreeSet::new(),
                hidden_secret_fact_ids: hidden,
            },
        )
        .await
        .unwrap();
    assert!(
        selection
            .items
            .iter()
            .all(|summary| !summary.secret_fact_ids.contains(&fact.canonical_fact_id)),
        "hidden summary text must never be selected: {:?}",
        selection.items
    );
}

#[tokio::test]
async fn entity_state_is_retrievable_without_embedding_provider() {
    use spindle_core::models::RegisterCanonicalFactInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, seed_scene_id, _) = project_with_scene(&svc, "Layered Entity").await;
    let character_id = create_character_named(&svc, &project.project_id, "Veyra").await;
    for (predicate, value) in [
        ("Veyra current location", "market"),
        ("Veyra injury", "broken rib"),
        ("Veyra possession", "sword"),
    ] {
        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(seed_scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("character_attribute".into()),
            key: Some(predicate.into()),
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(character_id.clone()),
            predicate: Some(predicate.into()),
            value_kind: Some("text".into()),
            value_text: Some(value.into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();
    }
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let entity = svc
        .retrieve_entity_state_layered(
            &project.project_id,
            &branch.id,
            &character_id,
            Some(&spindle_core::models::StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            }),
        )
        .await
        .unwrap();
    assert!(entity.is_available_without_embedding());
    assert_eq!(entity.subject_id, character_id);
    assert!(entity.names.iter().any(|name| name == "Veyra"));
    assert!(
        entity
            .location
            .as_deref()
            .is_some_and(|location| location.contains("market"))
    );
    assert!(entity.injuries.iter().any(|injury| injury.contains("rib")));
    assert!(
        entity
            .possessions
            .iter()
            .any(|possession| possession.contains("sword"))
    );
}
