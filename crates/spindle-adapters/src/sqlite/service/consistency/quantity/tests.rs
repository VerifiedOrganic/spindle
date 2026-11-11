use super::*;

#[tokio::test]
async fn quantity_scheme_and_state_round_trip() {
    use crate::sqlite::repository::AppendQuantityStateParams;
    use spindle_core::models::{QuantityBand, QuantityDenomination, QuantityScheme, QuantityState};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "qty-roundtrip".into(),
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
    let repo = svc.repository();
    let branch_id = repo.get_active_branch(&proj.project_id).await.unwrap().id;

    // Additivity: a project with no scheme reads back None.
    assert!(
        repo.get_project_quantity_scheme(&proj.project_id, &branch_id, "wealth")
            .await
            .unwrap()
            .is_none()
    );

    let scheme = QuantityScheme {
        measure: "wealth".into(),
        denominations: vec![
            QuantityDenomination {
                name: "gold".into(),
                per_base: 100,
            },
            QuantityDenomination {
                name: "copper".into(),
                per_base: 1,
            },
        ],
        bands: vec![
            QuantityBand {
                name: "destitute".into(),
                lower_bound: Some(0),
            },
            QuantityBand {
                name: "comfortable".into(),
                lower_bound: Some(100),
            },
            QuantityBand {
                name: "wealthy".into(),
                lower_bound: Some(10_000),
            },
        ],
        max_band_jump: Some(1),
    };
    repo.upsert_project_quantity_scheme(&proj.project_id, &branch_id, &scheme)
        .await
        .unwrap();
    let loaded = repo
        .get_project_quantity_scheme(&proj.project_id, &branch_id, "wealth")
        .await
        .unwrap()
        .expect("scheme present after upsert");
    assert_eq!(loaded.scheme, scheme, "scheme round-trips losslessly");

    let pos = |book: i32, chapter: i32, scene_order: i32| -> i64 {
        book as i64 * 1_000_000 + chapter as i64 * 1_000 + scene_order as i64
    };
    let mk = |chapter: i32, band: &str| AppendQuantityStateParams {
        project_id: proj.project_id.clone(),
        branch_id: branch_id.clone(),
        subject_table: "character".into(),
        subject_id: "character:hero".into(),
        measure: "wealth".into(),
        state: QuantityState {
            band: Some(band.into()),
            ..Default::default()
        },
        scene_id: None,
        book_number: 1,
        chapter_number: chapter,
        scene_order: 1,
    };
    repo.append_quantity_state(mk(1, "destitute"))
        .await
        .unwrap();
    repo.append_quantity_state(mk(5, "comfortable"))
        .await
        .unwrap();

    // At ch3 the latest stamp is the ch1 'destitute' reading.
    let at_ch3 = repo
        .latest_quantity_state_at_or_before(
            &proj.project_id,
            &branch_id,
            "character",
            "character:hero",
            "wealth",
            pos(1, 3, 0),
        )
        .await
        .unwrap()
        .expect("state at or before ch3");
    assert_eq!(at_ch3.state.band.as_deref(), Some("destitute"));

    // At ch10 the latest stamp is the ch5 'comfortable' reading.
    let at_ch10 = repo
        .latest_quantity_state_at_or_before(
            &proj.project_id,
            &branch_id,
            "character",
            "character:hero",
            "wealth",
            pos(1, 10, 0),
        )
        .await
        .unwrap()
        .expect("state at or before ch10");
    assert_eq!(at_ch10.state.band.as_deref(), Some("comfortable"));
}

#[tokio::test]
async fn quantity_tools_validate_and_persist() {
    use spindle_core::models::{
        CommitQuantityStateInput, QuantityBand, QuantityScheme, QuantityState,
        SetProjectQuantitySchemeInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "qty-tools".into(),
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

    // An invalid scheme (empty measure) is rejected.
    assert!(
        svc.set_project_quantity_scheme(SetProjectQuantitySchemeInput {
            project_id: proj.project_id.clone(),
            scheme: QuantityScheme {
                measure: String::new(),
                ..Default::default()
            },
        })
        .await
        .is_err(),
        "empty-measure scheme must be rejected"
    );

    let set = svc
        .set_project_quantity_scheme(SetProjectQuantitySchemeInput {
            project_id: proj.project_id.clone(),
            scheme: QuantityScheme {
                measure: "wealth".into(),
                denominations: Vec::new(),
                bands: vec![
                    QuantityBand {
                        name: "destitute".into(),
                        lower_bound: None,
                    },
                    QuantityBand {
                        name: "wealthy".into(),
                        lower_bound: None,
                    },
                ],
                max_band_jump: Some(1),
            },
        })
        .await
        .unwrap();
    assert_eq!(set.measure, "wealth");

    let commit = |band: &str| CommitQuantityStateInput {
        project_id: proj.project_id.clone(),
        subject_table: "character".into(),
        subject_id: "character:hero".into(),
        measure: "wealth".into(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
        scene_id: None,
        state: QuantityState {
            band: Some(band.into()),
            ..Default::default()
        },
    };

    // A reading whose band is not in the scheme is rejected.
    assert!(
        svc.commit_quantity_state(commit("mythic")).await.is_err(),
        "undeclared band must be rejected against the scheme"
    );

    // A valid reading persists and returns a stamped id.
    let ok = svc
        .commit_quantity_state(commit("destitute"))
        .await
        .unwrap();
    assert!(ok.quantity_state_id.starts_with("quantity_state:"));
}

#[tokio::test]
async fn quantity_drift_flags_unexplained_band_jump() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project_with_wealth_scheme(&svc, "qty-drift").await;

    svc.commit_quantity_state(wealth_commit(&project_id, 1, "destitute", None))
        .await
        .unwrap();
    // destitute -> wealthy is a 2-tier jump with no reason: warned at commit.
    let jumped = svc
        .commit_quantity_state(wealth_commit(&project_id, 2, "wealthy", None))
        .await
        .unwrap();
    assert!(
        !jumped.warnings.is_empty(),
        "unexplained band jump should warn at commit"
    );

    let out = svc
        .check_consistency(quantity_drift_input(&project_id))
        .await
        .unwrap();
    assert!(
        out.issues.iter().any(|i| i.check_type == "quantity_drift"),
        "check_consistency should flag the unexplained band jump: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn quantity_band_jump_with_reason_is_clean() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project_with_wealth_scheme(&svc, "qty-reason").await;

    svc.commit_quantity_state(wealth_commit(&project_id, 1, "destitute", None))
        .await
        .unwrap();
    // Same 2-tier jump, but with a change_reason: no warning, no drift.
    let ok = svc
        .commit_quantity_state(wealth_commit(
            &project_id,
            2,
            "wealthy",
            Some("inherited a fortune"),
        ))
        .await
        .unwrap();
    assert!(ok.warnings.is_empty(), "an explained jump must not warn");

    let out = svc
        .check_consistency(quantity_drift_input(&project_id))
        .await
        .unwrap();
    assert!(
        !out.issues.iter().any(|i| i.check_type == "quantity_drift"),
        "an explained jump must not be flagged: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn derive_quantity_scheme_from_overlay_creates_band_scheme() {
    use spindle_core::models::{
        CommitQuantityStateInput, CreateSystemOverlayInput, DeriveQuantitySchemeFromOverlayInput,
        QuantityState,
    };

    let (_tmp, svc) = fresh_service().await;
    let (project, _scene_id, _text) = project_with_scene(&svc, "overlay-scheme").await;
    let overlay = svc
        .create_system_overlay(CreateSystemOverlayInput {
            project_id: project.project_id.clone(),
            system_name: "Cultivation".into(),
            system_type: "cultivation".into(),
            rules: "advance by breakthroughs".into(),
            visibility: "explicit".into(),
            progression_currency: None,
            stats: vec!["qi".into()],
            advancement_tiers: vec!["Mortal".into(), "Foundation".into(), "Core".into()],
        })
        .await
        .unwrap();

    let out = svc
        .derive_quantity_scheme_from_system_overlay(DeriveQuantitySchemeFromOverlayInput {
            project_id: project.project_id.clone(),
            system_overlay_id: overlay.system_overlay_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(out.measure, "Cultivation");
    assert_eq!(out.band_count, 3);

    let commit = |chapter: i32, band: &str| CommitQuantityStateInput {
        project_id: project.project_id.clone(),
        subject_table: "character".into(),
        subject_id: "character:hero".into(),
        measure: "Cultivation".into(),
        book_number: 1,
        chapter_number: chapter,
        scene_order: 1,
        scene_id: None,
        state: QuantityState {
            band: Some(band.into()),
            ..Default::default()
        },
    };
    svc.commit_quantity_state(commit(1, "Mortal"))
        .await
        .unwrap();
    // Mortal -> Core skips Foundation (a 2-tier jump): warned by the derived scheme.
    let jumped = svc.commit_quantity_state(commit(2, "Core")).await.unwrap();
    assert!(!jumped.warnings.is_empty(), "skipping a tier should warn");
    // A tier the overlay never declared is hard-rejected.
    assert!(
        svc.commit_quantity_state(commit(3, "Immortal"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn scan_scene_prices_detects_mentions() {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput, ScanScenePricesInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "scan-prices".into(),
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
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "The merchant wanted 5 silver for bread and 2 gold for the blade.".into(),
            summary: "market".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // No scheme declared: the default coinage vocabulary still finds prices.
    let out = svc
        .scan_scene_prices(ScanScenePricesInput {
            project_id: proj.project_id.clone(),
            scene_id: scene.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(out.mentions.len(), 2, "mentions: {:?}", out.mentions);
    assert!(
        out.mentions
            .iter()
            .any(|m| m.amount == 5.0 && m.unit == "silver")
    );
    assert!(
        out.mentions
            .iter()
            .any(|m| m.amount == 2.0 && m.unit == "gold")
    );
}

#[tokio::test]
async fn currency_consistency_flags_denomination_mismatch() {
    use crate::sqlite::repository::CreateCanonicalFactParams;
    use spindle_core::models::{
        QuantityDenomination, QuantityScheme, SetProjectQuantitySchemeInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let (project, scene_id, _text) = project_with_scene(&svc, "currency-consistency").await;

    svc.set_project_quantity_scheme(SetProjectQuantitySchemeInput {
        project_id: project.project_id.clone(),
        scheme: QuantityScheme {
            measure: "wealth".into(),
            denominations: vec![
                QuantityDenomination {
                    name: "silver".into(),
                    per_base: 10,
                },
                QuantityDenomination {
                    name: "copper".into(),
                    per_base: 1,
                },
            ],
            bands: Vec::new(),
            max_band_jump: None,
        },
    })
    .await
    .unwrap();

    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    // bread = 5 silver (50 base) AND bread = 100 copper (100 base): a contradiction.
    for (number, unit) in [(5.0_f64, "silver"), (100.0_f64, "copper")] {
        svc.repository()
            .create_canonical_fact(CreateCanonicalFactParams {
                project_id: project.project_id.clone(),
                branch_id: branch.id.clone(),
                scene_id: Some(scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                subject_table: "economy".into(),
                subject_id: Some("economy:river".into()),
                predicate: "bread_price".into(),
                value_kind: "number".into(),
                value_text: None,
                value_number: Some(number),
                unit: Some(unit.into()),
                value_json: None,
                aliases: Vec::new(),
                scope: "evolving".into(),
                valid_from: None,
                valid_until: None,
                legacy_untyped: false,
                secret: false,
                concealment_note: None,
            })
            .await
            .unwrap();
    }

    let out = svc
        .check_consistency(spindle_core::models::CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project.project_id.clone(),
            scope: spindle_core::models::ConsistencyScopeInput::full(),
            checks: vec!["currency_consistency".into()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        out.issues
            .iter()
            .any(|i| i.check_type == "currency_consistency"),
        "mismatched denominations must be flagged: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn affordability_flags_unaffordable_named_price() {
    use spindle_core::models::{
        CommitQuantityStateInput, ContentRating, QuantityDenomination, QuantityScheme,
        QuantityState, SaveSceneDraftInput, SetProjectQuantitySchemeInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "affordability".into(),
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
    svc.set_project_quantity_scheme(SetProjectQuantitySchemeInput {
        project_id: proj.project_id.clone(),
        scheme: QuantityScheme {
            measure: "wealth".into(),
            denominations: vec![
                QuantityDenomination {
                    name: "silver".into(),
                    per_base: 10,
                },
                QuantityDenomination {
                    name: "copper".into(),
                    per_base: 1,
                },
            ],
            bands: Vec::new(),
            max_band_jump: None,
        },
    })
    .await
    .unwrap();
    let mara = seed_character(&svc, &proj.project_id, "Mara").await;

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Mara bought the 100 silver blade she could not begin to afford.".into(),
        summary: "market".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // Mara holds 5 silver (= 50 base); the blade is 100 silver (= 1000 base).
    svc.commit_quantity_state(CommitQuantityStateInput {
        project_id: proj.project_id.clone(),
        subject_table: "character".into(),
        subject_id: mara.clone(),
        measure: "wealth".into(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
        scene_id: None,
        state: QuantityState {
            amount: Some(5.0),
            unit: Some("silver".into()),
            ..Default::default()
        },
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(spindle_core::models::CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: spindle_core::models::ConsistencyScopeInput::full(),
            checks: vec!["affordability".into()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        out.issues.iter().any(|i| i.check_type == "affordability"),
        "an unaffordable named price should raise an advisory: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn affordability_resolves_spelled_price_and_pronoun_but_keeps_ambiguity_info() {
    use spindle_core::models::{
        CommitQuantityStateInput, ContentRating, QuantityDenomination, QuantityScheme,
        QuantityState, SaveSceneDraftInput, SetProjectQuantitySchemeInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "affordability-resolution".into(),
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
    svc.set_project_quantity_scheme(SetProjectQuantitySchemeInput {
        project_id: proj.project_id.clone(),
        scheme: QuantityScheme {
            measure: "wealth".into(),
            denominations: vec![QuantityDenomination {
                name: "silver".into(),
                per_base: 10,
            }],
            bands: Vec::new(),
            max_band_jump: None,
        },
    })
    .await
    .unwrap();
    let mara = seed_character(&svc, &proj.project_id, "Mara Voss").await;
    let _soren = seed_character(&svc, &proj.project_id, "Soren Pike").await;

    for (scene_order, full_text) in [
        (
            1,
            "Soren Pike left.\n\nMara Voss checked the stall. She paid twelve silver for the blade.",
        ),
        (
            2,
            "Mara Voss and Soren Pike paid twelve silver for the chart.",
        ),
    ] {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order,
            full_text: full_text.into(),
            summary: "market".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    }

    svc.commit_quantity_state(CommitQuantityStateInput {
        project_id: proj.project_id.clone(),
        subject_table: "character".into(),
        subject_id: mara.clone(),
        measure: "wealth".into(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
        scene_id: None,
        state: QuantityState {
            amount: Some(1.0),
            unit: Some("silver".into()),
            ..Default::default()
        },
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(spindle_core::models::CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: spindle_core::models::ConsistencyScopeInput::full(),
            checks: vec!["affordability".into()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    let affordability: Vec<_> = out
        .issues
        .iter()
        .filter(|issue| issue.check_type == "affordability")
        .collect();
    assert!(
        affordability
            .iter()
            .any(|issue| issue.severity == "warning"
                && issue.entity_ids.iter().any(|id| id == &mara)),
        "resolved pronoun spend should warn: {affordability:?}"
    );
    assert!(
        affordability.iter().any(|issue| issue.severity == "info"),
        "two named spenders must stay advisory: {affordability:?}"
    );
}

#[tokio::test]
async fn affordability_first_person_protagonist_warns() {
    use spindle_core::models::{
        CommitQuantityStateInput, ContentRating, QuantityDenomination, QuantityScheme,
        QuantityState, SaveSceneDraftInput, SetProjectQuantitySchemeInput, UpdateCharacterInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "affordability-pov".into(),
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
    svc.set_project_quantity_scheme(SetProjectQuantitySchemeInput {
        project_id: proj.project_id.clone(),
        scheme: QuantityScheme {
            measure: "wealth".into(),
            denominations: vec![QuantityDenomination {
                name: "silver".into(),
                per_base: 10,
            }],
            bands: Vec::new(),
            max_band_jump: None,
        },
    })
    .await
    .unwrap();
    let tamsin = seed_character(&svc, &proj.project_id, "Tamsin Okoro").await;
    let _mara = seed_character(&svc, &proj.project_id, "Mara Voss").await;
    svc.update_character(UpdateCharacterInput {
        character_id: tamsin.clone(),
        name: None,
        summary: None,
        role: Some("protagonist".into()),
        realm: None,
        appearance: None,
        aliases: None,
        allow_rename: None,
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "I paid Mara twelve silver ✦ Élodie’s note was short.".into(),
        summary: "market".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.commit_quantity_state(CommitQuantityStateInput {
        project_id: proj.project_id.clone(),
        subject_table: "character".into(),
        subject_id: tamsin.clone(),
        measure: "wealth".into(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
        scene_id: None,
        state: QuantityState {
            amount: Some(1.0),
            unit: Some("silver".into()),
            ..Default::default()
        },
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(spindle_core::models::CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: spindle_core::models::ConsistencyScopeInput::full(),
            checks: vec!["affordability".into()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        out.issues
            .iter()
            .any(|issue| issue.check_type == "affordability"
                && issue.severity == "warning"
                && issue.entity_ids.iter().any(|id| id == &tamsin)),
        "first-person protagonist spend should warn: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn affordability_ignores_balances_negated_purchases_and_recipients() {
    use spindle_core::models::{
        CommitQuantityStateInput, ContentRating, QuantityDenomination, QuantityScheme,
        QuantityState, SaveSceneDraftInput, SetProjectQuantitySchemeInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "affordability-negatives".into(),
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
    svc.set_project_quantity_scheme(SetProjectQuantitySchemeInput {
        project_id: proj.project_id.clone(),
        scheme: QuantityScheme {
            measure: "wealth".into(),
            denominations: vec![QuantityDenomination {
                name: "silver".into(),
                per_base: 1,
            }],
            bands: Vec::new(),
            max_band_jump: None,
        },
    })
    .await
    .unwrap();
    let mara = seed_character(&svc, &proj.project_id, "Mara Voss").await;

    for (scene_order, full_text) in [
        (
            1,
            "Mara Voss paid twelve silver for the blade and had ten silver left.",
        ),
        (2, "Mara Voss did not buy the thirty silver coat."),
        (3, "The merchant paid Mara Voss twelve silver."),
        (4, "Mara Voss was paid twelve silver by the merchant."),
    ] {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order,
            full_text: full_text.into(),
            summary: "market".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    }
    svc.commit_quantity_state(CommitQuantityStateInput {
        project_id: proj.project_id.clone(),
        subject_table: "character".into(),
        subject_id: mara.clone(),
        measure: "wealth".into(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
        scene_id: None,
        state: QuantityState {
            amount: Some(15.0),
            unit: Some("silver".into()),
            ..Default::default()
        },
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(spindle_core::models::CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: spindle_core::models::ConsistencyScopeInput::full(),
            checks: vec!["affordability".into()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        !out.issues
            .iter()
            .any(|issue| issue.check_type == "affordability" && issue.severity == "warning"),
        "balance, negation, and receipt must not warn: {:?}",
        out.issues
    );
}
