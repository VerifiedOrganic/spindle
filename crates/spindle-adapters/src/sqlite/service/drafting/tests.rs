use super::*;

/// `chapter_id` is documented as sufficient placement for
/// `save_scene_draft`: Spindle must resolve book/chapter from the existing
/// chapter instead of defaulting the numbers to 0 (which corrupted receipt
/// claims in the live run: `book 0 not found for project`).
#[tokio::test]
async fn save_scene_draft_resolves_placement_from_chapter_id() {
    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "chapter-id-placement".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "p".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let chapter = svc
        .create_chapter(spindle_core::models::CreateChapterInput {
            project_id: project.project_id.clone(),
            book_number: Some(2),
            book_id: None,
            chapter_number: Some(1),
            title: Some("The Long Road".to_string()),
        })
        .await
        .unwrap();

    let out = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            chapter_id: Some(chapter.chapter_id.clone()),
            scene_order: 1,
            full_text: "The road stretched past the last watchtower.".to_string(),
            summary: "leaving the city".to_string(),
            content_rating: spindle_core::models::ContentRating::General,
            ..Default::default()
        })
        .await
        .expect("chapter_id alone must be sufficient placement");

    let branch_id = svc
        .repository
        .active_branch_id_public(&project.project_id)
        .await
        .unwrap();
    let resolved = svc
        .repository
        .find_scene_by_natural_key(&project.project_id, &branch_id, 2, 1, 1)
        .await
        .unwrap()
        .expect("the scene must land in the chapter named by chapter_id");
    assert_eq!(resolved.id, out.scene_id);
    assert_eq!(resolved.book_number, 2);
    assert_eq!(resolved.chapter_number, 1);
}

/// A call with NEITHER chapter_id NOR usable numbers must fail with a
/// clear validation error — silently falling back to book/chapter 0 is
/// what burned a receipt in the live run.
#[tokio::test]
async fn save_scene_draft_rejects_call_with_no_placement_at_all() {
    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "no-placement".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "p".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let err = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id,
            scene_order: 1,
            full_text: "Some prose.".to_string(),
            summary: "s".to_string(),
            content_rating: spindle_core::models::ContentRating::General,
            ..Default::default()
        })
        .await
        .expect_err("missing placement must be a validation error");
    assert!(
        err.to_string()
            .contains("requires chapter_id or both book_number and chapter_number"),
        "expected an explicit placement validation error, got: {err}"
    );
}

/// Explicit numbers that contradict the chapter named by `chapter_id`
/// must be rejected loudly, mirroring `get_scene_context`'s reconcile.
#[tokio::test]
async fn save_scene_draft_rejects_chapter_id_conflicting_with_numbers() {
    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "chapter-id-conflict".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "p".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let chapter = svc
        .create_chapter(spindle_core::models::CreateChapterInput {
            project_id: project.project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(2),
            title: None,
        })
        .await
        .unwrap();

    let err = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id,
            book_number: 2,
            chapter_number: 2,
            chapter_id: Some(chapter.chapter_id),
            scene_order: 1,
            full_text: "Some prose.".to_string(),
            summary: "s".to_string(),
            content_rating: spindle_core::models::ContentRating::General,
            ..Default::default()
        })
        .await
        .expect_err("contradictory placement must be rejected");
    assert!(
        err.to_string().contains("does not match book_number"),
        "expected the mismatch error, got: {err}"
    );
}

/// Bug-5 guard: the revise loop re-saves through `save_scene_draft`. A
/// second explicit save (the revised prose, referencing a fresh receipt)
/// must persist the REVISED full_text, not any receipt output.
#[tokio::test]
async fn save_scene_draft_re_save_persists_revised_full_text() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "explicit-revise-resave".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "receipt-gated drafting".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let seed_receipt = async |svc: &SqliteSpindleService, output: &str| {
        let receipt = svc
            .register_generation_receipt("draft", Some("explicit"), "explicit-agent", output)
            .await
            .unwrap();
        let mut receipts = svc.generation_receipts.write().unwrap();
        receipts
            .get_mut(&receipt.id)
            .expect("seeded receipt")
            .explicit_capable_agent = true;
        receipt.id
    };

    let first_receipt = seed_receipt(&svc, "first receipt narration").await;
    let first_text = "First-pass prose for the scene.";
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: first_text.to_string(),
        summary: "First pass.".to_string(),
        content_rating: spindle_core::models::ContentRating::Explicit,
        tone: None,
        generation_id: Some(first_receipt),
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let revised_receipt = seed_receipt(&svc, "revised receipt narration").await;
    let revised_text = "Revised prose after a surgical edit pass.";
    let out = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id,
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: revised_text.to_string(),
            summary: "Revised pass.".to_string(),
            content_rating: spindle_core::models::ContentRating::Explicit,
            tone: None,
            generation_id: Some(revised_receipt),
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        out.status, "updated",
        "second save to same scene is an update"
    );
    let scene = svc.repository.get_scene(&out.scene_id).await.unwrap();
    assert_eq!(
        scene.full_text, revised_text,
        "re-save must persist the revised full_text"
    );
    assert_eq!(out.chars_added, revised_text.len());
}

/// Draft-time surfacing: an unmarked morning→night jump appears in
/// `save_scene_draft` output as advisory temporal findings.
#[tokio::test]
async fn save_scene_draft_surfaces_temporal_findings() {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "draft-temporal".into(),
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
    let out = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: TELEPORT_PROSE.into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        out.temporal_findings
            .iter()
            .any(|i| i.check_type == "temporal_coherence"),
        "save_scene_draft must surface temporal findings on an unmarked jump: {:?}",
        out.temporal_findings
    );
    assert!(
        out.temporal_findings
            .iter()
            .all(|i| i.severity == "warning"),
        "temporal findings are advisory warnings only: {:?}",
        out.temporal_findings
    );
}

/// Draft-time precision: clean (bridged) prose produces no temporal findings.
#[tokio::test]
async fn save_scene_draft_clean_prose_has_no_temporal_findings() {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "draft-clean".into(),
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
    let out = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "He woke in the grey morning and ate his bread. Hours later, the \
                            midnight harbour lay black and still around him."
                .into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        out.temporal_findings.is_empty(),
        "bridged prose must produce no temporal findings: {:?}",
        out.temporal_findings
    );
}

/// Style learning from operator edits (evolution §3.9, P5.4). Capture-side
/// tests: an operator re-save over an agent draft on an opted-in project
/// stages a candidate; the many no-capture cases stay empty.
mod style_learning_capture {
    use super::*;

    /// Create a project, save an initial scene, then stamp it with agent
    /// provenance (`draft_origin = "agent:*"`) — the DB signal capture keys
    /// on. Returns (project_id, branch_id, scene_id, agent prose).
    async fn agent_drafted_scene(
        svc: &SqliteSpindleService,
        name: &str,
        agent_prose: &str,
    ) -> (String, String, String) {
        let project = svc
            .create_project(CreateProjectInput {
                name: name.to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "clean continuity".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let saved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: 1,
                full_text: agent_prose.to_string(),
                summary: "opening".to_string(),
                content_rating: spindle_core::models::ContentRating::General,
                ..Default::default()
            })
            .await
            .unwrap();
        // Seed the agent-provenance signal the harness agent-draft path
        // records (for explicit drafts) — the only DB signal capture keys on.
        svc.repository
            .update_scene_draft_origin(&saved.scene_id, "agent:test-agent")
            .await
            .unwrap();
        let branch_id = svc
            .repository
            .active_branch_id_public(&project.project_id)
            .await
            .unwrap();
        (project.project_id, branch_id, saved.scene_id)
    }

    async fn enable_style_learning(svc: &SqliteSpindleService, project_id: &str) {
        svc.update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "project".to_string(),
            entity_id: project_id.to_string(),
            changes: serde_json::json!({ "style_learning": 1 }),
        })
        .await
        .unwrap();
    }

    async fn operator_resave(svc: &SqliteSpindleService, project_id: &str, prose: &str) {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: prose.to_string(),
            summary: "opening".to_string(),
            content_rating: spindle_core::models::ContentRating::General,
            ..Default::default()
        })
        .await
        .unwrap();
    }

    // Test 1: agent-drafted scene re-saved by operator with changed prose,
    // project opted in → one pending candidate with both texts + rating;
    // re-edit → replaced (still one, latest edit).
    #[tokio::test]
    async fn operator_edit_over_agent_draft_captures_pending_candidate() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project_id, branch_id, _scene_id) =
            agent_drafted_scene(&svc, "learn", "The agent wrote this line plainly.").await;
        enable_style_learning(&svc, &project_id).await;

        operator_resave(
            &svc,
            &project_id,
            "The operator rewrote this line with flair.",
        )
        .await;

        let pending = svc
            .repository
            .list_style_edit_candidates(&project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert_eq!(pending.len(), 1, "one pending candidate captured");
        let c = &pending[0];
        assert_eq!(c.agent_draft, "The agent wrote this line plainly.");
        assert_eq!(
            c.operator_edit,
            "The operator rewrote this line with flair."
        );
        assert_eq!(c.content_rating, "general");
        assert_eq!(c.status, "pending");

        // Re-edit: the SECOND operator save over the same agent draft REPLACES
        // the pending candidate (one per scene — the latest edit is the signal).
        operator_resave(
            &svc,
            &project_id,
            "The operator revised it once more, sharper.",
        )
        .await;
        let pending = svc
            .repository
            .list_style_edit_candidates(&project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert_eq!(pending.len(), 1, "still one candidate after re-edit");
        assert_eq!(
            pending[0].agent_draft, "The agent wrote this line plainly.",
            "agent draft (contrast) is preserved across re-edits"
        );
        assert_eq!(
            pending[0].operator_edit, "The operator revised it once more, sharper.",
            "operator edit is the latest prose"
        );
    }

    // Test 2a: opt-out project → no capture.
    #[tokio::test]
    async fn host_assistant_rewrites_are_not_human_feedback() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project_id, branch_id, scene_id) =
            agent_drafted_scene(&svc, "host-ai", "Original draft.").await;
        enable_style_learning(&svc, &project_id).await;
        let input = SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: "The host assistant revised this prose.".into(),
            summary: "opening".into(),
            authorship: spindle_core::models::DraftAuthorship::Assistant,
            ..Default::default()
        };
        let saved = svc.save_scene_draft(input).await.unwrap();
        assert_eq!(saved.draft_origin, "assistant:host");
        assert!(
            svc.repository
                .list_style_edit_candidates(&project_id, &branch_id, Some("pending"))
                .await
                .unwrap()
                .is_empty()
        );
        operator_resave(&svc, &project_id, "The author chose a quieter sentence.").await;
        let pending = svc
            .repository
            .list_style_edit_candidates(&project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].agent_draft,
            "The host assistant revised this prose."
        );
        assert_eq!(
            pending[0].operator_edit,
            "The author chose a quieter sentence."
        );
        assert_eq!(
            svc.repository
                .get_scene(&scene_id)
                .await
                .unwrap()
                .draft_origin
                .as_deref(),
            Some("operator")
        );
    }

    #[tokio::test]
    async fn no_capture_when_project_opted_out() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project_id, branch_id, _scene_id) =
            agent_drafted_scene(&svc, "optout", "Agent prose one.").await;
        // Deliberately do NOT enable style_learning.
        operator_resave(&svc, &project_id, "Operator prose two.").await;
        let pending = svc
            .repository
            .list_style_edit_candidates(&project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert!(pending.is_empty(), "opt-out project captures nothing");
    }

    // Test 2b: unchanged prose (byte-identical after trim) → no capture.
    #[tokio::test]
    async fn no_capture_when_prose_unchanged() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project_id, branch_id, _scene_id) =
            agent_drafted_scene(&svc, "unchanged", "Identical prose.").await;
        enable_style_learning(&svc, &project_id).await;
        // Re-save the same prose with surrounding whitespace (trim-equal).
        operator_resave(&svc, &project_id, "  Identical prose.  ").await;
        let pending = svc
            .repository
            .list_style_edit_candidates(&project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert!(pending.is_empty(), "trim-equal prose is not an edit");
    }

    // Test 2c: operator-over-operator → the SECOND operator save does not
    // create a NEW candidate (only replaces the pending one from the agent
    // draft). A fresh operator save with no prior agent draft captures nothing.
    #[tokio::test]
    async fn no_capture_operator_over_operator_without_agent_draft() {
        let (_tmp, svc) = fresh_service_local().await;
        // Create a project + scene with NO agent provenance (plain host save).
        let project = svc
            .create_project(CreateProjectInput {
                name: "hostonly".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "p".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        enable_style_learning(&svc, &project.project_id).await;
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: "Operator wrote the first draft.".to_string(),
            summary: "s".to_string(),
            content_rating: spindle_core::models::ContentRating::General,
            ..Default::default()
        })
        .await
        .unwrap();
        // Second operator save over an operator draft (no agent provenance).
        operator_resave(
            &svc,
            &project.project_id,
            "Operator edited the operator draft.",
        )
        .await;
        let branch_id = svc
            .repository
            .active_branch_id_public(&project.project_id)
            .await
            .unwrap();
        let pending = svc
            .repository
            .list_style_edit_candidates(&project.project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert!(
            pending.is_empty(),
            "operator-over-operator with no agent draft captures nothing"
        );
    }

    // Test 2d: agent-over-agent (revise-loop re-save) → no capture. Both
    // saves go through the explicit-agent generation_id path, which stamps
    // draft_origin agent AND marks the save as an agent save
    // (is_agent_save = true) — capture is short-circuited.
    #[tokio::test]
    async fn no_capture_agent_over_agent_via_generation_id() {
        use spindle_core::models::ConfigureAgentsInput;
        let (tmp, svc) = fresh_service_local().await;

        // Configure an explicit-capable draft agent so the explicit save path
        // (verified_explicit_save_receipt) accepts a seeded receipt. No CLI
        // command is invoked — the receipt output is authoritative.
        let config_path = tmp.path().join("agents.toml");
        std::fs::write(
            &config_path,
            r#"
[health_check]
enabled = false

[[agents]]
id = "draft-agent"
name = "Draft Agent"
provider = "openai-compatible"
endpoint = "http://localhost:1/v1"
model = "drafter"
ratings = ["general", "explicit"]

[[routing]]
route = "draft"
agent = "draft-agent"
"#,
        )
        .unwrap();
        svc.configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.to_string_lossy().to_string()),
        })
        .unwrap();

        let project = svc
            .create_project(CreateProjectInput {
                name: "aoa".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "p".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        enable_style_learning(&svc, &project.project_id).await;

        // First explicit agent draft via a seeded generation receipt.
        let r1 = svc
            .register_generation_receipt(
                "draft",
                Some("explicit"),
                "draft-agent",
                "The agent drafted the gate scene, first pass.",
            )
            .await
            .unwrap();
        let saved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: 1,
                full_text: "ignored — server output is authoritative".to_string(),
                summary: "s".to_string(),
                content_rating: spindle_core::models::ContentRating::Explicit,
                generation_id: Some(r1.id.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            saved.draft_origin.starts_with("agent:"),
            "first save carries agent provenance"
        );

        // Second explicit agent draft (revise loop) via another receipt →
        // agent-over-agent. is_agent_save = true short-circuits capture.
        let r2 = svc
            .register_generation_receipt(
                "draft",
                Some("explicit"),
                "draft-agent",
                "The agent revised the gate scene, second pass, sharper.",
            )
            .await
            .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: "ignored — server output is authoritative".to_string(),
            summary: "s".to_string(),
            content_rating: spindle_core::models::ContentRating::Explicit,
            generation_id: Some(r2.id.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

        let branch_id = svc
            .repository
            .active_branch_id_public(&project.project_id)
            .await
            .unwrap();
        let pending = svc
            .repository
            .list_style_edit_candidates(&project.project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert!(
            pending.is_empty(),
            "agent-over-agent re-save must not capture a style candidate"
        );
    }

    // Test 6: a scene > 60k chars skips capture with a warn (degenerate guard).
    #[tokio::test]
    async fn oversized_scene_skips_capture() {
        let (_tmp, svc) = fresh_service_local().await;
        let big_agent = "a".repeat(61_000);
        let (project_id, branch_id, _scene_id) = agent_drafted_scene(&svc, "big", &big_agent).await;
        enable_style_learning(&svc, &project_id).await;
        let big_operator = "b".repeat(61_000);
        operator_resave(&svc, &project_id, &big_operator).await;
        let pending = svc
            .repository
            .list_style_edit_candidates(&project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert!(pending.is_empty(), "oversized scene skips capture");
    }

    // Test 5: pre-V0031 behavior — a freshly created project has NULL
    // style_learning = disabled (no capture without an explicit opt-in).
    #[tokio::test]
    async fn new_project_style_learning_null_is_disabled() {
        let (_tmp, svc) = fresh_service_local().await;
        let enabled = svc
            .repository
            .project_style_learning_enabled_public("project:does-not-exist-yet")
            .await;
        // A missing project is an error, not a silent enable.
        assert!(enabled.is_err() || !enabled.unwrap());
        let (project_id, _branch, _scene) =
            agent_drafted_scene(&svc, "nullflag", "Agent prose.").await;
        let enabled = svc
            .repository
            .project_style_learning_enabled_public(&project_id)
            .await
            .unwrap();
        assert!(!enabled, "NULL style_learning reads as disabled");
    }

    /// Configure a draft agent that covers `general` (the non-explicit
    /// majority path) so a seeded `draft`-route receipt at that rating is a
    /// valid agent-draft provenance signal. Mirrors the run's agent-draft
    /// seam: the harness resolves the draft route, generates through
    /// `test_agent`/`continue_generation` (which register the receipt), then
    /// calls `save_scene_draft` with that `generation_id`.
    async fn configure_general_draft_agent(svc: &SqliteSpindleService, tmp: &TempDir) {
        use spindle_core::models::ConfigureAgentsInput;
        let config_path = tmp.path().join("agents.toml");
        std::fs::write(
            &config_path,
            r#"
[health_check]
enabled = false

[[agents]]
id = "draft-agent"
name = "Draft Agent"
provider = "openai-compatible"
endpoint = "http://localhost:1/v1"
model = "drafter"
ratings = ["general"]

[[routing]]
route = "draft"
agent = "draft-agent"
"#,
        )
        .unwrap();
        svc.configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.to_string_lossy().to_string()),
        })
        .unwrap();
    }

    /// Test 1 (provenance): a NON-explicit scene saved through the
    /// agent-draft seam (a valid `draft`-route `generation_id` at `general`
    /// rating) must stamp `draft_origin = "agent:<agent_id>"`. This is the
    /// flagged gap — before uniform stamping, non-explicit agent drafts left
    /// `draft_origin` NULL.
    #[tokio::test]
    async fn non_explicit_agent_draft_stamps_agent_provenance() {
        let (tmp, svc) = fresh_service_local().await;
        configure_general_draft_agent(&svc, &tmp).await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "nonexpl-agent".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "p".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        // Seed a general-rated draft receipt (what the agent path registers
        // for a non-explicit scene).
        let receipt = svc
            .register_generation_receipt(
                "draft",
                Some("general"),
                "draft-agent",
                "The agent drafted this general-rated watch scene.",
            )
            .await
            .unwrap();
        let saved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: 1,
                full_text: "The agent drafted this general-rated watch scene.".to_string(),
                summary: "s".to_string(),
                content_rating: spindle_core::models::ContentRating::General,
                generation_id: Some(receipt.id.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        let scene = svc.repository.get_scene(&saved.scene_id).await.unwrap();
        assert_eq!(
            scene.draft_origin.as_deref(),
            Some("agent:draft-agent"),
            "non-explicit agent draft must stamp agent provenance, got {:?}",
            scene.draft_origin
        );
        assert!(
            saved.draft_origin.starts_with("agent:"),
            "output draft_origin must carry agent provenance, got {:?}",
            saved.draft_origin
        );
    }

    /// Test 2 (capture end-to-end, non-explicit): opt-in project, a
    /// non-explicit agent draft (via `generation_id`), then an operator
    /// re-save with changed prose → one pending candidate. Before uniform
    /// stamping this captured nothing (NULL provenance).
    #[tokio::test]
    async fn operator_edit_over_non_explicit_agent_draft_captures() {
        let (tmp, svc) = fresh_service_local().await;
        configure_general_draft_agent(&svc, &tmp).await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "nonexpl-capture".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "p".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        enable_style_learning(&svc, &project.project_id).await;

        let receipt = svc
            .register_generation_receipt(
                "draft",
                Some("general"),
                "draft-agent",
                "The agent wrote this line plainly.",
            )
            .await
            .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: "The agent wrote this line plainly.".to_string(),
            summary: "opening".to_string(),
            content_rating: spindle_core::models::ContentRating::General,
            generation_id: Some(receipt.id.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

        // Operator re-save (no generation_id → host/operator origin) with
        // changed prose.
        operator_resave(
            &svc,
            &project.project_id,
            "The operator rewrote this line with flair.",
        )
        .await;

        let branch_id = svc
            .repository
            .active_branch_id_public(&project.project_id)
            .await
            .unwrap();
        let pending = svc
            .repository
            .list_style_edit_candidates(&project.project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert_eq!(
            pending.len(),
            1,
            "operator edit over a non-explicit agent draft must capture one candidate"
        );
        assert_eq!(pending[0].agent_draft, "The agent wrote this line plainly.");
        assert_eq!(
            pending[0].operator_edit,
            "The operator rewrote this line with flair."
        );
        assert_eq!(pending[0].content_rating, "general");
    }

    /// Test 3 (host semantics): a HOST-origin draft stamps `"host"`, and an
    /// operator re-save over a host draft does NOT capture — host drafting is
    /// the operator's own prose, not agent style signal (§3.9 intent). The
    /// host path is `authoring_save_scene_draft`; at the service seam a host
    /// save is a non-explicit save with NO valid `draft` receipt whose origin
    /// is stamped `"host"` by the caller. This test drives that stamp through
    /// `update_scene_draft_origin("host")` (as the host handler does) and
    /// asserts no capture.
    #[tokio::test]
    async fn host_draft_then_operator_resave_does_not_capture() {
        let (_tmp, svc) = fresh_service_local().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "host-nocapture".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "p".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        enable_style_learning(&svc, &project.project_id).await;

        // Host draft: a plain operator-style save, then stamped "host" the way
        // the host handler (authoring_save_scene_draft) does.
        let saved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: 1,
                full_text: "The host drafted this scene by hand.".to_string(),
                summary: "opening".to_string(),
                content_rating: spindle_core::models::ContentRating::General,
                ..Default::default()
            })
            .await
            .unwrap();
        svc.repository
            .update_scene_draft_origin(&saved.scene_id, "host")
            .await
            .unwrap();
        let scene = svc.repository.get_scene(&saved.scene_id).await.unwrap();
        assert_eq!(
            scene.draft_origin.as_deref(),
            Some("host"),
            "host draft carries host provenance"
        );

        // Operator re-save over a host draft: host prose is the operator's
        // own, not an agent style contrast → no capture.
        operator_resave(
            &svc,
            &project.project_id,
            "The host revised their own scene further.",
        )
        .await;

        let branch_id = svc
            .repository
            .active_branch_id_public(&project.project_id)
            .await
            .unwrap();
        let pending = svc
            .repository
            .list_style_edit_candidates(&project.project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert!(
            pending.is_empty(),
            "operator re-save over a host draft must not capture a style candidate"
        );
    }

    /// Test 4 (revise-loop agent-over-agent, non-explicit): two successive
    /// NON-explicit agent drafts (both via `generation_id`). The second
    /// re-save is itself an agent save (`is_agent_save = true`) — capture is
    /// short-circuited, and the scene keeps agent provenance.
    #[tokio::test]
    async fn non_explicit_agent_over_agent_keeps_stamp_no_capture() {
        let (tmp, svc) = fresh_service_local().await;
        configure_general_draft_agent(&svc, &tmp).await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "nonexpl-aoa".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "p".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        enable_style_learning(&svc, &project.project_id).await;

        let r1 = svc
            .register_generation_receipt(
                "draft",
                Some("general"),
                "draft-agent",
                "The agent drafted the watch, first pass.",
            )
            .await
            .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: "The agent drafted the watch, first pass.".to_string(),
            summary: "s".to_string(),
            content_rating: spindle_core::models::ContentRating::General,
            generation_id: Some(r1.id.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

        // Revise loop re-save: a second agent draft (agent-over-agent).
        let r2 = svc
            .register_generation_receipt(
                "draft",
                Some("general"),
                "draft-agent",
                "The agent revised the watch, second pass, sharper.",
            )
            .await
            .unwrap();
        let resaved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: 1,
                full_text: "The agent revised the watch, second pass, sharper.".to_string(),
                summary: "s".to_string(),
                content_rating: spindle_core::models::ContentRating::General,
                generation_id: Some(r2.id.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        let scene = svc.repository.get_scene(&resaved.scene_id).await.unwrap();
        assert!(
            scene
                .draft_origin
                .as_deref()
                .is_some_and(|o| o.starts_with("agent:")),
            "revise-loop re-save keeps agent provenance, got {:?}",
            scene.draft_origin
        );

        let branch_id = svc
            .repository
            .active_branch_id_public(&project.project_id)
            .await
            .unwrap();
        let pending = svc
            .repository
            .list_style_edit_candidates(&project.project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert!(
            pending.is_empty(),
            "non-explicit agent-over-agent re-save must not capture a candidate"
        );
    }
}

/// Style learning from operator edits (evolution §3.9, P5.4). Feed-side
/// tests: pending candidates surface in the refresh preview (counts + refs,
/// no prose) and are consumed by refresh.
mod style_learning_refresh {
    use super::*;
    use spindle_core::style::{
        CreateStyleProfileFromMarkdownInput, PreviewRefreshStyleProfileInput,
        RefreshStyleProfileInput,
    };

    /// A local-only service whose `style_analyze` route is EXPLICITLY bound
    /// to a local-stub agent. The built-in default style stub now errors
    /// (live-run bug 6a) so it cannot fabricate mock guidance over a real
    /// narrator voice; binding a local-stub agent (`builtin_default = false`)
    /// is the documented opt-in that makes the stub serve its deterministic
    /// mock — exactly what these style-learning refresh tests need.
    async fn fresh_service_with_local_style() -> (TempDir, SqliteSpindleService) {
        use spindle_core::models::ConfigureAgentsInput;
        let (tmp, svc) = fresh_service_local().await;
        let config_path = tmp.path().join("style-agent.toml");
        std::fs::write(
            &config_path,
            r#"
[health_check]
enabled = false

[[agents]]
id = "local-style"
name = "Local Style Stub"
provider = "local"
endpoint = "local"
model = "local-style"
ratings = ["general", "teen", "mature", "explicit"]

[[routing]]
route = "style_analyze"
agent = "local-style"

[[routing]]
route = "style_revise"
agent = "local-style"
"#,
        )
        .unwrap();
        svc.configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.display().to_string()),
        })
        .unwrap();
        (tmp, svc)
    }

    /// Create a project with an opted-in style-learning flag, a style
    /// profile built from a small Markdown corpus, and an agent-drafted
    /// scene the operator then edits (capturing one pending candidate).
    /// Returns (project_id, branch_id, profile_id).
    async fn project_with_profile_and_candidate(
        svc: &SqliteSpindleService,
        data_dir: &std::path::Path,
        rating: spindle_core::models::ContentRating,
    ) -> (String, String, String) {
        let project = svc
            .create_project(CreateProjectInput {
                name: "refresh".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "p".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        // Opt in to style learning.
        svc.update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "project".to_string(),
            entity_id: project.project_id.clone(),
            changes: serde_json::json!({ "style_learning": 1 }),
        })
        .await
        .unwrap();

        // Markdown corpus under the data_dir (an allowed root).
        let md_path = data_dir.join("corpus.md");
        std::fs::write(
            &md_path,
            "The wind moved slow across the plain. She watched it, unhurried, \
                 and said nothing. The grass bent. The light held. A long breath \
                 passed before the door opened and the day began again.",
        )
        .unwrap();
        let created = svc
            .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
                project_id: project.project_id.clone(),
                profile_name: "House".to_string(),
                source_paths: vec![md_path.to_string_lossy().to_string()],
                recursive: None,
                include_globs: None,
                exclude_globs: None,
                max_files: None,
                max_bytes_per_file: None,
                max_total_words: None,
                apply: None,
                application_mode: None,
                source_sample_word_budget: None,
                metrics_only: Some(true),
                force_apply: None,
            })
            .await
            .unwrap();

        // Agent-drafted scene, then an operator edit → one pending candidate.
        let saved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: 1,
                full_text: "Agent prose that reads flat and plain.".to_string(),
                summary: "s".to_string(),
                content_rating: rating.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        svc.repository
            .update_scene_draft_origin(&saved.scene_id, "agent:test-agent")
            .await
            .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: "Operator prose that breathes and turns with care.".to_string(),
            summary: "s".to_string(),
            content_rating: rating,
            ..Default::default()
        })
        .await
        .unwrap();

        let branch_id = svc
            .repository
            .active_branch_id_public(&project.project_id)
            .await
            .unwrap();
        (project.project_id, branch_id, created.profile.profile_id)
    }

    // Test 3: preview surfaces candidates (count + refs, no full prose);
    // refresh consumes → status consumed; consumed candidates absent from
    // the next preview.
    #[tokio::test]
    async fn preview_surfaces_candidates_and_refresh_consumes_them() {
        let (tmp, svc) = fresh_service_with_local_style().await;
        let data_dir = svc.repository().data_dir().to_path_buf();
        let (project_id, branch_id, profile_id) = project_with_profile_and_candidate(
            &svc,
            &data_dir,
            spindle_core::models::ContentRating::General,
        )
        .await;

        // Preview surfaces the candidate: count + ref, prose-free diff.
        let preview = svc
            .preview_refresh_style_profile(PreviewRefreshStyleProfileInput {
                project_id: project_id.clone(),
                profile_id: profile_id.clone(),
                metrics_only: Some(true),
            })
            .await
            .unwrap();
        assert_eq!(preview.style_edit_candidates.included_count, 1);
        assert_eq!(preview.style_edit_candidates.withheld_count, 0);
        assert_eq!(preview.style_edit_candidates.candidates.len(), 1);
        let cref = &preview.style_edit_candidates.candidates[0];
        assert_eq!(cref.scene_ref, "1.1.1");
        assert!(cref.included);
        // The preview must NOT carry the full prose (metadata/diff only).
        let serialized = serde_json::to_string(&preview).unwrap();
        assert!(
            !serialized.contains("Operator prose that breathes"),
            "preview must not carry full operator prose"
        );
        assert!(
            !serialized.contains("Agent prose that reads flat"),
            "preview must not carry full agent prose"
        );

        // Refresh consumes the candidate.
        let refreshed = svc
            .refresh_style_profile(RefreshStyleProfileInput {
                project_id: project_id.clone(),
                profile_id: profile_id.clone(),
                apply_after_refresh: None,
                force_apply: None,
                metrics_only: Some(true),
                dismiss_candidate_ids: Vec::new(),
            })
            .await
            .unwrap();
        assert_eq!(refreshed.consumed_candidate_ids.len(), 1);
        assert_eq!(refreshed.consumed_candidate_ids[0], cref.candidate_id);

        // The candidate is now consumed, not pending.
        let pending = svc
            .repository
            .list_style_edit_candidates(&project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        assert!(
            pending.is_empty(),
            "consumed candidate leaves the pending set"
        );
        let consumed = svc
            .repository
            .list_style_edit_candidates(&project_id, &branch_id, Some("consumed"))
            .await
            .unwrap();
        assert_eq!(consumed.len(), 1);

        // The NEXT preview (on the refreshed profile) shows no candidates.
        let next = svc
            .preview_refresh_style_profile(PreviewRefreshStyleProfileInput {
                project_id: project_id.clone(),
                profile_id: refreshed.new_profile.profile_id.clone(),
                metrics_only: Some(true),
            })
            .await
            .unwrap();
        assert_eq!(next.style_edit_candidates.included_count, 0);
        drop(tmp);
    }

    // Dismissal path (evolution §3.9): a dismissed candidate is flipped to
    // `dismissed`, not fed, and never resurfaces.
    #[tokio::test]
    async fn refresh_dismisses_named_candidates() {
        let (tmp, svc) = fresh_service_with_local_style().await;
        let data_dir = svc.repository().data_dir().to_path_buf();
        let (project_id, branch_id, profile_id) = project_with_profile_and_candidate(
            &svc,
            &data_dir,
            spindle_core::models::ContentRating::General,
        )
        .await;
        let pending = svc
            .repository
            .list_style_edit_candidates(&project_id, &branch_id, Some("pending"))
            .await
            .unwrap();
        let cid = pending[0].id.clone();

        let refreshed = svc
            .refresh_style_profile(RefreshStyleProfileInput {
                project_id: project_id.clone(),
                profile_id: profile_id.clone(),
                apply_after_refresh: None,
                force_apply: None,
                metrics_only: Some(true),
                dismiss_candidate_ids: vec![cid.clone()],
            })
            .await
            .unwrap();
        assert_eq!(refreshed.dismissed_candidate_ids, vec![cid.clone()]);
        assert!(
            refreshed.consumed_candidate_ids.is_empty(),
            "a dismissed candidate is not also consumed"
        );
        let dismissed = svc
            .repository
            .list_style_edit_candidates(&project_id, &branch_id, Some("dismissed"))
            .await
            .unwrap();
        assert_eq!(dismissed.len(), 1);
        drop(tmp);
    }
}
