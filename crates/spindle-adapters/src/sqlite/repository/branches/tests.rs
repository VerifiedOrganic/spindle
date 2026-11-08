use super::*;

#[tokio::test]
async fn create_and_switch_branch() {
    let (_tmp, repo) = fresh_repo().await;
    let input = CreateProjectInput {
        name: "Spindle".into(),
        project_type: "novel".into(),
        genre: "fantasy".into(),
        reader_contract: ReaderContract {
            promise: "p".into(),
            style_notes: Vec::new(),
            boundaries: Vec::new(),
        },
    };
    let (project, main, _book, _chapter) = repo.create_project(&input).await.unwrap();

    let feature = repo
        .create_branch(
            &project.id,
            &main.id,
            "feature-arc-2",
            "feature",
            Some("trying new ending".into()),
        )
        .await
        .unwrap();
    assert_eq!(feature.name, "feature-arc-2");
    assert_eq!(feature.parent_branch_id.as_deref(), Some(main.id.as_str()));

    let updated = repo
        .switch_active_branch(&project.id, &feature.id)
        .await
        .unwrap();
    assert_eq!(
        updated.active_branch_id.as_deref(),
        Some(feature.id.as_str())
    );

    let branches = repo.list_branches_by_project(&project.id).await.unwrap();
    assert_eq!(branches.len(), 2);

    let active = repo.get_active_branch(&project.id).await.unwrap();
    assert_eq!(active.id, feature.id);
}

/// `merge_branch_snapshot` applies the source-branch slice onto a
/// feature branch and re-stamps every row's `branch_id`.
///
/// Exercises all four merge paths in a single tx:
///   * scene INSERT (new position) AND UPDATE (existing position)
///   * character_state DELETE-then-INSERT at a fresh position
///   * relates_to DELETE-then-INSERT for an existing edge on the target
///   * pacing_tracker DELETE-then-INSERT for the target branch's arc
///
/// Verifies via list-by-branch reads that the target branch holds the
/// expected rows after merge and the source branch is untouched.
#[tokio::test]
async fn merge_branch_snapshot_applies_source_slice_to_target_branch() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, ContentRating,
        CreateCharacterArcInput, CreateCharacterInput, CreateRelationshipInput,
        SaveSceneDraftInput, StoryPlacement,
    };

    let (_tmp, repo) = fresh_repo().await;
    let (project, main_branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "Merge Source Project".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "ride".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Two characters live on the main branch (per-branch character model).
    let empty_voice = CharacterVoiceProfileData {
        vocabulary: Vec::new(),
        sentence_structure: Vec::new(),
        tics: Vec::new(),
        forbidden_words: Vec::new(),
        example_lines: Vec::new(),
        tone: None,
        established_in_scene_id: None,
        updated_at: None,
    };
    let empty_emotional = CharacterEmotionalProfileData {
        base_emotions: Default::default(),
        suppressed: Vec::new(),
        triggers: Vec::new(),
        defense_mechanisms: Vec::new(),
        flex_range: None,
    };
    let (mara, _, _, _) = repo
        .create_character(&CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.id.clone(),
            name: "Mara".into(),
            role: "protagonist".into(),
            realm: None,
            summary: "warden".into(),
            voice_profile: empty_voice.clone(),
            emotional_profile: empty_emotional.clone(),
            initial_state: None,
        })
        .await
        .unwrap();
    let (aldric, _, _, _) = repo
        .create_character(&CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.id.clone(),
            name: "Aldric".into(),
            role: "scribe".into(),
            realm: None,
            summary: "ink-stained".into(),
            voice_profile: empty_voice.clone(),
            emotional_profile: empty_emotional.clone(),
            initial_state: None,
        })
        .await
        .unwrap();

    // A target-branch baseline scene at (1,1,1) — will be UPDATED by merge.
    repo.save_scene_draft(
        &project.id,
        &main_branch.id,
        &SaveSceneDraftInput {
            project_id: project.id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "main: gate stands.".into(),
            summary: "main 1.1.1".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // A baseline pacing_tracker on the main branch — will be replaced.
    let arc = repo
        .create_character_arc(&CreateCharacterArcInput {
            project_id: project.id.clone(),
            character_id: mara.id.clone(),
            arc_type: "growth".into(),
            starting_state: "wary".into(),
            ending_state: "open".into(),
            milestones: Vec::new(),
            thematic_purpose: "trust".into(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();
    let original_tracker = repo
        .create_pacing_tracker(&project.id, &arc.id)
        .await
        .unwrap();
    assert_eq!(original_tracker.status, "on_track");
    assert_eq!(original_tracker.branch_id, main_branch.id);

    // A baseline relationship on the main branch — will be replaced.
    repo.create_relationship(
        &main_branch.id,
        &CreateRelationshipInput {
            character_a_id: mara.id.clone(),
            character_b_id: aldric.id.clone(),
            relationship_type: "ally".into(),
            initial_trust: 5,
            initial_tension: 0,
            dynamics: vec!["trusting".into()],
        },
    )
    .await
    .unwrap();

    // Now build a synthetic source-branch slice. We don't actually need
    // a second branch to exist for `merge_branch_snapshot` — it operates
    // on owned Vec<Scene>/etc passed in by the caller (the service
    // layer fetches them from a real source branch). But to keep the
    // FK invariants sane (scene references book/chapter, etc.), reuse
    // the main-branch row metadata for book_id/chapter_id and just
    // re-stamp branch_id on the way in.
    let main_scenes = repo
        .list_scenes_by_project_and_branch(&project.id, &main_branch.id)
        .await
        .unwrap();
    let baseline_scene = main_scenes
        .iter()
        .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 1))
        .unwrap()
        .clone();

    // Mutate the baseline scene to simulate a source-branch edit, and
    // synthesize a brand-new scene at (1,1,2) that will be INSERTed.
    let updated_scene = Scene {
        full_text: "source: gate held under siege.".into(),
        summary: "source 1.1.1".into(),
        ..baseline_scene.clone()
    };
    let new_scene = Scene {
        id: "scene:source-only-1-1-2".into(),
        scene_order: 2,
        full_text: "source: candle gutters.".into(),
        summary: "source 1.1.2".into(),
        ..baseline_scene.clone()
    };
    let source_scenes = vec![updated_scene, new_scene];

    // Source character_state at a fresh position (no existing row on
    // target at that position — so DELETE is a no-op and INSERT lands).
    let placement = StoryPlacement {
        book_number: 1,
        chapter_number: 1,
        scene_order: Some(3),
        note: None,
    };
    let synthetic_state = CharacterState {
        id: "character_state:source-1".into(),
        project_id: project.id.clone(),
        branch_id: "bible_branch:other".into(), // will be re-stamped
        character_id: mara.id.clone(),
        scene_id: None,
        book_number: placement.book_number,
        chapter_number: placement.chapter_number,
        scene_order: placement.scene_order.unwrap(),
        emotional_state: Default::default(),
        goals: vec!["hold the gate".into()],
        status: vec!["resolute".into()],
        notes: Vec::new(),
        source_summary: Some("source-snapshot".into()),
        created_at: chrono::Utc::now(),
    };

    // Source relationship: same edge, but with different trust/tension.
    let source_rel = crate::sqlite::records::RelatesTo {
        in_id: mara.id.clone(),
        out_id: aldric.id.clone(),
        branch_id: "bible_branch:other".into(), // will be re-stamped
        relationship_type: "rival".into(),
        trust: 1,
        tension: 7,
        dynamics: vec!["fraught".into()],
        reason: Some("the gate quarrel".into()),
        last_scene_id: None,
        updated_at: chrono::Utc::now(),
    };

    // Source pacing tracker: same arc, but with different progress.
    let source_tracker = crate::sqlite::records::PacingTracker {
        id: "pacing_tracker:source-1".into(),
        project_id: project.id.clone(),
        branch_id: "bible_branch:other".into(), // will be re-stamped
        character_arc_id: arc.id.clone(),
        per_book_budget: Default::default(),
        max_progress_per_chapter: Some(0.5),
        milestone_spacing: Some(2),
        sprint_allowance: Some(1),
        regression_budget: Some(0.1),
        current_progress: 0.75,
        budget_remaining: 0.25,
        velocity: "fast".into(),
        status: "ahead".into(),
        next_milestone: Some("crisis".into()),
        warnings: vec!["watch the trust slide".into()],
        updated_at: chrono::Utc::now(),
    };

    // Execute the merge: target is the main branch.
    repo.merge_branch_snapshot(
        &project.id,
        &main_branch.id,
        &source_scenes,
        &[synthetic_state],
        &[source_rel],
        &[source_tracker],
    )
    .await
    .unwrap();

    // Verify: target main now has 2 scenes, the existing one was UPDATED,
    // the new one was INSERTed under main_branch.id.
    let after_scenes = repo
        .list_scenes_by_project_and_branch(&project.id, &main_branch.id)
        .await
        .unwrap();
    assert_eq!(after_scenes.len(), 2, "INSERT path lands new scene");
    let one_one_one = after_scenes
        .iter()
        .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 1))
        .unwrap();
    assert_eq!(
        one_one_one.summary, "source 1.1.1",
        "UPDATE path overwrites"
    );
    assert_eq!(one_one_one.full_text, "source: gate held under siege.");
    let one_one_two = after_scenes
        .iter()
        .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 2))
        .unwrap();
    assert_eq!(one_one_two.summary, "source 1.1.2");
    assert_eq!(
        one_one_two.branch_id, main_branch.id,
        "INSERTed scene is stamped with target branch"
    );

    // character_state landed on target branch.
    let after_states = repo
        .list_character_states_by_project_and_branch(&project.id, &main_branch.id)
        .await
        .unwrap();
    let merged_state = after_states
        .iter()
        .find(|s| {
            s.character_id == mara.id
                && (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 3)
        })
        .expect("merged character_state is present at (1,1,3)");
    assert_eq!(merged_state.branch_id, main_branch.id);
    assert_eq!(merged_state.goals, vec!["hold the gate".to_string()]);
    assert_eq!(merged_state.status, vec!["resolute".to_string()]);

    // relationship was overwritten with source's type/trust/tension.
    let merged_rel = repo
        .get_relationship(&main_branch.id, &mara.id, &aldric.id)
        .await
        .unwrap();
    assert_eq!(merged_rel.relationship_type, "rival");
    assert_eq!(merged_rel.trust, 1);
    assert_eq!(merged_rel.tension, 7);
    assert_eq!(merged_rel.reason.as_deref(), Some("the gate quarrel"));

    // pacing_tracker for the arc was replaced with source values.
    let merged_tracker = repo.get_pacing_tracker_by_arc(&arc.id).await.unwrap();
    assert_eq!(merged_tracker.branch_id, main_branch.id);
    assert_eq!(merged_tracker.status, "ahead");
    assert_eq!(merged_tracker.velocity, "fast");
    assert!((merged_tracker.current_progress - 0.75).abs() < f64::EPSILON);
    assert!((merged_tracker.budget_remaining - 0.25).abs() < f64::EPSILON);
    assert_eq!(
        merged_tracker.warnings,
        vec!["watch the trust slide".to_string()]
    );
    assert_ne!(
        merged_tracker.id, original_tracker.id,
        "DELETE-then-INSERT mints a fresh tracker id"
    );
}

/// `restore_branch_snapshot` deletes the current branch content and
/// re-inserts the snapshot rows in a single transaction.
#[tokio::test]
async fn restore_branch_snapshot_replaces_branch_content() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, CreateCharacterInput,
    };

    let (_tmp, repo) = fresh_repo().await;
    let (project, main_branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "RestoreTarget".into(),
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
    let empty_voice = CharacterVoiceProfileData {
        tone: None,
        vocabulary: Vec::new(),
        sentence_structure: Vec::new(),
        tics: Vec::new(),
        forbidden_words: Vec::new(),
        example_lines: Vec::new(),
        established_in_scene_id: None,
        updated_at: None,
    };
    let empty_emotional = CharacterEmotionalProfileData {
        base_emotions: std::collections::BTreeMap::new(),
        suppressed: Vec::new(),
        triggers: Vec::new(),
        defense_mechanisms: Vec::new(),
        flex_range: None,
    };

    // Live state we'll wipe: one character on the active branch.
    repo.create_character(&CreateCharacterInput {
        aliases: Vec::new(),
        project_id: project.id.clone(),
        name: "DoomedDeniz".into(),
        summary: "Will be wiped.".into(),
        role: "antagonist".into(),
        realm: None,
        voice_profile: empty_voice.clone(),
        emotional_profile: empty_emotional.clone(),
        initial_state: None,
    })
    .await
    .unwrap();
    let pre = repo
        .list_characters_by_project_and_branch(&project.id, &main_branch.id)
        .await
        .unwrap();
    assert_eq!(pre.len(), 1, "live character must exist before restore");

    // Snapshot rows: a hand-rolled BranchRestoreSnapshot carrying a
    // single replacement character. Mirrors what
    // `build_branch_restore_snapshot` would produce.
    let mut rows_by_table: std::collections::BTreeMap<String, Vec<serde_json::Map<String, Value>>> =
        std::collections::BTreeMap::new();
    let now_us = chrono::Utc::now().timestamp_micros();
    let mut char_row = serde_json::Map::new();
    char_row.insert(
        "id".to_string(),
        Value::String("character:01HZRESTORE0000000000000001".to_string()),
    );
    char_row.insert("project_id".to_string(), Value::String(project.id.clone()));
    char_row.insert(
        "branch_id".to_string(),
        Value::String(main_branch.id.clone()),
    );
    char_row.insert("name".to_string(), Value::String("RestoredRei".to_string()));
    char_row.insert(
        "normalized_name".to_string(),
        Value::String("restoredrei".to_string()),
    );
    char_row.insert(
        "summary".to_string(),
        Value::String("From snapshot.".to_string()),
    );
    char_row.insert("role".to_string(), Value::String("protagonist".to_string()));
    char_row.insert("realm".to_string(), Value::Null);
    char_row.insert("notes".to_string(), Value::Null);
    char_row.insert("appearance".to_string(), Value::Null);
    char_row.insert(
        "created_at".to_string(),
        Value::Number(serde_json::Number::from(now_us)),
    );
    char_row.insert(
        "updated_at".to_string(),
        Value::Number(serde_json::Number::from(now_us)),
    );
    rows_by_table.insert("character".to_string(), vec![char_row]);

    let snapshot = BranchRestoreSnapshot { rows_by_table };
    repo.restore_branch_snapshot(&project.id, &main_branch.id, &snapshot)
        .await
        .unwrap();

    // Post-condition: live state has the snapshot character only.
    let post = repo
        .list_characters_by_project_and_branch(&project.id, &main_branch.id)
        .await
        .unwrap();
    assert_eq!(post.len(), 1);
    assert_eq!(post[0].name, "RestoredRei");
    assert_eq!(post[0].id, "character:01HZRESTORE0000000000000001");
    assert!(
        !post.iter().any(|c| c.name == "DoomedDeniz"),
        "pre-restore character must be deleted"
    );
}
