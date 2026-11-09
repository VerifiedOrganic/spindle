use spindle_adapters::ModelRouter;
use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    CharacterArcMilestone, CreateCharacterArcInput, CreateCharacterInput, CreateConflictInput,
    CreateLocationInput, CreateProjectInput, ReaderContract, StatedConsequence, TryFailCycleStep,
    UpdateCharacterArcInput, UpdateCharacterInput, UpdateConflictInput, UpdateEntityInput,
    UpdateLocationInput,
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

#[tokio::test]
async fn character_update_round_trips_and_rename_stays_controlled() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Typed Character").await;
    let character = svc
        .create_character(CreateCharacterInput {
            project_id,
            name: "Mira".to_string(),
            summary: "A courier.".to_string(),
            role: "supporting".to_string(),
            realm: Some("North".to_string()),
            aliases: Vec::new(),
            voice_profile: Default::default(),
            emotional_profile: Default::default(),
            initial_state: None,
        })
        .await
        .unwrap();

    svc.update_character(UpdateCharacterInput {
        character_id: character.character_id.clone(),
        name: None,
        summary: Some("A royal courier.".to_string()),
        role: Some("protagonist".to_string()),
        realm: Some("Riverlands".to_string()),
        appearance: Some("Weather-beaten cloak.".to_string()),
        aliases: Some(vec!["Mira of the Ford".to_string()]),
        allow_rename: None,
    })
    .await
    .unwrap();

    let stored = svc
        .repository()
        .get_character(&character.character_id)
        .await
        .unwrap();
    assert_eq!(stored.summary, "A royal courier.");
    assert_eq!(stored.role, "protagonist");
    assert_eq!(stored.realm.as_deref(), Some("Riverlands"));
    assert_eq!(stored.appearance.as_deref(), Some("Weather-beaten cloak."));
    assert_eq!(stored.aliases, vec!["Mira of the Ford"]);

    let denied = svc
        .update_character(UpdateCharacterInput {
            character_id: character.character_id.clone(),
            name: Some("Mira Vale".to_string()),
            summary: None,
            role: None,
            realm: None,
            appearance: None,
            aliases: None,
            allow_rename: None,
        })
        .await
        .unwrap_err();
    assert!(
        format!("{denied:#}").contains("allow_rename"),
        "unexpected rename error: {denied:#}"
    );

    let renamed = svc
        .update_character(UpdateCharacterInput {
            character_id: character.character_id.clone(),
            name: Some("Mira Vale".to_string()),
            summary: None,
            role: None,
            realm: None,
            appearance: None,
            aliases: None,
            allow_rename: Some(true),
        })
        .await
        .unwrap();
    let report = renamed.rename_report.expect("rename report");
    assert_eq!(report.old_name, "Mira");
    assert_eq!(report.new_name, "Mira Vale");
    let stored = svc
        .repository()
        .get_character(&character.character_id)
        .await
        .unwrap();
    assert_eq!(stored.name, "Mira Vale");
    assert!(stored.aliases.iter().any(|alias| alias == "Mira"));
}

#[tokio::test]
async fn location_conflict_and_arc_updates_round_trip() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Typed World").await;

    let location = svc
        .create_location(CreateLocationInput {
            project_id: project_id.clone(),
            name: "Old Harbor".to_string(),
            kind: "port".to_string(),
            realm: Some("North".to_string()),
            summary: "A working harbor.".to_string(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();
    svc.update_location(UpdateLocationInput {
        location_id: location.location_id.clone(),
        summary: Some("A fog-bound harbor.".to_string()),
        realm: Some("Riverlands".to_string()),
        kind: Some("ruined port".to_string()),
    })
    .await
    .unwrap();
    let stored = svc
        .repository()
        .get_location(&location.location_id)
        .await
        .unwrap();
    assert_eq!(stored.summary, "A fog-bound harbor.");
    assert_eq!(stored.realm.as_deref(), Some("Riverlands"));
    assert_eq!(stored.kind, "ruined port");

    let conflict = svc
        .create_conflict(CreateConflictInput {
            project_id: project_id.clone(),
            name: "The blocked ford".to_string(),
            conflict_type: "external".to_string(),
            stakes: "The company cannot cross.".to_string(),
            escalation_stages: vec!["warning".to_string()],
            expected_total_cycles: Some(1),
            try_fail_cycles: Vec::new(),
            stated_consequences: Vec::new(),
        })
        .await
        .unwrap();
    svc.update_conflict(UpdateConflictInput {
        conflict_id: conflict.conflict_id.clone(),
        stakes: Some("The company is trapped north of the river.".to_string()),
        resolution_summary: Some("They build a ferry.".to_string()),
        conflict_type: Some("environmental".to_string()),
        expected_total_cycles: Some(3),
        try_fail_cycles: Some(vec![TryFailCycleStep {
            attempt_order: 1,
            label: "Ford the river".to_string(),
            outcome: "failure".to_string(),
            cost: Some("A lost pack horse.".to_string()),
            revelation: None,
        }]),
        escalation_stages: Some(vec!["warning".to_string(), "flood".to_string()]),
        stated_consequences: Some(vec![StatedConsequence {
            description: "A delay costs the company a week.".to_string(),
            stated_at: None,
            must_demonstrate_by: None,
            delivered: false,
        }]),
        escalation_demonstrated: Some(vec![None, None]),
    })
    .await
    .unwrap();
    let stored = svc
        .repository()
        .get_conflict(&conflict.conflict_id)
        .await
        .unwrap();
    assert_eq!(stored.stakes, "The company is trapped north of the river.");
    assert_eq!(stored.conflict_type, "environmental");
    assert_eq!(stored.expected_total_cycles, Some(3));
    assert_eq!(stored.escalation_stages, vec!["warning", "flood"]);
    assert_eq!(stored.try_fail_cycles.len(), 1);
    assert_eq!(stored.stated_consequences.len(), 1);

    let character = svc
        .create_character(CreateCharacterInput {
            project_id: project_id.clone(),
            name: "Tarek".to_string(),
            summary: "An engineer.".to_string(),
            role: "supporting".to_string(),
            realm: None,
            aliases: Vec::new(),
            voice_profile: Default::default(),
            emotional_profile: Default::default(),
            initial_state: None,
        })
        .await
        .unwrap();
    let arc = svc
        .create_character_arc(CreateCharacterArcInput {
            project_id,
            character_id: character.character_id,
            arc_type: "positive".to_string(),
            starting_state: "Cautious".to_string(),
            ending_state: "Trusting".to_string(),
            milestones: vec![CharacterArcMilestone {
                label: "First risk".to_string(),
                placement: None,
                description: "Tarek trusts an outsider.".to_string(),
                unlocks: Vec::new(),
                reached_at: None,
            }],
            thematic_purpose: "Trust is earned.".to_string(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();
    svc.update_character_arc(UpdateCharacterArcInput {
        character_arc_id: arc.character_arc_id.clone(),
        arc_type: Some("positive".to_string()),
        starting_state: Some("Cautious".to_string()),
        ending_state: Some("Openly loyal".to_string()),
        thematic_purpose: Some("Trust is built by shared risk.".to_string()),
        connected_theme_ids: None,
        status: Some("active".to_string()),
        milestones: Some(vec![CharacterArcMilestone {
            label: "First risk".to_string(),
            placement: None,
            description: "Tarek trusts an outsider.".to_string(),
            unlocks: vec!["Ferry plan".to_string()],
            reached_at: None,
        }]),
    })
    .await
    .unwrap();
    let stored = svc
        .repository()
        .get_character_arc(&arc.character_arc_id)
        .await
        .unwrap();
    assert_eq!(stored.ending_state, "Openly loyal");
    assert_eq!(stored.status, "active");
    assert_eq!(stored.milestones.len(), 1);
    assert_eq!(stored.milestones[0].unlocks, vec!["Ferry plan"]);
}

#[tokio::test]
async fn typed_inputs_reject_unknown_fields_and_generic_alias_still_works() {
    let unknown = serde_json::json!({"character_id": "character:x", "bogus": true});
    assert!(serde_json::from_value::<UpdateCharacterInput>(unknown).is_err());

    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Generic Alias").await;
    let character = svc
        .create_character(CreateCharacterInput {
            project_id,
            name: "Sela".to_string(),
            summary: "A scout.".to_string(),
            role: "supporting".to_string(),
            realm: None,
            aliases: Vec::new(),
            voice_profile: Default::default(),
            emotional_profile: Default::default(),
            initial_state: None,
        })
        .await
        .unwrap();
    svc.update_entity(UpdateEntityInput {
        entity_type: "character".to_string(),
        entity_id: character.character_id.clone(),
        changes: serde_json::json!({"role": "mentor"}),
        allow_rename: None,
    })
    .await
    .unwrap();
    let stored = svc
        .repository()
        .get_character(&character.character_id)
        .await
        .unwrap();
    assert_eq!(stored.role, "mentor");
}
