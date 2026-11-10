use super::*;

#[tokio::test]
async fn phase_four_validator_context_uses_requested_branch_not_active_branch() {
    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "branch-context".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "branch isolation".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: project.project_id.clone(),
        rule_name: "main-only rule".to_string(),
        rule_type: "magic".to_string(),
        description: "This rule belongs to main.".to_string(),
        scan_pattern: Some("main-only".to_string()),
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();
    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            parent_branch_id: Some(project.branch_id.clone()),
            name: "feature".to_string(),
            branch_type: "experiment".to_string(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: feature.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: project.project_id.clone(),
        rule_name: "feature-only rule".to_string(),
        rule_type: "magic".to_string(),
        description: "This rule belongs to the feature branch.".to_string(),
        scan_pattern: Some("feature-only".to_string()),
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: project.branch_id.clone(),
    })
    .await
    .unwrap();

    let context = svc
        .build_phase_four_validator_context(&project.project_id, &feature.branch_id, &[])
        .await
        .unwrap();
    let rule_names = context
        .world_rules
        .into_iter()
        .map(|rule| rule.rule_name)
        .collect::<Vec<_>>();

    assert_eq!(
        rule_names,
        vec![
            "feature-only rule".to_string(),
            "main-only rule".to_string()
        ]
    );

    let main_context = svc
        .build_phase_four_validator_context(&project.project_id, &project.branch_id, &[])
        .await
        .unwrap();
    let main_rule_names = main_context
        .world_rules
        .into_iter()
        .map(|rule| rule.rule_name)
        .collect::<Vec<_>>();
    assert_eq!(main_rule_names, vec!["main-only rule".to_string()]);
}

#[tokio::test]
async fn get_writer_state_aggregates_current_branch() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        ContentRating, CreateCharacterInput, GetWriterStateInput, SaveSceneDraftInput,
        WriterIntent,
    };

    let (_tmp, svc) = fresh_service().await;

    // Project + 1 character + 1 saved scene on chapter 1.
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Wardens".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Oathbound wardens fail.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    svc.create_character(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: proj.project_id.clone(),
        name: "Mara".into(),
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
            status: Some(vec!["holding the gate".into()]),
            notes: None,
            source_summary: None,
        }),
    })
    .await
    .unwrap();

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Mara stood at the Ash Gate.".into(),
        summary: "Mara holds the gate".into(),
        content_rating: ContentRating::General,
        tone: Some("grim".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // No branch / cursor / format supplied — defaults exercise the
    // active-branch resolution, cursor-from-last-scene fallback, and
    // markdown budget enforcement together.
    let state = svc
        .get_writer_state(GetWriterStateInput {
            project_id: proj.project_id.clone(),
            branch_id: None,
            at_scene_id: None,
            format: None,
            budget_tokens: None,
            include_subjects: None,
            include_recent_activity: None,
            recent_activity_limit: None,
        })
        .await
        .unwrap();

    assert_eq!(state.current.project.project_id, proj.project_id);
    assert_eq!(state.current.branch.branch_id, proj.branch_id);
    assert!(state.current.branch.is_active);
    let scene = state
        .current
        .scene
        .expect("cursor scene resolves from last scene");
    assert_eq!(scene.book_number, 1);
    assert_eq!(scene.chapter_number, 1);
    assert_eq!(scene.scene_order, 1);
    assert_eq!(state.current.intent, WriterIntent::Drafting);

    // Character snapshot present and includes the initial status patch.
    assert_eq!(state.subjects.len(), 1);
    assert_eq!(state.subjects[0].subject.name, "Mara");
    assert_eq!(
        state.subjects[0].status,
        vec!["holding the gate".to_string()]
    );

    // Bundle summary populated by enforce_writer_state_budget.
    assert!(state.bundle_summary.estimated_tokens > 0);
    assert_eq!(state.bundle_summary.token_budget, Some(8000));
    assert!(
        state
            .bundle_summary
            .included_sections
            .iter()
            .any(|s| s == "current")
    );

    // No scene_source_link rows were created in this fixture, so
    // SourceBridge divergence detection has nothing to report.
    assert!(state.unsynced_local_files.is_empty());
    assert!(state.drift_warnings.is_empty());

    let envelope = svc
        .get_writer_state_envelope(GetWriterStateInput {
            project_id: proj.project_id.clone(),
            branch_id: None,
            at_scene_id: None,
            format: None,
            budget_tokens: None,
            include_subjects: None,
            include_recent_activity: None,
            recent_activity_limit: None,
        })
        .await
        .unwrap();
    assert_eq!(envelope.current.project.project_id, proj.project_id);
    assert!(
        envelope
            .writer_state_markdown
            .as_deref()
            .is_some_and(|markdown| markdown.contains("# Writer state")),
        "public envelope should include markdown when requested/defaulted"
    );
}

#[tokio::test]
async fn diff_branches_reports_scene_modifications() {
    use spindle_core::models::{
        ContentRating, CreateBranchInput, DiffBranchesInput, SaveSceneDraftInput, SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Diff".into(),
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
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Original prose.".into(),
        summary: "original".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            parent_branch_id: None,
            name: "feature".into(),
            branch_type: "feature".into(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: feature.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Revised prose with different content.".into(),
        summary: "revised".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let out = svc
        .diff_branches(DiffBranchesInput {
            project_id: proj.project_id.clone(),
            base_branch_id: proj.branch_id.clone(),
            compare_branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();

    assert_eq!(out.scene_diffs.len(), 1);
    let diff = &out.scene_diffs[0];
    assert_eq!(diff.change_type, "modified");
    assert_eq!(diff.base_summary.as_deref(), Some("original"));
    assert_eq!(diff.compare_summary.as_deref(), Some("revised"));
    assert!(out.narrative_impact_summary.contains("1 scene changes"));
}

#[tokio::test]
async fn generate_alternatives_creates_branches_and_restores_active() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, ContentRating,
        CreateCharacterInput, CreateLocationInput, GenerateAlternativesInput, SaveSceneDraftInput,
        WorldStateInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "AltGen".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Wardens hold the line.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden.".into(),
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
            initial_state: None,
        })
        .await
        .unwrap();
    let gate = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A blackened wall.".into(),
            initial_state: WorldStateInput {
                controlling_faction: None,
                status: None,
                prosperity: None,
                stability: None,
                threat_level: None,
                sensory_details: Vec::new(),
            },
        })
        .await
        .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Mara stood at the gate.".into(),
        summary: "stage".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let out = svc
        .generate_alternatives(GenerateAlternativesInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            character_ids: vec![mara.character_id.clone()],
            location_id: gate.location_id.clone(),
            alternatives: Some(2),
            variation_strategy: "approach".into(),
        })
        .await
        .unwrap();

    assert_eq!(out.alternatives.len(), 2);
    assert!(
        out.alternatives[0]
            .branch_name
            .starts_with("alt-approach-1-1-")
    );
    assert_eq!(out.alternatives[0].variation_strategy, "approach");

    // Active branch must be restored to the original (project's main).
    let active_after = svc
        .repository()
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();
    assert_eq!(active_after.id, proj.branch_id);
}

/// `merge_branch` fast-forwards a feature branch onto the project's
/// main when there are no conflicts, and reports a conflict when the
/// target's existing scene has been edited after the source branch
/// diverged.
///
/// Per-project main lookup (Risk #6): the test omits
/// `target_branch_id` so the service must default to the project's
/// per-project `main` branch by name (NOT by the SurrealDB-era
/// singleton). The merged scene must land on the project's actual
/// main branch id.
#[tokio::test]
async fn merge_branch_fast_forwards_into_main_when_no_conflicts() {
    use spindle_core::models::{
        ContentRating, CreateBranchInput, MergeBranchInput, SaveSceneDraftInput, SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Merge".into(),
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

    // Feature branch off main, switch into it, write a brand-new scene
    // at a position main doesn't have. Switching back to main keeps the
    // feature branch's scene isolated.
    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            parent_branch_id: None,
            name: "feature-alt".into(),
            branch_type: "feature".into(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: feature.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "feature: oath sworn at the wall.".into(),
        summary: "feature 1.1.1".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: proj.branch_id.clone(),
    })
    .await
    .unwrap();

    // Merge feature → (defaulted-to-main). No target_branch_id supplied:
    // the service must resolve main by name on the per-project schema.
    let merged = svc
        .merge_branch(MergeBranchInput {
            project_id: proj.project_id.clone(),
            source_branch_id: feature.branch_id.clone(),
            target_branch_id: None,
            merge_type: "fast_forward".into(),
        })
        .await
        .unwrap();
    assert!(
        !merged.has_conflicts,
        "no conflicts when target has no scene at this position"
    );
    assert_eq!(merged.applied_scenes, 1);
    assert_eq!(
        merged.target_branch_id, proj.branch_id,
        "default target resolves to the project's per-project main branch"
    );

    // Main now holds the merged scene.
    let main_scenes = svc
        .repository()
        .list_scenes_by_project_and_branch(&proj.project_id, &proj.branch_id)
        .await
        .unwrap();
    let merged_scene = main_scenes
        .iter()
        .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 1))
        .expect("merged scene present on main");
    assert_eq!(merged_scene.summary, "feature 1.1.1");
    assert_eq!(merged_scene.branch_id, proj.branch_id);
}

/// Conflict detection: if main is edited at a position AFTER the
/// feature branch diverged, merging the feature branch's edit at that
/// position reports a conflict (target wasn't merged, source row is
/// dropped from the mergeable set).
#[tokio::test]
async fn merge_branch_reports_conflict_when_target_edited_after_divergence() {
    use spindle_core::models::{
        ContentRating, CreateBranchInput, MergeBranchInput, SaveSceneDraftInput, SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Merge Conflict".into(),
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

    // Baseline scene on main, then feature branch (so source.created_at
    // > target.updated_at for the baseline scene — that one is safe).
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "main v1: gate stands.".into(),
        summary: "main v1".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // Brief pause so the next branch's created_at is strictly later
    // than main's baseline scene's updated_at (microsecond precision).
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            parent_branch_id: None,
            name: "feature-conflict".into(),
            branch_type: "feature".into(),
            description: None,
        })
        .await
        .unwrap();

    // Now main edits the scene at (1,1,1) — strictly after feature's
    // created_at. The feature branch will see this as a conflict.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "main v2: gate cracks.".into(),
        summary: "main v2".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // Switch into feature and write a conflicting edit at (1,1,1).
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: feature.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "feature: gate held under siege.".into(),
        summary: "feature 1.1.1".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: proj.branch_id.clone(),
    })
    .await
    .unwrap();

    let merged = svc
        .merge_branch(MergeBranchInput {
            project_id: proj.project_id.clone(),
            source_branch_id: feature.branch_id.clone(),
            target_branch_id: None,
            merge_type: "fast_forward".into(),
        })
        .await
        .unwrap();
    assert!(merged.has_conflicts);
    assert_eq!(merged.conflicts.len(), 1);
    assert_eq!(merged.conflicts[0].book_number, 1);
    assert_eq!(merged.conflicts[0].chapter_number, 1);
    assert_eq!(merged.conflicts[0].scene_order, 1);
    assert_eq!(
        merged.applied_scenes, 0,
        "conflicting scene is dropped from the mergeable set"
    );

    // Main still has the v2 edit — merge did not overwrite a conflict.
    let main_scenes = svc
        .repository()
        .list_scenes_by_project_and_branch(&proj.project_id, &proj.branch_id)
        .await
        .unwrap();
    let main_scene = main_scenes
        .iter()
        .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 1))
        .unwrap();
    assert_eq!(main_scene.summary, "main v2");
}

/// `select_alternative` merges the alternative branch into the
/// project's main and switches the active branch onto main. Verifies
/// the per-project main lookup (Risk #6) — the post-merge
/// switch_active_branch call must target the project's actual main
/// branch id, not the SurrealDB-era singleton.
#[tokio::test]
async fn select_alternative_promotes_branch_and_switches_active_to_main() {
    use spindle_core::models::{
        ContentRating, CreateBranchInput, SaveSceneDraftInput, SelectAlternativeInput,
        SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Select".into(),
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

    // Feature branch with a unique scene at (1,1,1).
    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            parent_branch_id: None,
            name: "feature-pick".into(),
            branch_type: "feature".into(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: feature.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "feature pick: gate held.".into(),
        summary: "feature picked".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // Run select_alternative from the feature branch (active branch is
    // feature). After the call, active branch must be main again.
    let out = svc
        .select_alternative(SelectAlternativeInput {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(out.selected_branch_id, feature.branch_id);
    assert_eq!(out.target_branch_id, proj.branch_id);
    assert_eq!(out.merge_type, "fast_forward");

    // Active branch is now the project's main, NOT the singleton.
    let active = svc
        .repository()
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();
    assert_eq!(active.id, proj.branch_id);
    assert_eq!(active.name, "main");

    // The merged scene lives on main now.
    let main_scenes = svc
        .repository()
        .list_scenes_by_project_and_branch(&proj.project_id, &proj.branch_id)
        .await
        .unwrap();
    let merged = main_scenes
        .iter()
        .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 1))
        .expect("alternative's scene now on main");
    assert_eq!(merged.summary, "feature picked");
}

/// End-to-end restore_save_point flow: create a project + character,
/// snapshot the branch via `create_save_point`, mutate the live state
/// (add a second character), then restore. The snapshot character must
/// survive; the post-snapshot mutation must be rolled back; and a
/// pre-restore backup save_point must be produced.
#[tokio::test]
async fn restore_save_point_rewinds_active_branch_to_snapshot() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, CreateCharacterInput,
        CreateSavePointInput, RestoreSavePointInput,
    };

    let (_tmp, svc) = fresh_service().await;

    let project = svc
        .create_project(CreateProjectInput {
            name: "RestoreSavePointFlow".into(),
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
    // Initial character we want preserved across the restore.
    svc.create_character(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: project.project_id.clone(),
        name: "Mara".into(),
        summary: "Survives.".into(),
        role: "protagonist".into(),
        realm: None,
        voice_profile: empty_voice.clone(),
        emotional_profile: empty_emotional.clone(),
        initial_state: None,
    })
    .await
    .unwrap();

    // Capture the save_point. This also writes the snapshot file +
    // sha256/record_count metadata via `persist_save_point_snapshot`.
    let sp = svc
        .create_save_point(CreateSavePointInput {
            project_id: project.project_id.clone(),
            name: "milestone-1".into(),
            description: None,
        })
        .await
        .unwrap();

    // Post-snapshot mutation: a second character that should NOT
    // survive the restore.
    svc.create_character(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: project.project_id.clone(),
        name: "Ephemera".into(),
        summary: "Will vanish on restore.".into(),
        role: "antagonist".into(),
        realm: None,
        voice_profile: empty_voice.clone(),
        emotional_profile: empty_emotional.clone(),
        initial_state: None,
    })
    .await
    .unwrap();
    let pre = svc
        .repository
        .list_characters_by_project_and_branch(&project.project_id, &project.branch_id)
        .await
        .unwrap();
    assert_eq!(pre.len(), 2, "two characters before restore");

    // Restore.
    let out = svc
        .restore_save_point(RestoreSavePointInput {
            project_id: project.project_id.clone(),
            save_point_id: sp.save_point_id.clone(),
        })
        .await
        .unwrap();

    assert_eq!(out.save_point_id, sp.save_point_id);
    assert_eq!(out.branch_id, project.branch_id);
    assert_eq!(out.status, "restored");
    assert!(
        out.backup_save_point_id.starts_with("save_point:"),
        "backup save_point id should be minted: got {}",
        out.backup_save_point_id
    );
    // The restored snapshot carried exactly one character row.
    assert!(out.restored_tables >= 1, "got {}", out.restored_tables);
    assert!(out.restored_records >= 1, "got {}", out.restored_records);

    // Post-condition: only the original character is on the branch.
    let post = svc
        .repository
        .list_characters_by_project_and_branch(&project.project_id, &project.branch_id)
        .await
        .unwrap();
    assert_eq!(post.len(), 1, "exactly the snapshot character remains");
    assert_eq!(post[0].name, "Mara");
}

#[tokio::test]
async fn merge_branch_carries_canonical_fact_canon() {
    use crate::sqlite::repository::CreateCanonicalFactParams;
    use spindle_core::models::{
        ContentRating, CreateBranchInput, MergeBranchInput, SaveSceneDraftInput, SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "mergecf".into(),
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
    let main_branch = svc
        .repository()
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();
    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            name: "feature".into(),
            branch_type: "experiment".into(),
            description: None,
            parent_branch_id: Some(main_branch.id.clone()),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: feature.branch_id.clone(),
    })
    .await
    .unwrap();
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "x".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    // A branch-local canonical fact whose scene_id is a NOT NULL FK.
    svc.repository()
        .create_canonical_fact(CreateCanonicalFactParams {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
            scene_id: Some(scene.scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            subject_table: "character".into(),
            subject_id: Some("character:mara".into()),
            predicate: "eye_color".into(),
            value_kind: "string".into(),
            value_text: Some("blue".into()),
            value_number: None,
            unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: "invariant".into(),
            valid_from: None,
            valid_until: None,
            legacy_untyped: false,
            secret: false,
            concealment_note: None,
        })
        .await
        .unwrap();

    svc.merge_branch(MergeBranchInput {
        project_id: proj.project_id.clone(),
        source_branch_id: feature.branch_id.clone(),
        target_branch_id: Some(main_branch.id.clone()),
        merge_type: "squash".into(),
    })
    .await
    .unwrap();

    let main_facts = svc
        .repository()
        .list_active_canonical_facts_by_project_and_branch(&proj.project_id, &main_branch.id)
        .await
        .unwrap();
    assert!(
        main_facts.iter().any(
            |f| f.predicate == "eye_color" && f.subject_id.as_deref() == Some("character:mara")
        ),
        "branch-local canonical fact must survive merge into main (count={})",
        main_facts.len()
    );
}

#[tokio::test]
async fn merge_branch_carries_quantity_canon() {
    use spindle_core::models::{
        CommitQuantityStateInput, CreateBranchInput, MergeBranchInput, QuantityBand,
        QuantityScheme, QuantityState, SetProjectQuantitySchemeInput, SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "mergeqty".into(),
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
    let main_branch = svc
        .repository()
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();
    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            name: "feature".into(),
            branch_type: "experiment".into(),
            description: None,
            parent_branch_id: Some(main_branch.id.clone()),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: feature.branch_id.clone(),
    })
    .await
    .unwrap();

    // Declare a scheme and a stamped reading on the feature branch.
    svc.set_project_quantity_scheme(SetProjectQuantitySchemeInput {
        project_id: proj.project_id.clone(),
        scheme: QuantityScheme {
            measure: "wealth".into(),
            denominations: Vec::new(),
            bands: vec![QuantityBand {
                name: "destitute".into(),
                lower_bound: None,
            }],
            max_band_jump: Some(1),
        },
    })
    .await
    .unwrap();
    svc.commit_quantity_state(CommitQuantityStateInput {
        project_id: proj.project_id.clone(),
        subject_table: "character".into(),
        subject_id: "character:mara".into(),
        measure: "wealth".into(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
        scene_id: None,
        state: QuantityState {
            band: Some("destitute".into()),
            ..Default::default()
        },
    })
    .await
    .unwrap();

    svc.merge_branch(MergeBranchInput {
        project_id: proj.project_id.clone(),
        source_branch_id: feature.branch_id.clone(),
        target_branch_id: Some(main_branch.id.clone()),
        merge_type: "squash".into(),
    })
    .await
    .unwrap();

    // Both the scheme and the stamped state must now exist on main.
    assert!(
        svc.repository()
            .get_project_quantity_scheme(&proj.project_id, &main_branch.id, "wealth")
            .await
            .unwrap()
            .is_some(),
        "branch-local quantity scheme must survive merge into main"
    );
    let main_states = svc
        .repository()
        .list_quantity_states_by_project_and_branch(&proj.project_id, &main_branch.id)
        .await
        .unwrap();
    assert!(
        main_states
            .iter()
            .any(|s| s.subject_id == "character:mara"
                && s.state.band.as_deref() == Some("destitute")),
        "branch-local quantity state must survive merge into main (count={})",
        main_states.len()
    );
}

#[tokio::test]
async fn merge_branch_carries_world_rule_canon() {
    use spindle_core::models::{
        CreateBranchInput, CreateWorldRuleInput, MergeBranchInput, SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "merge".into(),
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
    let main_branch = svc
        .repository()
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();
    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            name: "feature".into(),
            branch_type: "experiment".into(),
            description: None,
            parent_branch_id: Some(main_branch.id.clone()),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: feature.branch_id.clone(),
    })
    .await
    .unwrap();
    // A branch-local world rule that today is dropped on merge.
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: proj.project_id.clone(),
        rule_name: "Iron repels the fae".into(),
        rule_type: "magic".into(),
        description: "Cold iron burns fae flesh.".into(),
        scan_pattern: None,
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();

    svc.merge_branch(MergeBranchInput {
        project_id: proj.project_id.clone(),
        source_branch_id: feature.branch_id.clone(),
        target_branch_id: Some(main_branch.id.clone()),
        merge_type: "squash".into(),
    })
    .await
    .unwrap();

    let main_rules = svc
        .repository()
        .list_world_rules_by_project_and_branch(&proj.project_id, &main_branch.id)
        .await
        .unwrap();
    assert!(
        main_rules
            .iter()
            .any(|r| r.rule_name == "Iron repels the fae"),
        "branch-local world rule must survive merge into main: {:?}",
        main_rules.iter().map(|r| &r.rule_name).collect::<Vec<_>>()
    );
}

/// `scene_id` delete must never act on a row that is not on the active
/// branch — those are the alternative-branch rows the live run mistook
/// for duplicates, and active-branch tools have no business touching
/// them.
#[tokio::test]
async fn delete_scene_by_id_rejects_scene_on_another_branch() {
    use spindle_core::models::DeleteSceneInput;

    let (_tmp, svc, proj, spine_ids, alt_ids) = spine_chapter_with_unresolved_alternatives().await;

    let err = svc
        .delete_scene(DeleteSceneInput {
            project_id: proj.project_id.clone(),
            scene_id: Some(alt_ids[0].clone()),
            ..Default::default()
        })
        .await
        .expect_err("an alternative-branch scene must not be deletable by an active-branch tool");
    assert!(
        err.to_string().contains("not the active branch"),
        "expected an active-branch guard, got: {err}"
    );
    // The alternative row is untouched.
    assert!(
        svc.repository.get_scene(&alt_ids[0]).await.is_ok(),
        "the rejected delete must not touch the row"
    );
    // The spine scene at the same position is still deletable by id.
    let out = svc
        .delete_scene(DeleteSceneInput {
            project_id: proj.project_id.clone(),
            scene_id: Some(spine_ids[1].clone()),
            ..Default::default()
        })
        .await
        .expect("the real spine scene must be deletable by id");
    assert_eq!(out.scene_id, spine_ids[1]);
}

#[tokio::test]
async fn commit_character_state_and_record_knowledge_target_active_branch_and_merge_branch_aware() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CommitCharacterStateInput, ContentRating, CreateBranchInput, CreateCharacterInput,
        CreateProjectInput, RecordKnowledgeInput, SaveSceneDraftInput, StoryPlacement,
        SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "branch-canon-writes".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "branch isolation".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let hero = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Kael".to_string(),
            summary: "Guard captain of the Citadel.".to_string(),
            role: "protagonist".to_string(),
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
            initial_state: None,
        })
        .await
        .unwrap();

    let main_scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Kael walked the Citadel battlements with his bronze badge.".to_string(),
            summary: "Kael patrols the Citadel.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let mut main_emotions = std::collections::BTreeMap::new();
    main_emotions.insert("vigilance".to_string(), serde_json::json!(0.8));
    svc.commit_character_state(CommitCharacterStateInput {
        character_id: hero.character_id.clone(),
        scene_id: Some(main_scene.scene_id.clone()),
        anchor: None,
        changes: CharacterStatePatch {
            emotional_state: main_emotions,
            goals: Some(vec!["patrol the Citadel".to_string()]),
            status: Some(vec!["healthy".to_string()]),
            notes: Some(vec!["carrying bronze badge".to_string()]),
            source_summary: Some("main patrol".to_string()),
        },
    })
    .await
    .unwrap();

    let child_branch = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            parent_branch_id: Some(project.branch_id.clone()),
            name: "alt-timeline".to_string(),
            branch_type: "experiment".to_string(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: child_branch.branch_id.clone(),
    })
    .await
    .unwrap();

    let child_scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: "Kael fled into the Catacombs clutching the iron key.".to_string(),
            summary: "Kael flees into the Catacombs.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Omit `goals` and `status` so they merge from the prior state inherited from `main`.
    let child_commit = svc
        .commit_character_state(CommitCharacterStateInput {
            character_id: hero.character_id.clone(),
            scene_id: Some(child_scene.scene_id.clone()),
            anchor: None,
            changes: CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: None,
                status: None,
                notes: Some(vec!["carrying iron key".to_string()]),
                source_summary: Some("catacombs escape".to_string()),
            },
        })
        .await
        .unwrap();

    let child_state = svc
        .repository
        .get_character_state_by_id(&child_commit.state_id)
        .await
        .unwrap();
    assert_eq!(child_state.branch_id, child_branch.branch_id);
    assert_eq!(
        child_state.goals,
        vec!["patrol the Citadel".to_string()],
        "child branch commit must merge prior goals inherited from parent branch"
    );
    assert_eq!(
        child_state.status,
        vec!["healthy".to_string()],
        "child branch commit must merge prior status inherited from parent branch"
    );
    assert_eq!(
        child_state.notes,
        vec!["carrying iron key".to_string()],
        "explicitly supplied notes replace prior notes on child branch"
    );

    let all_events = svc
        .repository
        .list_all_progression_events(&project.project_id)
        .await
        .unwrap();
    let child_events: Vec<_> = all_events
        .into_iter()
        .filter(|e| e.source_scene_id.as_deref() == Some(child_scene.scene_id.as_str()))
        .collect();
    assert_eq!(child_events.len(), 1);
    assert_eq!(child_events[0].branch_id, child_branch.branch_id);

    let knowledge = svc
        .record_knowledge(RecordKnowledgeInput {
            project_id: project.project_id.clone(),
            branch_id: None,
            character_id: hero.character_id.clone(),
            fact: "The regent forged the decree".to_string(),
            source_summary: "witnessed in catacombs".to_string(),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(2),
                note: None,
            }),
            confidence: Some(1.0),
            tags: Vec::new(),
            reader_visible: true,
            secret_of_fact_id: None,
        })
        .await
        .unwrap();
    assert_eq!(knowledge.fact.branch_id, child_branch.branch_id);
    assert!(
        knowledge
            .knows_edge_id
            .as_deref()
            .unwrap()
            .contains(&child_branch.branch_id)
    );

    // Main branch must NOT see child-branch state or knowledge writes.
    let main_resolved = svc
        .repository
        .resolve_character_state_for_branch(&project.branch_id, &hero.character_id, 1, 1, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(main_resolved.branch_id, project.branch_id);
    assert_eq!(
        main_resolved.notes,
        vec!["carrying bronze badge".to_string()]
    );

    let all_knows = svc
        .repository
        .list_all_knows(&project.project_id)
        .await
        .unwrap();
    assert!(
        all_knows
            .iter()
            .all(|k| k.branch_id == child_branch.branch_id)
    );
}

#[tokio::test]
async fn child_branch_inherits_parent_chain_canon_and_overrides_parent_rows() {
    use spindle_core::models::{
        CanonicalFactScope, ContentRating, CreateBranchInput, CreateProjectInput,
        CreateWorldRuleInput, RegisterCanonicalFactInput, SaveSceneDraftInput, SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "branch-inheritance".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "consistent parent chain".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let main_scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Opening scene text.".to_string(),
            summary: "Opening.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let main_fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(main_scene.scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("project".to_string()),
            subject_id: Some(project.project_id.clone()),
            predicate: Some("sunrise_direction".to_string()),
            value_kind: Some("string".to_string()),
            value_text: Some("west".to_string()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: Some(CanonicalFactScope::Invariant),
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();

    svc.create_world_rule(CreateWorldRuleInput {
        project_id: project.project_id.clone(),
        rule_name: "Gravity Law".to_string(),
        rule_type: "physics".to_string(),
        description: "Heavy objects fall downward.".to_string(),
        scan_pattern: None,
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();

    let parent_branch = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            parent_branch_id: Some(project.branch_id.clone()),
            name: "book-2-draft".to_string(),
            branch_type: "arc".to_string(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: parent_branch.branch_id.clone(),
    })
    .await
    .unwrap();

    let parent_fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: None,
            book_number: 1,
            chapter_number: 2,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("project".to_string()),
            subject_id: Some(project.project_id.clone()),
            predicate: Some("northern_pass_state".to_string()),
            value_kind: Some("string".to_string()),
            value_text: Some("sealed by ice".to_string()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: Some(CanonicalFactScope::Evolving),
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();

    let leaf_branch = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            parent_branch_id: Some(parent_branch.branch_id.clone()),
            name: "book-2-alt".to_string(),
            branch_type: "experiment".to_string(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: leaf_branch.branch_id.clone(),
    })
    .await
    .unwrap();

    // Fresh leaf branch sees canonical facts from both parent and main.
    let inherited_facts = svc
        .repository
        .list_active_canonical_facts_by_project_and_branch(
            &project.project_id,
            &leaf_branch.branch_id,
        )
        .await
        .unwrap();
    let inherited_ids: Vec<String> = inherited_facts.iter().map(|f| f.id.clone()).collect();
    assert!(inherited_ids.contains(&main_fact.canonical_fact_id));
    assert!(inherited_ids.contains(&parent_fact.canonical_fact_id));

    // Override the world rule on the leaf branch and supersede the parent fact.
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: project.project_id.clone(),
        rule_name: "Gravity Law".to_string(),
        rule_type: "physics".to_string(),
        description: "Gravity reverses inside the Spire.".to_string(),
        scan_pattern: None,
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();

    let leaf_fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: None,
            book_number: 1,
            chapter_number: 3,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("project".to_string()),
            subject_id: Some(project.project_id.clone()),
            predicate: Some("northern_pass_state".to_string()),
            value_kind: Some("string".to_string()),
            value_text: Some("thawed by dragonfire".to_string()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: Some(CanonicalFactScope::Evolving),
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: Some(parent_fact.canonical_fact_id.clone()),
            secrecy: None,
        })
        .await
        .unwrap();

    let leaf_rules = svc
        .repository
        .list_world_rules_by_project_and_branch(&project.project_id, &leaf_branch.branch_id)
        .await
        .unwrap();
    assert_eq!(leaf_rules.len(), 1);
    assert_eq!(
        leaf_rules[0].description,
        "Gravity reverses inside the Spire."
    );

    // Main branch still sees the original rule.
    let main_rules = svc
        .repository
        .list_world_rules_by_project_and_branch(&project.project_id, &project.branch_id)
        .await
        .unwrap();
    assert_eq!(main_rules.len(), 1);
    assert_eq!(main_rules[0].description, "Heavy objects fall downward.");

    let leaf_active_facts = svc
        .repository
        .list_active_canonical_facts_by_project_and_branch(
            &project.project_id,
            &leaf_branch.branch_id,
        )
        .await
        .unwrap();
    let leaf_fact_ids: Vec<String> = leaf_active_facts.iter().map(|f| f.id.clone()).collect();
    assert!(leaf_fact_ids.contains(&main_fact.canonical_fact_id));
    assert!(leaf_fact_ids.contains(&leaf_fact.canonical_fact_id));
    assert!(!leaf_fact_ids.contains(&parent_fact.canonical_fact_id));

    // Parent branch still sees `parent_fact` as active because `leaf_fact` is only on `leaf_branch`.
    let parent_active_facts = svc
        .repository
        .list_active_canonical_facts_by_project_and_branch(
            &project.project_id,
            &parent_branch.branch_id,
        )
        .await
        .unwrap();
    let parent_fact_ids: Vec<String> = parent_active_facts.iter().map(|f| f.id.clone()).collect();
    assert!(parent_fact_ids.contains(&main_fact.canonical_fact_id));
    assert!(parent_fact_ids.contains(&parent_fact.canonical_fact_id));
    assert!(!parent_fact_ids.contains(&leaf_fact.canonical_fact_id));
}

#[tokio::test]
async fn commit_scene_changes_validates_against_effective_write_branch_for_inherited_scene() {
    use spindle_core::models::{
        CanonicalFactEntry, CanonicalFactScope, CharacterEmotionalProfileData,
        CharacterVoiceProfileData, CommitContinuityGate, CommitSceneChangesInput, ContentRating,
        CreateBranchInput, CreateCharacterInput, CreateProjectInput, RegisterCanonicalFactInput,
        SaveSceneDraftInput, SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "effective-branch-validation".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "branch-aware continuity gate".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let character = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Lyra".to_string(),
            summary: "Navigator.".to_string(),
            role: "protagonist".to_string(),
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
            initial_state: None,
        })
        .await
        .unwrap();

    // Inherited scene S on `main`.
    let main_scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Lyra checked the compass on the deck.".to_string(),
            summary: "Lyra checks the compass.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Parent (`main`) invariant fact: eye_color = "blue".
    let main_eye_fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(main_scene.scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".to_string()),
            subject_id: Some(character.character_id.clone()),
            predicate: Some("eye_color".to_string()),
            value_kind: Some("string".to_string()),
            value_text: Some("blue".to_string()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: Some(CanonicalFactScope::Invariant),
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();

    // Create and switch to child branch.
    let child_branch = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            name: "child-canon-gate".to_string(),
            description: Some("child branch for continuity gate test".to_string()),
            parent_branch_id: None,
            branch_type: "what_if".to_string(),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: child_branch.branch_id.clone(),
    })
    .await
    .unwrap();

    // Child-only invariant fact: height = 170 (registered via inherited main_scene while child is active).
    let child_height_fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(main_scene.scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".to_string()),
            subject_id: Some(character.character_id.clone()),
            predicate: Some("height".to_string()),
            value_kind: Some("number".to_string()),
            value_text: None,
            value_number: Some(170.0),
            value_unit: Some("cm".to_string()),
            value_json: None,
            aliases: Vec::new(),
            scope: Some(CanonicalFactScope::Invariant),
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();

    // Override parent's eye_color ("blue") on child with "green".
    let child_eye_fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(main_scene.scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".to_string()),
            subject_id: Some(character.character_id.clone()),
            predicate: Some("eye_color".to_string()),
            value_kind: Some("string".to_string()),
            value_text: Some("green".to_string()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: Some(CanonicalFactScope::Invariant),
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: Some(main_eye_fact.canonical_fact_id.clone()),
            secrecy: None,
        })
        .await
        .unwrap();

    // 1. Committing contradicting `height = 180` through inherited `main_scene` while
    //    `child` is active MUST see the child-only `height = 170` fact and block under
    //    `BlockErrors` without writing the contradiction to `child`.
    let blocked_child_only = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: main_scene.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: vec![CanonicalFactEntry {
                fact_type: None,
                key: None,
                value: None,
                subject_table: Some("character".to_string()),
                subject_id: Some(character.character_id.clone()),
                predicate: Some("height".to_string()),
                value_kind: Some("number".to_string()),
                value_text: None,
                value_number: Some(180.0),
                value_unit: Some("cm".to_string()),
                value_json: None,
                aliases: Vec::new(),
                scope: Some(CanonicalFactScope::Invariant),
                valid_from: None,
                valid_until: None,
                context: None,
                supersedes_fact_id: None,
            }],
            relationship_updates: Vec::new(),
            accept_world_rule_risks: false,
            accept_continuity_risks: false,
            continuity_gate: Some(CommitContinuityGate::BlockErrors),
        })
        .await;
    assert!(
        blocked_child_only.is_err(),
        "committing height=180 through inherited scene while child has height=170 must be blocked"
    );

    let child_facts_after_block = svc
        .repository
        .list_active_canonical_facts_by_project_and_branch(
            &project.project_id,
            &child_branch.branch_id,
        )
        .await
        .unwrap();
    let height_values: Vec<Option<f64>> = child_facts_after_block
        .iter()
        .filter(|f| f.predicate == "height")
        .map(|f| f.value_number)
        .collect();
    assert_eq!(height_values, vec![Some(170.0)]);

    // 2. Committing `eye_color = "green"` (matching child's override of parent's "blue")
    //    and superseding `child_height_fact` with `height = 172` through inherited
    //    `main_scene` while `child` is active MUST succeed under `BlockErrors` and not
    //    be falsely blocked by parent's overridden `eye_color = "blue"`.
    let valid_child_commit = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: main_scene.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: vec![
                CanonicalFactEntry {
                    fact_type: None,
                    key: None,
                    value: None,
                    subject_table: Some("character".to_string()),
                    subject_id: Some(character.character_id.clone()),
                    predicate: Some("eye_color".to_string()),
                    value_kind: Some("string".to_string()),
                    value_text: Some("green".to_string()),
                    value_number: None,
                    value_unit: None,
                    value_json: None,
                    aliases: Vec::new(),
                    scope: Some(CanonicalFactScope::Invariant),
                    valid_from: None,
                    valid_until: None,
                    context: None,
                    supersedes_fact_id: Some(child_eye_fact.canonical_fact_id.clone()),
                },
                CanonicalFactEntry {
                    fact_type: None,
                    key: None,
                    value: None,
                    subject_table: Some("character".to_string()),
                    subject_id: Some(character.character_id.clone()),
                    predicate: Some("height".to_string()),
                    value_kind: Some("number".to_string()),
                    value_text: None,
                    value_number: Some(172.0),
                    value_unit: Some("cm".to_string()),
                    value_json: None,
                    aliases: Vec::new(),
                    scope: Some(CanonicalFactScope::Invariant),
                    valid_from: None,
                    valid_until: None,
                    context: None,
                    supersedes_fact_id: Some(child_height_fact.canonical_fact_id.clone()),
                },
            ],
            relationship_updates: Vec::new(),
            accept_world_rule_risks: false,
            accept_continuity_risks: false,
            continuity_gate: Some(CommitContinuityGate::BlockErrors),
        })
        .await
        .expect("valid child commit against overridden parent fact should succeed");
    assert_eq!(valid_child_commit.canonical_facts.len(), 2);

    // Parent (`main`) still has only `eye_color = "blue"` and no `height` fact.
    let main_facts = svc
        .repository
        .list_active_canonical_facts_by_project_and_branch(&project.project_id, &project.branch_id)
        .await
        .unwrap();
    assert_eq!(main_facts.len(), 1);
    assert_eq!(main_facts[0].predicate, "eye_color");
    assert_eq!(main_facts[0].value_text.as_deref(), Some("blue"));
    assert!(
        svc.repository
            .get_canonical_fact(&main_eye_fact.canonical_fact_id)
            .await
            .unwrap()
            .superseded_by
            .is_none()
    );

    let save = svc
        .create_save_point(spindle_core::models::CreateSavePointInput {
            project_id: project.project_id.clone(),
            name: "supersession-chain".into(),
            description: None,
        })
        .await
        .unwrap();
    svc.restore_save_point(spindle_core::models::RestoreSavePointInput {
        project_id: project.project_id.clone(),
        save_point_id: save.save_point_id,
    })
    .await
    .unwrap();

    svc.merge_branch(spindle_core::models::MergeBranchInput {
        project_id: project.project_id.clone(),
        source_branch_id: child_branch.branch_id.clone(),
        target_branch_id: Some(project.branch_id.clone()),
        merge_type: "squash".into(),
    })
    .await
    .unwrap();
    let merged = svc
        .repository
        .list_active_canonical_facts_by_project_and_branch(&project.project_id, &project.branch_id)
        .await
        .unwrap();
    assert!(
        !merged
            .iter()
            .any(|f| f.id == main_eye_fact.canonical_fact_id),
        "merging inherited supersession must retire the original parent fact"
    );
    assert!(
        merged
            .iter()
            .filter(|f| f.predicate == "eye_color")
            .all(|f| f.value_text.as_deref() == Some("green"))
    );

    let sibling = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            name: "voice-sibling".into(),
            parent_branch_id: Some(project.branch_id.clone()),
            description: None,
            branch_type: "what_if".into(),
        })
        .await
        .unwrap();
    for branch_id in [&child_branch.branch_id, &sibling.branch_id] {
        svc.switch_branch(SwitchBranchInput {
            project_id: project.project_id.clone(),
            branch_id: branch_id.clone(),
        })
        .await
        .unwrap();
        let profile = svc
            .repository
            .get_character_voice_profile(&character.character_id)
            .await
            .unwrap();
        assert_eq!(profile.tone, None);
        let mut profile: CharacterVoiceProfileData =
            serde_json::from_value(serde_json::to_value(profile).unwrap()).unwrap();
        profile.tone = Some("child-only".into());
        profile.updated_at = None;
        let result = svc
            .set_character_voice_profile(spindle_core::models::SetCharacterVoiceProfileInput {
                project_id: project.project_id.clone(),
                character_id: character.character_id.clone(),
                branch_id: branch_id.clone(),
                profile,
            })
            .await;
        assert!(
            result.is_err(),
            "inherited voice edits require branch-local storage"
        );
    }
    let parent_voice = svc
        .repository
        .get_character_voice_profile(&character.character_id)
        .await
        .unwrap();
    assert_eq!(parent_voice.tone, None);
}
