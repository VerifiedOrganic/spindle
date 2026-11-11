use super::*;

/// Regression for the stale-cache bug that made the secrecy-scoping fix
/// invisible: the world_rule_semantic_drift context hash keyed only on
/// rule metadata (rule_id, rule_name, scan_pattern, established_in), so
/// findings computed by the pre-secrecy scanner kept matching the cache
/// key and were served forever. The hash must now incorporate the
/// scanner semantics version AND the classification inputs (rule_type,
/// description), so a scanner upgrade — or an edit to either field —
/// invalidates cached findings.
#[test]
fn world_rule_context_hash_covers_scanner_semantics_and_classification_inputs() {
    use spindle_core::validators::{ValidatorContext, WorldRuleSnapshot};

    let context = |rule_type: &str, description: &str| ValidatorContext {
        project_id: "project:p".to_string(),
        branch_id: "branch:b".to_string(),
        scenes: Vec::new(),
        canonical_facts: Vec::new(),
        world_rules: vec![WorldRuleSnapshot {
            rule_id: "world_rule:r".to_string(),
            rule_name: "System Secrecy".to_string(),
            scan_pattern: Some("the system".to_string()),
            established_in: None,
            rule_type: rule_type.to_string(),
            description: description.to_string(),
        }],
        voice_profiles: Vec::new(),
        timeline_events: Vec::new(),
        temporal_interventions: Vec::new(),
        style_directive: None,
    };
    let checks: std::collections::BTreeSet<PhaseFourCacheId> =
        std::iter::once(PhaseFourCacheId::WorldRuleSemanticDrift).collect();

    let hash = phase_four_context_hashes(
        &context("secrecy", "Nate must never disclose the N.A.I.P."),
        &checks,
    )
    .unwrap()
    .get("world_rule_semantic_drift")
    .unwrap()
    .clone();

    // The pre-fix payload (rule metadata only, no scanner version, no
    // classification inputs) must NOT collide with the new hash — this
    // is exactly the key the stale cached findings were stored under.
    let legacy_payload = serde_json::json!({
        "world_rules": [serde_json::json!({
            "rule_id": "world_rule:r",
            "rule_name": "System Secrecy",
            "scan_pattern": "the system",
            "established_in": null,
        })]
    });
    let legacy_hash = generation_sha256_hex(&serde_json::to_vec(&legacy_payload).unwrap());
    assert_ne!(
        hash, legacy_hash,
        "findings cached by the pre-secrecy scanner must no longer match"
    );

    // Classification inputs participate: reclassifying the same rule
    // (or rewording its description) invalidates the cache.
    let reclassified = phase_four_context_hashes(
        &context("magic_limitation", "Different constraint entirely."),
        &checks,
    )
    .unwrap();
    assert_ne!(
        hash,
        reclassified
            .get("world_rule_semantic_drift")
            .unwrap()
            .as_str(),
        "rule_type/description edits must change the context hash"
    );
}

#[tokio::test]
async fn read_project_resource_continuity_health_reports_cache_orphans_and_duplicates() {
    let (_tmp, svc) = fresh_service().await;
    let (project, scene_id, full_text) = project_with_scene(&svc, "continuity-health").await;
    seed_phase_four_cache(
        &svc,
        &project.project_id,
        &project.branch_id,
        &scene_id,
        &full_text,
        PhaseFourCacheId::WorldRuleSemanticDrift,
    )
    .await;

    svc.create_temporal_intervention(CreateTemporalInterventionInput {
        project_id: project.project_id.clone(),
        title: "Warning with missing anchor".to_string(),
        intervention_type: "message".to_string(),
        source_event_id: None,
        target_event_id: None,
        summary: "A warning is missing its source and target events.".to_string(),
        consequences: vec!["continuity needs endpoint repair".to_string()],
        status: Some("planned".to_string()),
    })
    .await
    .unwrap();

    for value_text in ["clear", "stormy"] {
        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("project".to_string()),
            subject_id: None,
            predicate: Some("weather".to_string()),
            value_kind: Some("string".to_string()),
            value_text: Some(value_text.to_string()),
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
    }

    let health = svc
        .read_project_resource(&project.project_id, "continuity/health")
        .await
        .unwrap();

    assert_eq!(
        health["active_branch_id"].as_str(),
        Some(project.branch_id.as_str())
    );
    assert_eq!(health["branch_count"].as_u64(), Some(1));
    assert_eq!(health["branch_lineage"].as_array().unwrap().len(), 1);
    assert_eq!(
        health["validator_findings"]["open_by_validator_and_severity"]
            [PhaseFourCacheId::WorldRuleSemanticDrift.as_str()]["info"]
            .as_u64(),
        Some(1)
    );
    assert_eq!(
            health["validator_cache_counts"][PhaseFourCacheId::WorldRuleSemanticDrift.as_str()]
                ["active_count"]
                .as_u64(),
            Some(1)
        );
    assert_eq!(
        health["validator_cache_totals"]["active_count"].as_u64(),
        Some(1)
    );
    assert!(health["last_check_consistency_at"].is_null());

    let orphans = health["orphaned_temporal_interventions"]
        .as_array()
        .unwrap();
    assert_eq!(orphans.len(), 1);
    let missing_endpoints = orphans[0]["missing_endpoints"].as_array().unwrap();
    assert_eq!(missing_endpoints.len(), 2);
    assert!(missing_endpoints.iter().any(|endpoint| {
        endpoint["field"].as_str() == Some("source_event_id")
            && endpoint["reason"].as_str() == Some("unset")
    }));
    assert!(missing_endpoints.iter().any(|endpoint| {
        endpoint["field"].as_str() == Some("target_event_id")
            && endpoint["reason"].as_str() == Some("unset")
    }));

    let duplicates = health["duplicate_active_canonical_facts"]
        .as_array()
        .unwrap();
    assert_eq!(duplicates.len(), 1);
    assert_eq!(duplicates[0]["subject_table"].as_str(), Some("project"));
    assert_eq!(duplicates[0]["subject_id"], serde_json::Value::Null);
    assert_eq!(duplicates[0]["predicate"].as_str(), Some("weather"));
    assert_eq!(duplicates[0]["fact_ids"].as_array().unwrap().len(), 2);
    assert_eq!(duplicates[0]["unique_values"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn create_world_rule_resolves_world_rule_semantic_drift_cache() {
    let (_tmp, svc) = fresh_service().await;
    let (project, scene_id, full_text) = project_with_scene(&svc, "world-cache").await;
    let scene_text_hash = seed_phase_four_cache(
        &svc,
        &project.project_id,
        &project.branch_id,
        &scene_id,
        &full_text,
        PhaseFourCacheId::WorldRuleSemanticDrift,
    )
    .await;

    svc.create_world_rule(CreateWorldRuleInput {
        project_id: project.project_id.clone(),
        rule_name: "No salt magic".to_string(),
        rule_type: "magic".to_string(),
        description: "Salt magic fails near iron bells.".to_string(),
        scan_pattern: Some("salt magic".to_string()),
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();

    assert_active_phase_four_cache(
        &svc,
        &project.branch_id,
        &scene_id,
        &scene_text_hash,
        PhaseFourCacheId::WorldRuleSemanticDrift,
        0,
    )
    .await;
}

#[tokio::test]
async fn update_world_rule_resolves_world_rule_semantic_drift_cache() {
    let (_tmp, svc) = fresh_service().await;
    let (project, scene_id, full_text) = project_with_scene(&svc, "world-update-cache").await;
    let rule = svc
        .create_world_rule(CreateWorldRuleInput {
            project_id: project.project_id.clone(),
            rule_name: "Iron bells disrupt salt magic".to_string(),
            rule_type: "magic".to_string(),
            description: "Salt magic fails near iron bells.".to_string(),
            scan_pattern: Some("salt magic".to_string()),
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();
    let scene_text_hash = seed_phase_four_cache(
        &svc,
        &project.project_id,
        &project.branch_id,
        &scene_id,
        &full_text,
        PhaseFourCacheId::WorldRuleSemanticDrift,
    )
    .await;

    svc.update_world_rule(UpdateWorldRuleInput {
        world_rule_id: rule.world_rule_id,
        changes: serde_json::json!({
            "description": "Salt magic fails near iron bells and cold river stone."
        }),
    })
    .await
    .unwrap();

    assert_active_phase_four_cache(
        &svc,
        &project.branch_id,
        &scene_id,
        &scene_text_hash,
        PhaseFourCacheId::WorldRuleSemanticDrift,
        0,
    )
    .await;
}

#[tokio::test]
async fn context_hash_blocks_stale_world_rule_cache_when_invalidation_is_missed() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};

    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "context-hash-cache".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "context hashes protect cache hits".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let full_text = "Eldrin tried to ignore the sigil and cast at range anyway.".to_string();
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: full_text.clone(),
            summary: "Eldrin ignores a sigil rule.".to_string(),
            content_rating: spindle_core::models::ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let scene_record = svc.repository.get_scene(&scene.scene_id).await.unwrap();
    let phase_four_checks = std::iter::once(PhaseFourCacheId::WorldRuleSemanticDrift)
        .collect::<std::collections::BTreeSet<_>>();
    let old_context = svc
        .build_phase_four_validator_context(
            &project.project_id,
            &project.branch_id,
            std::slice::from_ref(&scene_record),
        )
        .await
        .unwrap();
    let old_context_hash = phase_four_context_hashes(&old_context, &phase_four_checks)
        .unwrap()
        .get(PhaseFourCacheId::WorldRuleSemanticDrift.as_str())
        .cloned()
        .unwrap();
    let scene_text_hash = generation_sha256_hex(full_text.as_bytes());
    svc.repository
        .upsert_validator_finding(crate::sqlite::repository::UpsertValidatorFindingParams {
            project_id: project.project_id.clone(),
            branch_id: project.branch_id.clone(),
            scene_id: scene.scene_id.clone(),
            scene_text_hash: scene_text_hash.clone(),
            context_hash: Some(old_context_hash),
            validator_id: PhaseFourCacheId::WorldRuleSemanticDrift
                .as_str()
                .to_string(),
            finding_id: "__cache__".to_string(),
            severity: "warning".to_string(),
            message: "stale cache".to_string(),
            byte_range: None,
            details_json: Some(serde_json::json!({
                "issues": [{
                    "severity": "warning",
                    "check_type": "world_rule_semantic_drift",
                    "message": "stale cached issue",
                    "byte_range": null
                }]
            })),
        })
        .await
        .unwrap();

    // Bypass the service-level invalidation on purpose. The context hash
    // must still prevent the old cache row from being used.
    svc.repository
        .create_world_rule(&CreateWorldRuleInput {
            project_id: project.project_id.clone(),
            rule_name: "Sigil contact rule".to_string(),
            rule_type: "magic".to_string(),
            description: "Magic sigils must require physical contact.".to_string(),
            scan_pattern: Some(r"\bsigil\b".to_string()),
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();

    let output = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec![
                PhaseFourCacheId::WorldRuleSemanticDrift
                    .as_str()
                    .to_string(),
            ],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !output
            .issues
            .iter()
            .any(|issue| issue.message.contains("stale cached issue")),
        "stale cache issue should not be returned: {:?}",
        output.issues
    );
    assert!(
        output.issues.iter().any(|issue| {
            issue.check_type == PhaseFourCacheId::WorldRuleSemanticDrift.as_str()
                && issue.message.contains("Sigil contact rule")
        }),
        "fresh world-rule issue should be recomputed: {:?}",
        output.issues
    );

    let active_rows = svc
        .repository
        .list_active_validator_findings_by_scene_hash(
            &project.branch_id,
            &scene.scene_id,
            &scene_text_hash,
            &[PhaseFourCacheId::WorldRuleSemanticDrift
                .as_str()
                .to_string()],
        )
        .await
        .unwrap();
    assert_eq!(
        active_rows.len(),
        1,
        "recompute should leave one active current-context cache row"
    );
    assert!(
        active_rows[0].context_hash.is_some(),
        "current cache row should carry context_hash"
    );
}

#[tokio::test]
async fn register_canonical_fact_resolves_canonical_fact_prose_drift_cache() {
    let (_tmp, svc) = fresh_service().await;
    let (project, scene_id, full_text) = project_with_scene(&svc, "fact-cache").await;
    let scene_text_hash = seed_phase_four_cache(
        &svc,
        &project.project_id,
        &project.branch_id,
        &scene_id,
        &full_text,
        PhaseFourCacheId::CanonicalFactProseDrift,
    )
    .await;

    svc.register_canonical_fact(RegisterCanonicalFactInput {
        project_id: project.project_id.clone(),
        scene_id: Some(scene_id.clone()),
        book_number: 1,
        chapter_number: 1,
        fact_type: Some("typed_fact".to_string()),
        key: Some("scene.opening_state".to_string()),
        value: Some("quiet".to_string()),
        context: None,
        subject_table: Some("scene".to_string()),
        subject_id: Some(scene_id.clone()),
        predicate: Some("opening_state".to_string()),
        value_kind: Some("string".to_string()),
        value_text: Some("quiet".to_string()),
        value_number: None,
        value_unit: None,
        value_json: None,
        aliases: Vec::new(),
        scope: Some(CanonicalFactScope::Invariant),
        valid_from: None,
        valid_until: None,
        legacy_untyped: Some(false),
        supersedes_fact_id: None,
        secrecy: None,
    })
    .await
    .unwrap();

    assert_active_phase_four_cache(
        &svc,
        &project.branch_id,
        &scene_id,
        &scene_text_hash,
        PhaseFourCacheId::CanonicalFactProseDrift,
        0,
    )
    .await;
}

#[tokio::test]
async fn extract_canonical_facts_from_scene_does_not_resolve_cache() {
    let (_tmp, svc) = fresh_service().await;
    let (project, scene_id, full_text) = project_with_scene(&svc, "fact-proposal-cache").await;
    let scene_text_hash = seed_phase_four_cache(
        &svc,
        &project.project_id,
        &project.branch_id,
        &scene_id,
        &full_text,
        PhaseFourCacheId::CanonicalFactProseDrift,
    )
    .await;

    svc.extract_canonical_facts_from_scene(
        spindle_core::models::ExtractCanonicalFactsFromSceneInput {
            scene_id: scene_id.clone(),
        },
    )
    .await
    .unwrap();

    assert_active_phase_four_cache(
        &svc,
        &project.branch_id,
        &scene_id,
        &scene_text_hash,
        PhaseFourCacheId::CanonicalFactProseDrift,
        1,
    )
    .await;
}

#[tokio::test]
async fn timeline_metadata_resolves_retcon_reachability_cache() {
    let (_tmp, svc) = fresh_service().await;
    let (project, scene_id, full_text) = project_with_scene(&svc, "retcon-cache").await;
    let scene_text_hash = seed_phase_four_cache(
        &svc,
        &project.project_id,
        &project.branch_id,
        &scene_id,
        &full_text,
        PhaseFourCacheId::RetconReachability,
    )
    .await;

    let first_event = svc
        .create_timeline_event(CreateTimelineEventInput {
            project_id: project.project_id.clone(),
            title: "First anchor".to_string(),
            event_type: "anchor".to_string(),
            placement: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            summary: "First anchor".to_string(),
            related_entity_ids: Vec::new(),
        })
        .await
        .unwrap();

    assert_active_phase_four_cache(
        &svc,
        &project.branch_id,
        &scene_id,
        &scene_text_hash,
        PhaseFourCacheId::RetconReachability,
        0,
    )
    .await;

    let second_event = svc
        .create_timeline_event(CreateTimelineEventInput {
            project_id: project.project_id.clone(),
            title: "Second anchor".to_string(),
            event_type: "anchor".to_string(),
            placement: StoryPlacement {
                book_number: 1,
                chapter_number: 2,
                scene_order: Some(1),
                note: None,
            },
            summary: "Second anchor".to_string(),
            related_entity_ids: Vec::new(),
        })
        .await
        .unwrap();
    let scene_text_hash = seed_phase_four_cache(
        &svc,
        &project.project_id,
        &project.branch_id,
        &scene_id,
        &full_text,
        PhaseFourCacheId::RetconReachability,
    )
    .await;

    svc.create_temporal_intervention(CreateTemporalInterventionInput {
        project_id: project.project_id.clone(),
        title: "Warning sent backward".to_string(),
        intervention_type: "message".to_string(),
        source_event_id: Some(second_event.timeline_event_id),
        target_event_id: Some(first_event.timeline_event_id),
        summary: "A warning is sent backward.".to_string(),
        consequences: vec!["changes the first anchor".to_string()],
        status: Some("planned".to_string()),
    })
    .await
    .unwrap();

    assert_active_phase_four_cache(
        &svc,
        &project.branch_id,
        &scene_id,
        &scene_text_hash,
        PhaseFourCacheId::RetconReachability,
        0,
    )
    .await;
}

#[tokio::test]
async fn set_narrator_voice_resolves_style_compliance_cache() {
    let (_tmp, svc) = fresh_service().await;
    let (project, scene_id, full_text) = project_with_scene(&svc, "style-cache").await;
    let scene_text_hash = seed_phase_four_cache(
        &svc,
        &project.project_id,
        &project.branch_id,
        &scene_id,
        &full_text,
        PhaseFourCacheId::StyleCompliance,
    )
    .await;

    svc.set_narrator_voice(spindle_core::models::SetNarratorVoiceInput {
        project_id: project.project_id.clone(),
        narrator_voice: spindle_core::style::NarratorVoice {
            emotional_register: Some("funny-and-sarcastic".to_string()),
            ..Default::default()
        },
    })
    .await
    .unwrap();

    assert_active_phase_four_cache(
        &svc,
        &project.branch_id,
        &scene_id,
        &scene_text_hash,
        PhaseFourCacheId::StyleCompliance,
        0,
    )
    .await;
}
