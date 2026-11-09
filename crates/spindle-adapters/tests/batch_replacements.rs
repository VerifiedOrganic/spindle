use spindle_adapters::ModelRouter;
use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    BatchCreateMotifItem, BatchCreateNarrativePromiseItem, BatchCreateTermItem,
    BatchCreateTermsInput, BatchSetCharacterVoiceProfileItem, CharacterVoiceProfileData,
    CreateBranchInput, CreateCharacterInput, CreateMotifsInput, CreateNarrativePromisesInput,
    CreateProjectInput, CreateTermsInput, ReaderContract, SetCharacterVoiceProfilesInput,
    StoryPlacement, SwitchBranchInput,
};
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::with_model_router(pool, data_dir, ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    (tmp, svc)
}

async fn project(svc: &SqliteSpindleService, name: &str) -> String {
    svc.create_project(CreateProjectInput {
        name: name.to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "A test promise.".to_string(),
            style_notes: Vec::new(),
            boundaries: Vec::new(),
        },
    })
    .await
    .unwrap()
    .project_id
}

fn term(text: &str) -> BatchCreateTermItem {
    BatchCreateTermItem {
        term_text: text.to_string(),
        pronunciation: None,
        definition: format!("Definition of {text}."),
        usage_context: None,
        origin: None,
    }
}

#[tokio::test]
async fn term_batch_is_atomic_dry_run_safe_and_alias_compatible() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Batch Terms").await;
    let branch_id = svc
        .repository()
        .active_branch_id_public(&project_id)
        .await
        .unwrap();

    let created = svc
        .create_terms(CreateTermsInput {
            project_id: project_id.clone(),
            items: vec![term("Aether"), term("Ley line")],
            dry_run: false,
        })
        .await
        .unwrap();
    assert_eq!(created.created, 2);
    assert_eq!(created.term_ids.len(), 2);
    assert_eq!(
        svc.repository()
            .list_terms_by_project_and_branch(&project_id, &branch_id)
            .await
            .unwrap()
            .len(),
        2
    );

    let dry = svc
        .create_terms(CreateTermsInput {
            project_id: project_id.clone(),
            items: vec![term("Dry only")],
            dry_run: true,
        })
        .await
        .unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.created, 0);
    assert_eq!(
        svc.repository()
            .list_terms_by_project_and_branch(&project_id, &branch_id)
            .await
            .unwrap()
            .len(),
        2
    );

    let existing_dry = svc
        .create_terms(CreateTermsInput {
            project_id: project_id.clone(),
            items: vec![term("Aether")],
            dry_run: true,
        })
        .await
        .unwrap_err();
    assert!(
        format!("{existing_dry:#}").contains("already exists"),
        "dry-run must predict an existing-name collision: {existing_dry:#}"
    );
    let within_batch_dry = svc
        .create_terms(CreateTermsInput {
            project_id: project_id.clone(),
            items: vec![term("Twin"), term("twin")],
            dry_run: true,
        })
        .await
        .unwrap_err();
    assert!(
        format!("{within_batch_dry:#}").contains("item 1")
            && format!("{within_batch_dry:#}").contains("duplicate normalized"),
        "dry-run must predict a within-batch normalized collision: {within_batch_dry:#}"
    );

    let too_many: Vec<_> = (0..51).map(|i| term(&format!("Term {i}"))).collect();
    assert!(
        svc.create_terms(CreateTermsInput {
            project_id: project_id.clone(),
            items: too_many,
            dry_run: true,
        })
        .await
        .is_err()
    );
    assert!(
        svc.create_terms(CreateTermsInput {
            project_id: project_id.clone(),
            items: Vec::new(),
            dry_run: true,
        })
        .await
        .is_err()
    );

    // A duplicate second item must roll back the first insert.
    let err = svc
        .create_terms(CreateTermsInput {
            project_id: project_id.clone(),
            items: vec![term("Rollback"), term("Aether")],
            dry_run: false,
        })
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("item 1"), "{err:#}");
    assert_eq!(
        svc.repository()
            .list_terms_by_project_and_branch(&project_id, &branch_id)
            .await
            .unwrap()
            .len(),
        2
    );

    let alias = svc
        .batch_create_terms(BatchCreateTermsInput {
            project_id,
            items: vec![term("Alias")],
            dry_run: None,
        })
        .await
        .unwrap();
    assert_eq!(alias.created, 1);
    assert_eq!(alias.term_ids.len(), 1);
}

#[tokio::test]
async fn motif_and_promise_batches_are_atomic() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Batch Narrative").await;

    let motifs = svc
        .create_motifs(CreateMotifsInput {
            project_id: project_id.clone(),
            items: vec![
                BatchCreateMotifItem {
                    name: "Salt".to_string(),
                    description: "Preservation and grief.".to_string(),
                    max_uses_per_chapter: Some(2),
                    connected_theme_ids: Vec::new(),
                },
                BatchCreateMotifItem {
                    name: "Bell".to_string(),
                    description: "Warning and ritual.".to_string(),
                    max_uses_per_chapter: None,
                    connected_theme_ids: Vec::new(),
                },
            ],
            dry_run: false,
        })
        .await
        .unwrap();
    assert_eq!(motifs.created, 2);
    assert_eq!(
        svc.repository()
            .list_motifs_by_project(&project_id)
            .await
            .unwrap()
            .len(),
        2
    );

    let existing_motif_dry = svc
        .create_motifs(CreateMotifsInput {
            project_id: project_id.clone(),
            items: vec![BatchCreateMotifItem {
                name: "Salt".to_string(),
                description: "Duplicate.".to_string(),
                max_uses_per_chapter: None,
                connected_theme_ids: Vec::new(),
            }],
            dry_run: true,
        })
        .await
        .unwrap_err();
    assert!(
        format!("{existing_motif_dry:#}").contains("already exists"),
        "dry-run must predict an existing motif: {existing_motif_dry:#}"
    );
    let within_motif_dry = svc
        .create_motifs(CreateMotifsInput {
            project_id: project_id.clone(),
            items: vec![
                BatchCreateMotifItem {
                    name: "Echo".to_string(),
                    description: "One.".to_string(),
                    max_uses_per_chapter: None,
                    connected_theme_ids: Vec::new(),
                },
                BatchCreateMotifItem {
                    name: "echo".to_string(),
                    description: "Two.".to_string(),
                    max_uses_per_chapter: None,
                    connected_theme_ids: Vec::new(),
                },
            ],
            dry_run: true,
        })
        .await
        .unwrap_err();
    assert!(
        format!("{within_motif_dry:#}").contains("item 1")
            && format!("{within_motif_dry:#}").contains("duplicate normalized"),
        "dry-run must predict a within-batch motif collision: {within_motif_dry:#}"
    );

    let placement = StoryPlacement {
        book_number: 1,
        chapter_number: 1,
        scene_order: Some(1),
        note: None,
    };
    let promises = svc
        .create_narrative_promises(CreateNarrativePromisesInput {
            project_id: project_id.clone(),
            items: vec![
                BatchCreateNarrativePromiseItem {
                    promise_type: "chekhov_gun".to_string(),
                    description: "The sealed letter matters.".to_string(),
                    planted_at: placement.clone(),
                    planned_payoff: None,
                    notes: Vec::new(),
                },
                BatchCreateNarrativePromiseItem {
                    promise_type: "foreshadowing".to_string(),
                    description: "The bell rings twice.".to_string(),
                    planted_at: placement,
                    planned_payoff: None,
                    notes: Vec::new(),
                },
            ],
            dry_run: false,
        })
        .await
        .unwrap();
    assert_eq!(promises.created, 2);
    assert_eq!(
        svc.repository()
            .list_narrative_promises_by_project(&project_id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn voice_batch_updates_all_or_nothing_and_rejects_wrong_branch() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Batch Voice").await;
    let branch_id = svc
        .repository()
        .active_branch_id_public(&project_id)
        .await
        .unwrap();

    let mut characters = Vec::new();
    for name in ["Mira", "Tarek"] {
        let character = svc
            .create_character(CreateCharacterInput {
                project_id: project_id.clone(),
                name: name.to_string(),
                summary: "A character.".to_string(),
                role: "supporting".to_string(),
                realm: None,
                aliases: Vec::new(),
                voice_profile: Default::default(),
                emotional_profile: Default::default(),
                initial_state: None,
            })
            .await
            .unwrap();
        characters.push(character.character_id);
    }

    let out = svc
        .set_character_voice_profiles(SetCharacterVoiceProfilesInput {
            project_id: project_id.clone(),
            branch_id: branch_id.clone(),
            items: vec![
                BatchSetCharacterVoiceProfileItem {
                    character_id: characters[0].clone(),
                    profile: CharacterVoiceProfileData {
                        tone: Some("dry".to_string()),
                        vocabulary: vec!["ledger".to_string()],
                        ..Default::default()
                    },
                },
                BatchSetCharacterVoiceProfileItem {
                    character_id: characters[1].clone(),
                    profile: CharacterVoiceProfileData {
                        tone: Some("warm".to_string()),
                        ..Default::default()
                    },
                },
            ],
            dry_run: false,
        })
        .await
        .unwrap();
    assert_eq!(out.updated, 2);
    assert_eq!(
        svc.repository()
            .get_character_voice_profile(&characters[0])
            .await
            .unwrap()
            .tone
            .as_deref(),
        Some("dry")
    );

    let wrong_branch = svc
        .set_character_voice_profiles(SetCharacterVoiceProfilesInput {
            project_id,
            branch_id: "branch:missing".to_string(),
            items: vec![BatchSetCharacterVoiceProfileItem {
                character_id: characters[0].clone(),
                profile: CharacterVoiceProfileData {
                    tone: Some("wrong".to_string()),
                    ..Default::default()
                },
            }],
            dry_run: true,
        })
        .await
        .unwrap_err();
    assert!(
        format!("{wrong_branch:#}").contains("active branch"),
        "cross-branch batches must be rejected: {wrong_branch:#}"
    );
}

#[tokio::test]
async fn child_branch_can_shadow_inherited_term_and_motif_names() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Branch Shadowing").await;
    let parent_branch = svc
        .repository()
        .active_branch_id_public(&project_id)
        .await
        .unwrap();

    let mut parent_term = term("Aether");
    parent_term.definition = "Parent Aether.".to_string();
    let parent_term_id = svc
        .create_terms(CreateTermsInput {
            project_id: project_id.clone(),
            items: vec![parent_term],
            dry_run: false,
        })
        .await
        .unwrap()
        .term_ids[0]
        .clone();
    let parent_motif_id = svc
        .create_motifs(CreateMotifsInput {
            project_id: project_id.clone(),
            items: vec![BatchCreateMotifItem {
                name: "Salt".to_string(),
                description: "Parent salt.".to_string(),
                max_uses_per_chapter: None,
                connected_theme_ids: Vec::new(),
            }],
            dry_run: false,
        })
        .await
        .unwrap()
        .motif_ids[0]
        .clone();

    let child = svc
        .create_branch(CreateBranchInput {
            project_id: project_id.clone(),
            name: "child".to_string(),
            branch_type: "alternate".to_string(),
            description: None,
            parent_branch_id: Some(parent_branch),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project_id.clone(),
        branch_id: child.branch_id.clone(),
    })
    .await
    .unwrap();

    // Dry run and live canonical/alias calls may create branch-local
    // replacements for inherited names; the parent rows stay untouched.
    let term_dry = svc
        .create_terms(CreateTermsInput {
            project_id: project_id.clone(),
            items: vec![term("Aether")],
            dry_run: true,
        })
        .await
        .expect("child term shadow dry run");
    assert_eq!(term_dry.created, 0);
    let mut child_term = term("Aether");
    child_term.definition = "Child Aether.".to_string();
    let term_live = svc
        .create_terms(CreateTermsInput {
            project_id: project_id.clone(),
            items: vec![child_term],
            dry_run: false,
        })
        .await
        .expect("child term shadow live");
    assert_eq!(term_live.created, 1);
    let child_term_id = term_live.term_ids[0].clone();

    let motif_dry = svc
        .create_motifs(CreateMotifsInput {
            project_id: project_id.clone(),
            items: vec![BatchCreateMotifItem {
                name: "Salt".to_string(),
                description: "Child salt.".to_string(),
                max_uses_per_chapter: None,
                connected_theme_ids: Vec::new(),
            }],
            dry_run: true,
        })
        .await
        .expect("child motif shadow dry run");
    assert_eq!(motif_dry.created, 0);
    let motif_alias = svc
        .batch_create_motifs(spindle_core::models::BatchCreateMotifsInput {
            project_id: project_id.clone(),
            items: vec![BatchCreateMotifItem {
                name: "Salt".to_string(),
                description: "Child salt via alias.".to_string(),
                max_uses_per_chapter: None,
                connected_theme_ids: Vec::new(),
            }],
            dry_run: None,
        })
        .await
        .expect("child motif shadow via alias");
    assert_eq!(motif_alias.created, 1);
    let child_motif_id = motif_alias.motif_ids[0].clone();

    assert_eq!(
        svc.repository()
            .get_term(&parent_term_id)
            .await
            .unwrap()
            .definition,
        "Parent Aether."
    );
    assert_eq!(
        svc.repository()
            .get_term(&child_term_id)
            .await
            .unwrap()
            .definition,
        "Child Aether."
    );
    assert_eq!(
        svc.repository()
            .get_motif(&parent_motif_id)
            .await
            .unwrap()
            .description,
        "Parent salt."
    );
    assert_eq!(
        svc.repository()
            .get_motif(&child_motif_id)
            .await
            .unwrap()
            .description,
        "Child salt via alias."
    );
}
