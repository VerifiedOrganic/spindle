use super::*;
use serde_json::json;
use spindle_core::models::{PrepareSceneOutput, StoryPlacement};

async fn invoke_json(router: &ToolRouter, name: &str, args: Value) -> Value {
    let map = args.as_object().cloned().unwrap_or_default();
    let result = router
        .call_tool(name, Some(&map))
        .await
        .unwrap_or_else(|err| panic!("{name} failed: {err:#}"));
    if result.is_error == Some(true) {
        panic!(
            "{name} returned tool error: {}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    }
    structured_json(result)
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_scene_single_call_assembles_ordinary_scene_without_mutating_cursor() {
    let router = router().await;

    let project = invoke_json(
        &router,
        "create_project",
        json!({
            "name": "Prepare Scene Novel",
            "project_type": "novel",
            "genre": "fantasy",
            "reader_contract": {
                "promise": "Fast-paced intrigue with sharp banter",
                "style_notes": ["Keep dialogue brisk"],
                "boundaries": ["No modern slang"]
            }
        }),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap().to_string();

    let mara = invoke_json(
        &router,
        "create_character",
        json!({
            "project_id": project_id,
            "name": "Mara",
            "role": "protagonist",
            "summary": "A sharp-eyed courier."
        }),
    )
    .await;
    let mara_id = mara["character_id"].as_str().unwrap().to_string();

    let loc = invoke_json(
        &router,
        "create_location",
        json!({
            "project_id": project_id,
            "name": "Glass Wharf",
            "kind": "harbor",
            "summary": "Lantern-lit canal docks."
        }),
    )
    .await;
    let location_id = loc["location_id"].as_str().unwrap().to_string();

    invoke_json(
        &router,
        "create_world_rule",
        json!({
            "project_id": project_id,
            "rule_name": "Tide Bells",
            "rule_type": "lore",
            "description": "Canal locks close at the third bell."
        }),
    )
    .await;

    invoke_json(
        &router,
        "create_world_rule",
        json!({
            "project_id": project_id,
            "rule_name": "Clipped Cadence",
            "rule_type": "style",
            "description": "Open action beats with concrete verbs."
        }),
    )
    .await;

    invoke_json(
        &router,
        "plan_chapter",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "pov_character_id": mara_id,
            "synopsis": "Mara intercepts the coded manifest at Glass Wharf.",
            "scenes": [{
                "scene_order": 1,
                "summary": "Mara bribes the lock-keeper before the third bell.",
                "beat_structure": ["Approach the lockhouse", "Trade the cipher coin"],
                "character_ids": [mara_id],
                "location_id": location_id,
                "content_rating": "general",
                "purpose": "Establish the courier handoff."
            }]
        }),
    )
    .await;

    let saved = invoke_json(
        &router,
        "save_scene_draft",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "full_text": "Mara flipped the brass token across the wet counter before the third bell rang.",
            "summary": "Mara bribes the lock-keeper before the third bell.",
            "content_rating": "general",
            "location_id": location_id,
            "authorship": "assistant"
        }),
    )
    .await;
    let scene_id = saved["scene_id"].as_str().unwrap().to_string();

    invoke_json(
        &router,
        "update_writer_position",
        json!({
            "project_id": project_id,
            "scene_id": scene_id,
            "intent": "revising",
            "next_focus": "Tighten the lockhouse exchange."
        }),
    )
    .await;

    let src = invoke_json(
        &router,
        "research_add_source",
        json!({
            "project_id": project_id,
            "title": "Canal Lock Manual",
            "source_type": "reference",
            "reliability": "high",
            "tags": ["locks"],
            "summary": "Lock gates require two counterweights."
        }),
    )
    .await;
    invoke_json(
        &router,
        "research_add_claim",
        json!({
            "project_id": project_id,
            "source_id": src["source_id"].as_str().unwrap(),
            "claim": "Lock gates require two brass counterweights to seal.",
            "confidence": "high",
            "tags": ["locks"]
        }),
    )
    .await;

    let active_branch = router
        .service
        .repository()
        .get_active_branch(&project_id)
        .await
        .unwrap();
    let position_before = router
        .service
        .repository()
        .get_writer_position(&project_id, &active_branch.id)
        .await
        .unwrap();

    let prepared_val = invoke_json(
        &router,
        "prepare_scene",
        json!({
            "project_id": project_id,
            "intent": "revise",
            "research_query": "counterweights"
        }),
    )
    .await;
    let prepared: PrepareSceneOutput =
        serde_json::from_value(prepared_val).expect("deserialize PrepareSceneOutput");

    let position_after = router
        .service
        .repository()
        .get_writer_position(&project_id, &active_branch.id)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&position_before).unwrap(),
        serde_json::to_value(&position_after).unwrap(),
        "prepare_scene must not mutate writer_position"
    );

    assert!(prepared.preparation_id.starts_with("prep:"));
    assert_eq!(prepared.project_id, project_id);
    assert_eq!(prepared.branch_id, active_branch.id);
    assert_eq!(prepared.scene_id.as_deref(), Some(scene_id.as_str()));
    assert_eq!(prepared.scene_version, Some(1));
    assert_eq!(
        prepared.writer_cursor.cursor_scene_id.as_deref(),
        Some(scene_id.as_str())
    );
    assert_eq!(
        prepared.writer_cursor.notes.as_deref(),
        Some("Tighten the lockhouse exchange.")
    );
    assert_eq!(
        prepared.scene_purpose.chapter_synopsis.as_deref(),
        Some("Mara intercepts the coded manifest at Glass Wharf.")
    );
    assert_eq!(
        prepared.scene_purpose.pov_character_id.as_deref(),
        Some(mara_id.as_str())
    );
    assert_eq!(prepared.scene_purpose.location_id, location_id);
    assert_eq!(
        prepared.scene_purpose.expected_beats,
        vec![
            "Approach the lockhouse".to_string(),
            "Trade the cipher coin".to_string()
        ]
    );
    assert!(
        prepared
            .existing_prose
            .as_deref()
            .is_some_and(|text| text.contains("Mara flipped the brass token"))
    );
    assert!(
        prepared
            .hard_constraints
            .iter()
            .any(|hc| hc.id == "Tide Bells")
    );
    assert!(
        prepared
            .hard_constraints
            .iter()
            .any(|hc| hc.id == "[STYLE DIRECTIVE] Clipped Cadence")
    );
    assert!(prepared.style_directive.is_some());
    assert!(prepared.compact_shelf_digest.is_some());
    assert!(!prepared.research.is_empty());
    assert!(prepared.drafting_packet_v1.is_some());
    assert!(prepared.coverage.estimated_tokens <= prepared.coverage.budget_tokens);
    assert!(prepared.coverage.hard_constraint_tokens > 0);
    assert!(prepared.coverage.truncated_sections.is_empty());
    assert!(
        prepared
            .coverage
            .evidence_references
            .iter()
            .any(|r| r.kind == "scene" && r.id == scene_id)
    );
    assert!(
        prepared
            .coverage
            .evidence_references
            .iter()
            .any(|r| r.kind == "research")
    );

    // Draft intent omits existing_prose so fresh drafting is not primed with old prose.
    let draft_val = invoke_json(
        &router,
        "prepare_scene",
        json!({
            "project_id": project_id,
            "scene_id": scene_id,
            "intent": "draft"
        }),
    )
    .await;
    let draft_prepared: PrepareSceneOutput = serde_json::from_value(draft_val).unwrap();
    assert!(draft_prepared.existing_prose.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_scene_branch_and_secret_visibility() {
    let router = router().await;

    let project = invoke_json(
        &router,
        "create_project",
        json!({
            "name": "Secret & Branch Test",
            "project_type": "novel",
            "genre": "thriller",
            "reader_contract": {
                "promise": "Tight suspense",
                "style_notes": [],
                "boundaries": []
            }
        }),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap().to_string();
    let main_branch_id = router
        .service
        .repository()
        .get_active_branch(&project_id)
        .await
        .unwrap()
        .id;

    let mara = invoke_json(
        &router,
        "create_character",
        json!({
            "project_id": project_id,
            "name": "Mara",
            "role": "protagonist",
            "summary": "Keeper of the vault key."
        }),
    )
    .await;
    let mara_id = mara["character_id"].as_str().unwrap().to_string();

    let corin = invoke_json(
        &router,
        "create_character",
        json!({
            "project_id": project_id,
            "name": "Corin",
            "role": "supporting",
            "summary": "Dock inspector."
        }),
    )
    .await;
    let corin_id = corin["character_id"].as_str().unwrap().to_string();

    let loc = invoke_json(
        &router,
        "create_location",
        json!({
            "project_id": project_id,
            "name": "Vault Antechamber",
            "kind": "interior",
            "summary": "Stone chamber beneath the guildhall."
        }),
    )
    .await;
    let location_id = loc["location_id"].as_str().unwrap().to_string();

    let secret_fact = invoke_json(
        &router,
        "register_canonical_fact",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "subject_table": "character",
            "subject_id": mara_id,
            "predicate": "vault_combination",
            "value_kind": "text",
            "value_text": "7-19-43",
            "scope": "invariant",
            "secrecy": {
                "holder_ids": [mara_id],
                "concealment_note": "Only Mara knows the vault combination"
            }
        }),
    )
    .await;
    let _fact_id = secret_fact["canonical_fact_id"].as_str().unwrap();

    // When both Mara (knower) and Corin (non-knower) are present, [SECRETS IN PLAY] envelope appears.
    let both_val = invoke_json(
        &router,
        "prepare_scene",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "character_ids": [mara_id, corin_id],
            "location_id": location_id
        }),
    )
    .await;
    let both_prepared: PrepareSceneOutput = serde_json::from_value(both_val).unwrap();
    assert!(
        both_prepared
            .hard_constraints
            .iter()
            .any(|hc| hc.id.contains("SECRETS IN PLAY") && hc.statement.contains("Corin")),
        "expected [SECRETS IN PLAY] envelope naming Corin"
    );

    // When only Corin (non-knower) is present, the secret value 7-19-43 is completely withheld.
    let corin_only_val = invoke_json(
        &router,
        "prepare_scene",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "character_ids": [corin_id],
            "location_id": location_id
        }),
    )
    .await;
    let corin_serialized = serde_json::to_string(&corin_only_val).unwrap();
    assert!(
        !corin_serialized.contains("7-19-43"),
        "secret fact value must not leak when no knower is in the scene"
    );

    // Create an alternate branch, switch to it, add a branch-specific world rule, then switch back to main.
    let alt_branch = invoke_json(
        &router,
        "create_branch",
        json!({
            "project_id": project_id,
            "name": "alt-timeline",
            "branch_type": "what_if"
        }),
    )
    .await;
    let alt_branch_id = alt_branch["branch_id"].as_str().unwrap().to_string();

    invoke_json(
        &router,
        "switch_branch",
        json!({
            "project_id": project_id,
            "branch_id": alt_branch_id
        }),
    )
    .await;

    invoke_json(
        &router,
        "create_world_rule",
        json!({
            "project_id": project_id,
            "rule_name": "Alt Branch Curfew",
            "rule_type": "lore",
            "description": "The Vault Antechamber is flooded on the alternate branch."
        }),
    )
    .await;

    invoke_json(
        &router,
        "switch_branch",
        json!({
            "project_id": project_id,
            "branch_id": main_branch_id
        }),
    )
    .await;

    let main_prepared: PrepareSceneOutput = serde_json::from_value(
        invoke_json(
            &router,
            "prepare_scene",
            json!({
                "project_id": project_id,
                "branch_id": main_branch_id,
                "book_number": 1,
                "chapter_number": 1,
                "scene_order": 1,
                "character_ids": [mara_id],
                "location_id": location_id
            }),
        )
        .await,
    )
    .unwrap();
    assert!(
        !main_prepared
            .hard_constraints
            .iter()
            .any(|hc| hc.id == "Alt Branch Curfew"),
        "main branch must not see alt-branch world rule"
    );

    let alt_prepared: PrepareSceneOutput = serde_json::from_value(
        invoke_json(
            &router,
            "prepare_scene",
            json!({
                "project_id": project_id,
                "branch_id": alt_branch_id,
                "book_number": 1,
                "chapter_number": 1,
                "scene_order": 1,
                "character_ids": [mara_id],
                "location_id": location_id
            }),
        )
        .await,
    )
    .unwrap();
    assert_eq!(alt_prepared.branch_id, alt_branch_id);
    assert!(
        alt_prepared
            .hard_constraints
            .iter()
            .any(|hc| hc.id == "Alt Branch Curfew"),
        "alt branch must surface its branch-scoped world rule"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_scene_preserves_hard_constraints_and_reports_truncation_and_unavailable_passes() {
    let router = router().await;

    let project = invoke_json(
        &router,
        "create_project",
        json!({
            "name": "Budget & Coverage Test",
            "project_type": "novel",
            "genre": "fantasy",
            "reader_contract": {
                "promise": "Epic scope with strict magic rules",
                "style_notes": ["Grounded sensory detail"],
                "boundaries": []
            }
        }),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap().to_string();

    let loc = invoke_json(
        &router,
        "create_location",
        json!({
            "project_id": project_id,
            "name": "Obsidian Spire",
            "kind": "tower",
            "summary": "A wind-scoured basalt tower."
        }),
    )
    .await;
    let location_id = loc["location_id"].as_str().unwrap().to_string();

    invoke_json(
        &router,
        "create_world_rule",
        json!({
            "project_id": project_id,
            "rule_name": "Iron Law of Salt",
            "rule_type": "magic",
            "description": "Every spell cast inside Obsidian Spire consumes a full measure of sea salt and leaves frost on the caster's hands."
        }),
    )
    .await;

    // 1. With no characters in the project, prepare_scene succeeds and reports drafting_packet_v1 in unavailable_passes.
    let no_char_val = invoke_json(
        &router,
        "prepare_scene",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "location_id": location_id
        }),
    )
    .await;
    let no_char_prepared: PrepareSceneOutput = serde_json::from_value(no_char_val).unwrap();
    assert!(no_char_prepared.drafting_packet_v1.is_none());
    assert!(
        no_char_prepared
            .coverage
            .unavailable_passes
            .iter()
            .any(|p| p.pass == "drafting_packet_v1"),
        "expected unavailable_passes to report drafting_packet_v1 when no character exists"
    );

    let char_out = invoke_json(
        &router,
        "create_character",
        json!({
            "project_id": project_id,
            "name": "Soren",
            "role": "protagonist",
            "summary": "An apprentice salt-binder at Obsidian Spire."
        }),
    )
    .await;
    let soren_id = char_out["character_id"].as_str().unwrap().to_string();

    // 2. When budget_tokens is too small to fit hard constraints, prepare_scene fails naming the minimum required budget.
    let too_small_args = json!({
        "project_id": project_id,
        "book_number": 1,
        "chapter_number": 1,
        "scene_order": 1,
        "character_ids": [soren_id],
        "location_id": location_id,
        "budget_tokens": 5
    });
    let too_small = router
        .call_tool("prepare_scene", too_small_args.as_object())
        .await
        .expect("dispatch result");
    assert_eq!(too_small.is_error, Some(true));
    let err_text = serde_json::to_string(&too_small).unwrap();
    assert!(
        err_text.contains("too small to fit hard constraints")
            && err_text.contains("minimum required:"),
        "expected minimum required budget error, got: {err_text}"
    );

    // 3. When budget_tokens fits hard constraints but is tighter than full supplementary context,
    // hard constraints are preserved and truncated_sections lists the dropped sections.
    let tight_budget = no_char_prepared.coverage.hard_constraint_tokens + 20;
    let tight_val = invoke_json(
        &router,
        "prepare_scene",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "character_ids": [soren_id],
            "location_id": location_id,
            "budget_tokens": tight_budget
        }),
    )
    .await;
    let tight_prepared: PrepareSceneOutput = serde_json::from_value(tight_val).unwrap();
    assert!(
        tight_prepared
            .hard_constraints
            .iter()
            .any(|hc| hc.id == "Iron Law of Salt")
    );
    assert!(tight_prepared.coverage.estimated_tokens <= tight_budget);
    assert!(
        !tight_prepared.coverage.truncated_sections.is_empty(),
        "expected tight budget to truncate low-priority supplementary sections"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_scene_dependency_tokens_detect_scene_canon_style_and_plan_staleness() {
    let router = router().await;

    let project = invoke_json(
        &router,
        "create_project",
        json!({
            "name": "Freshness Token Test",
            "project_type": "novel",
            "genre": "mystery",
            "reader_contract": {
                "promise": "Fair-play clues",
                "style_notes": [],
                "boundaries": []
            }
        }),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap().to_string();

    let char_out = invoke_json(
        &router,
        "create_character",
        json!({
            "project_id": project_id,
            "name": "Inspector Vale",
            "role": "protagonist",
            "summary": "Methodical investigator."
        }),
    )
    .await;
    let vale_id = char_out["character_id"].as_str().unwrap().to_string();

    let loc = invoke_json(
        &router,
        "create_location",
        json!({
            "project_id": project_id,
            "name": "Clocktower Study",
            "kind": "room",
            "summary": "Oak-paneled study."
        }),
    )
    .await;
    let location_id = loc["location_id"].as_str().unwrap().to_string();

    let saved = invoke_json(
        &router,
        "save_scene_draft",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "full_text": "Vale examined the stopped pocket watch on the desk.",
            "summary": "Vale inspects the study.",
            "content_rating": "general",
            "location_id": location_id,
            "authorship": "assistant"
        }),
    )
    .await;
    let scene_id = saved["scene_id"].as_str().unwrap().to_string();

    let placement = StoryPlacement {
        book_number: 1,
        chapter_number: 1,
        scene_order: Some(1),
        note: None,
    };

    let prep1: PrepareSceneOutput = serde_json::from_value(
        invoke_json(
            &router,
            "prepare_scene",
            json!({
                "project_id": project_id,
                "scene_id": scene_id
            }),
        )
        .await,
    )
    .unwrap();

    let fresh1 = router
        .service
        .check_prepare_scene_freshness(
            &project_id,
            &prep1.branch_id,
            &placement,
            Some(&scene_id),
            &prep1.dependency_tokens,
        )
        .await
        .unwrap();
    assert!(fresh1.fresh);
    assert!(fresh1.stale_components.is_empty());

    // 1. Mutate scene text -> only "scene_text" is stale.
    invoke_json(
        &router,
        "save_scene_draft",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "full_text": "Vale examined the shattered pocket watch and noted the 3:17 stoppage.",
            "summary": "Vale inspects the shattered watch.",
            "content_rating": "general",
            "location_id": location_id,
            "authorship": "assistant"
        }),
    )
    .await;
    let after_scene = router
        .service
        .check_prepare_scene_freshness(
            &project_id,
            &prep1.branch_id,
            &placement,
            Some(&scene_id),
            &prep1.dependency_tokens,
        )
        .await
        .unwrap();
    assert!(!after_scene.fresh);
    assert_eq!(after_scene.stale_components, vec!["scene_text".to_string()]);

    // 2. Re-prepare, then mutate canon -> only "canon" is stale.
    let prep2: PrepareSceneOutput = serde_json::from_value(
        invoke_json(
            &router,
            "prepare_scene",
            json!({
                "project_id": project_id,
                "scene_id": scene_id
            }),
        )
        .await,
    )
    .unwrap();
    invoke_json(
        &router,
        "register_canonical_fact",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "subject_table": "character",
            "subject_id": vale_id,
            "predicate": "watch_stoppage_time",
            "value_kind": "text",
            "value_text": "3:17 AM",
            "scope": "invariant"
        }),
    )
    .await;
    let after_canon = router
        .service
        .check_prepare_scene_freshness(
            &project_id,
            &prep2.branch_id,
            &placement,
            Some(&scene_id),
            &prep2.dependency_tokens,
        )
        .await
        .unwrap();
    assert!(!after_canon.fresh);
    assert_eq!(after_canon.stale_components, vec!["canon".to_string()]);

    // 3. Re-prepare, then mutate style -> only "style" is stale.
    let prep3: PrepareSceneOutput = serde_json::from_value(
        invoke_json(
            &router,
            "prepare_scene",
            json!({
                "project_id": project_id,
                "scene_id": scene_id
            }),
        )
        .await,
    )
    .unwrap();
    invoke_json(
        &router,
        "create_world_rule",
        json!({
            "project_id": project_id,
            "rule_name": "No Weather Openings",
            "rule_type": "style",
            "description": "Never open a mystery scene with rain."
        }),
    )
    .await;
    let after_style = router
        .service
        .check_prepare_scene_freshness(
            &project_id,
            &prep3.branch_id,
            &placement,
            Some(&scene_id),
            &prep3.dependency_tokens,
        )
        .await
        .unwrap();
    assert!(!after_style.fresh);
    assert_eq!(after_style.stale_components, vec!["style".to_string()]);

    // 4. Re-prepare, then mutate chapter plan -> only "plan" is stale.
    let prep4: PrepareSceneOutput = serde_json::from_value(
        invoke_json(
            &router,
            "prepare_scene",
            json!({
                "project_id": project_id,
                "scene_id": scene_id
            }),
        )
        .await,
    )
    .unwrap();
    invoke_json(
        &router,
        "plan_chapter",
        json!({
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "pov_character_id": vale_id,
            "synopsis": "Vale discovers the hidden ledger behind the clockface.",
            "scenes": [{
                "scene_order": 1,
                "summary": "Vale inspects the shattered watch and opens the clockface.",
                "beat_structure": ["Check watch", "Open clockface"],
                "character_ids": [vale_id],
                "location_id": location_id,
                "content_rating": "general",
                "purpose": "Clue discovery"
            }]
        }),
    )
    .await;
    let after_plan = router
        .service
        .check_prepare_scene_freshness(
            &project_id,
            &prep4.branch_id,
            &placement,
            Some(&scene_id),
            &prep4.dependency_tokens,
        )
        .await
        .unwrap();
    assert!(!after_plan.fresh);
    assert_eq!(after_plan.stale_components, vec!["plan".to_string()]);
}
