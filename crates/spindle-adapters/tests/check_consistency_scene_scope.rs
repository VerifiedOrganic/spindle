//! P2.1 — scene-scoped verification. `check_consistency` gains an additive
//! `scene_order` narrowing on `ConsistencyScopeInput` that pins the scoped
//! scene set to exactly one scene inside an already single-chapter scope. The
//! deterministic, per-scene checks then attribute their findings to only the
//! scoped scene; chapter/range-level checks still run over the single-chapter
//! window unchanged.
//!
//! These are the red-first tests for V1–V4:
//!  1. scope pin — a pre-existing (no `scene_order`) scope JSON deserializes and
//!     a chapter-scoped run is byte-identical.
//!  2. scene narrowing — two scenes in one chapter, distinct violations;
//!     `scene_order=2` yields only scene 2's finding.
//!  3. validation — `scene_order` with a multi-chapter range is an input error.
//!  4. range-level unchanged — a chapter-level check still runs.
//!  5. `SCENE_VERIFY_CHECKS` — every name is accepted; no deep/model calls.
//!  6. `secret_leak` scene-scoped — a leak on the scoped scene fires; a leak on
//!     the other scene in the same chapter does not.

use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    CharacterEmotionalProfileData, CharacterVoiceProfileData, CheckConsistencyInput,
    ConsistencyScopeInput, ContentRating, CreateCharacterInput, CreateLocationInput,
    CreateProjectInput, ReaderContract, RecordKnowledgeInput, RegisterCanonicalFactInput,
    SCENE_VERIFY_CHECKS, SaveSceneDraftInput, SecrecyScope, StoryPlacement, WorldStateInput,
};
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);
    (tmp, SqliteSpindleService::new(repo))
}

/// Local-only router: no `review`/`draft` routes are configured, so any model
/// path a deep tier attempted would either error or emit a skip finding. With
/// `deep_check = false` no such path is reachable — the V4 no-model seam.
async fn fresh_service_local() -> (TempDir, SqliteSpindleService) {
    use spindle_adapters::ai::ModelRouter;
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::with_model_router(pool, data_dir, ModelRouter::local_only());
    (tmp, SqliteSpindleService::new(repo))
}

fn make_character_input(project_id: &str, name: &str) -> CreateCharacterInput {
    CreateCharacterInput {
        aliases: Vec::new(),
        project_id: project_id.to_string(),
        name: name.to_string(),
        summary: format!("{name} in play."),
        role: "supporting".into(),
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
    }
}

/// Scene 1 (order 1) carries a `knowledge_timing` violation: Mara references a
/// fact she does not learn until a later chapter. Scene 2 (order 2) carries a
/// `tone_consistency` violation: its tone falls outside the declared boundary.
/// Both live in book 1, chapter 1.
async fn two_scene_fixture() -> (TempDir, SqliteSpindleService, String, String) {
    let (tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "SceneScope".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                // A tone boundary the scene-2 tone will violate.
                boundaries: vec!["tone:calm".into()],
            },
        })
        .await
        .unwrap();
    let project_id = proj.project_id;

    let mara = svc
        .create_character(make_character_input(&project_id, "Mara"))
        .await
        .unwrap()
        .character_id;

    // Scene 1: Mara references a fact she doesn't learn until chapter 3.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Mara already knew the gate would fall by winter.".into(),
        summary: "scene one".into(),
        content_rating: ContentRating::General,
        tone: Some("calm".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // Scene 2: same chapter, a tone outside the declared boundary.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 2,
        full_text: "Aldric paced the empty courtyard.".into(),
        summary: "scene two".into(),
        content_rating: ContentRating::General,
        tone: Some("frantic".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // Mara learns the fact only at chapter 3 → scene-1 reference is premature.
    svc.record_knowledge(RecordKnowledgeInput {
        project_id: project_id.clone(),
        branch_id: None,
        character_id: mara.clone(),
        fact: "the gate would fall by winter".into(),
        source_summary: "a scout's warning".into(),
        learned_at: Some(StoryPlacement {
            book_number: 1,
            chapter_number: 3,
            scene_order: Some(1),
            note: None,
        }),
        confidence: None,
        tags: Vec::new(),
        reader_visible: true,
        secret_of_fact_id: None,
    })
    .await
    .unwrap();

    (tmp, svc, project_id, mara)
}

// ── Test 2: scene narrowing ─────────────────────────────────────────────────

#[tokio::test]
async fn scene_order_narrows_findings_to_one_scene() {
    let (_tmp, svc, project_id, _mara) = two_scene_fixture().await;

    // Whole-chapter scope sees BOTH violations.
    let chapter_out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::chapter_range(1, 1, 1, 1),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        chapter_out
            .issues
            .iter()
            .any(|i| i.check_type == "knowledge_timing"),
        "chapter scope must see scene 1's knowledge_timing violation"
    );
    assert!(
        chapter_out
            .issues
            .iter()
            .any(|i| i.check_type == "tone_consistency"
                && i.message.to_lowercase().contains("frantic")),
        "chapter scope must see scene 2's tone violation"
    );

    // Scene-2-narrowed scope sees ONLY scene 2's tone violation.
    let scene_two = ConsistencyScopeInput {
        scene_order: Some(2),
        ..ConsistencyScopeInput::chapter_range(1, 1, 1, 1)
    };
    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: scene_two,
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        out.issues
            .iter()
            .any(|i| i.check_type == "tone_consistency"
                && i.message.to_lowercase().contains("frantic")),
        "scene_order=2 must still surface scene 2's tone violation: {:?}",
        out.issues
            .iter()
            .map(|i| (i.check_type.clone(), i.message.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "knowledge_timing"),
        "scene_order=2 must NOT surface scene 1's knowledge_timing violation: {:?}",
        out.issues
            .iter()
            .map(|i| (i.check_type.clone(), i.message.clone()))
            .collect::<Vec<_>>()
    );
}

// ── Test 3: validation ──────────────────────────────────────────────────────

#[tokio::test]
async fn scene_order_with_multi_chapter_range_is_input_error() {
    let (_tmp, svc, project_id, _mara) = two_scene_fixture().await;
    let bad = ConsistencyScopeInput {
        scene_order: Some(1),
        // A multi-chapter range: start (1,1) != end (1,2).
        ..ConsistencyScopeInput::chapter_range(1, 1, 1, 2)
    };
    let err = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: bad,
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .expect_err("scene_order over a multi-chapter range must be rejected");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("scene_order") && msg.contains("single") && msg.contains("chapter"),
        "the error must name the single-chapter constraint: {msg}"
    );
}

// ── Test 1: scope pin (byte-identical chapter run + serde default) ───────────

#[tokio::test]
async fn preexisting_scope_json_without_scene_order_is_unchanged() {
    // A scope payload authored before this field existed deserializes with
    // `scene_order = None`, and a chapter-scoped run behaves exactly as today.
    let literal = r#"{
        "scope_type": "chapter_range",
        "book_number": null,
        "start_book_number": 1,
        "start_chapter_number": 1,
        "end_book_number": 1,
        "end_chapter_number": 1
    }"#;
    let scope: ConsistencyScopeInput = serde_json::from_str(literal).unwrap();
    assert_eq!(
        scope.scene_order, None,
        "legacy JSON pins scene_order = None"
    );

    let (_tmp, svc, project_id, _mara) = two_scene_fixture().await;
    let from_literal = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope,
            checks: vec!["tone_consistency".into(), "knowledge_timing".into()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    let from_builder = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::chapter_range(1, 1, 1, 1),
            checks: vec!["tone_consistency".into(), "knowledge_timing".into()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&from_literal).unwrap(),
        serde_json::to_value(&from_builder).unwrap(),
        "the legacy-JSON scope must be byte-identical to the builder scope"
    );
}

// ── Test 4: chapter/range-level checks still run under a scene scope ─────────

#[tokio::test]
async fn scene_scope_still_runs_chapter_level_checks() {
    // `content_boundary_compliance` attributes to scenes; but a chapter-level
    // check like `world_rule_compliance` keys to rules, not the scoped scene.
    // Under a scene scope the run must still execute it (not crash) and any
    // scene-scoped check must attribute correctly. We assert the scoped run
    // succeeds and surfaces the scene-2 tone finding while the range-level
    // machinery ran without panicking.
    let (_tmp, svc, project_id, _mara) = two_scene_fixture().await;
    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput {
                scene_order: Some(2),
                ..ConsistencyScopeInput::chapter_range(1, 1, 1, 1)
            },
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .expect("a scene-scoped run must complete without crashing");
    // The full check battery ran; the scene-attributed finding is present.
    assert!(
        out.issues
            .iter()
            .any(|i| i.check_type == "tone_consistency"),
        "the scene-attributed tone finding must appear under a scene scope"
    );
}

// ── Test 5: SCENE_VERIFY_CHECKS accepted; no deep/model calls ────────────────

#[tokio::test]
async fn scene_verify_checks_run_deterministically_with_no_model_calls() {
    // fresh_service_local: local-only router, no `review` route. If any deep
    // tier were reached it would produce a skip finding or error; with
    // deep_check = false none is reachable. Requesting exactly
    // SCENE_VERIFY_CHECKS on a violating scene fires the expected deterministic
    // findings and returns no skip findings.
    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "VerifyList".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: vec!["tone:calm".into()],
            },
        })
        .await
        .unwrap();
    let project_id = proj.project_id;
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Aldric paced the empty courtyard.".into(),
        summary: "scene".into(),
        content_rating: ContentRating::General,
        tone: Some("frantic".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let checks: Vec<String> = SCENE_VERIFY_CHECKS.iter().map(|c| c.to_string()).collect();
    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput {
                scene_order: Some(1),
                ..ConsistencyScopeInput::chapter_range(1, 1, 1, 1)
            },
            checks,
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .expect("scene-scoped SCENE_VERIFY_CHECKS run must succeed");

    // The deterministic tone check fired.
    assert!(
        out.issues
            .iter()
            .any(|i| i.check_type == "tone_consistency"),
        "SCENE_VERIFY_CHECKS must fire the deterministic tone check: {:?}",
        out.issues
            .iter()
            .map(|i| i.check_type.clone())
            .collect::<Vec<_>>()
    );
    // No skip finding leaked (no model path was attempted).
    assert!(
        !out.issues.iter().any(|i| {
            let m = i.message.to_lowercase();
            m.contains("skip") || m.contains("route") && m.contains("not covered")
        }),
        "no pass may have been skipped for a missing route: {:?}",
        out.issues
            .iter()
            .map(|i| i.message.clone())
            .collect::<Vec<_>>()
    );
}

// ── Test 6: secret_leak scene-scoped ────────────────────────────────────────

#[tokio::test]
async fn secret_leak_scene_scope_fires_only_on_scoped_scene() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "LeakScope".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Secrets stay secret.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let project_id = proj.project_id;
    let mara = svc
        .create_character(make_character_input(&project_id, "Mara"))
        .await
        .unwrap()
        .character_id;
    let bran = svc
        .create_character(make_character_input(&project_id, "Bran"))
        .await
        .unwrap()
        .character_id;
    let _location = svc
        .create_location(CreateLocationInput {
            project_id: project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A wall.".into(),
            initial_state: WorldStateInput {
                controlling_faction: None,
                status: Some("tense".into()),
                prosperity: None,
                stability: None,
                threat_level: None,
                sensory_details: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Seed scene (ch 1 sc 1) anchors the secret; Mara is the sole holder.
    let seed = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara kept her counsel.".into(),
            summary: "seed".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    svc.register_canonical_fact(RegisterCanonicalFactInput {
        project_id: project_id.clone(),
        scene_id: Some(seed.scene_id.clone()),
        book_number: 1,
        chapter_number: 1,
        fact_type: None,
        key: None,
        value: None,
        context: None,
        subject_table: Some("character".into()),
        subject_id: Some(mara.clone()),
        predicate: Some("reincarnation".into()),
        value_kind: Some("string".into()),
        value_text: Some("reincarnated".into()),
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
            holder_ids: vec![mara.clone()],
            concealment_note: None,
        }),
    })
    .await
    .unwrap();

    // Two leak scenes in ch 2: sc 1 (Bran leaks) and sc 2 (Bran leaks again).
    let _ = svc
        .create_chapter(spindle_core::models::CreateChapterInput {
            project_id: project_id.clone(),
            book_id: None,
            book_number: Some(1),
            chapter_number: Some(2),
            title: None,
        })
        .await;
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 2,
        chapter_id: None,
        scene_order: 1,
        full_text: "\"You are reincarnated, I know it,\" Bran said, watching Mara.".into(),
        summary: "leak one".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 2,
        chapter_id: None,
        scene_order: 2,
        full_text: "\"So you were reincarnated after all,\" Bran said again.".into(),
        summary: "leak two".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // Scope to ch 2 scene 1 only: the sc-1 leak fires, the sc-2 leak does not.
    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput {
                scene_order: Some(1),
                ..ConsistencyScopeInput::chapter_range(1, 2, 1, 2)
            },
            checks: vec!["secret_leak".into()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    let leaks: Vec<_> = out
        .issues
        .iter()
        .filter(|i| i.check_type == "secret_leak")
        .collect();
    assert_eq!(
        leaks.len(),
        1,
        "exactly the scoped scene's leak fires: {:?}",
        leaks.iter().map(|i| i.message.clone()).collect::<Vec<_>>()
    );
    assert!(
        leaks[0].entity_ids.contains(&bran),
        "the leaking character on the scoped scene is named: {:?}",
        leaks[0].entity_ids
    );
    assert!(
        leaks[0].message.to_lowercase().contains("bran"),
        "the scoped-scene finding names the leaker: {}",
        leaks[0].message
    );
}
