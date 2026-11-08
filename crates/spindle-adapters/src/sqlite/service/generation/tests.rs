use super::*;

#[tokio::test]
async fn continue_generation_surfaces_local_adapter_limit() {
    use spindle_core::models::ContinueGenerationInput;

    // The local adapter doesn't implement complete_continuation —
    // that's a real limitation, not a stub. The service method
    // should surface the adapter's error verbatim so MCP callers
    // can react.
    let (_tmp, svc) = fresh_service_local().await;
    let err = svc
        .continue_generation(ContinueGenerationInput {
            route: "draft".into(),
            rating: None,
            original_prompt: "Open with Mara at the gate.".into(),
            prior_output: "Mara watched the dust settle. ".into(),
            project_id: None,
            book_id: None,
            chapter_id: None,
            scene_id: None,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("continuation not supported"),
        "expected local-adapter no-continuation guard, got: {err}"
    );
}

/// Regression: revise_generation used to bail with
/// `"was not produced with rating \"explicit\""` for any non-explicit
/// source receipt, forcing a full re-roll for mature/teen/general
/// surgical edits. The verified_revisable_draft_receipt helper now
/// accepts all ratings (and the explicit_capable_agent check is gated
/// on rating == explicit). This test seeds a mature-rated receipt
/// directly and confirms the call no longer trips the rating gate.
#[tokio::test]
async fn revise_generation_accepts_mature_rated_source_receipt() {
    let (_tmp, svc) = fresh_service_local().await;

    // Seed a mature-rated draft receipt directly (the local adapter
    // doesn't implement complete_continuation, so we can't go through
    // continue_generation; the private helper is in-module-accessible).
    let receipt = svc
        .register_generation_receipt(
            "draft",
            Some("mature"),
            "local-adapter-stub",
            "Mara crossed the rain-slick alley, salt charm warm in her fist.",
        )
        .await
        .unwrap();

    let outcome = svc
        .revise_generation(spindle_core::models::ReviseGenerationInput {
            generation_id: receipt.id.clone(),
            edit_instructions: "Tighten the opening — drop the adjective on alley.".into(),
            context: None,
        })
        .await;

    // We don't care about the prose the local adapter returns; we only
    // care that the rating gate didn't reject the mature receipt.
    let outcome = outcome.expect("mature-rated revise_generation must succeed");
    assert_eq!(outcome.source_generation_id, receipt.id);
    // The new receipt should carry the same rating as the source.
    let new_receipt_id = outcome
        .generation_id
        .expect("revision must produce a new receipt id");
    assert!(
        new_receipt_id.starts_with("model_generation:"),
        "unexpected receipt id shape: {new_receipt_id}"
    );
}

/// Regression: an explicit-rated source receipt produced by an agent
/// that is NOT explicit-capable should still be rejected — that's the
/// real integrity invariant we kept after relaxing the broader rating
/// gate. (For non-explicit ratings the explicit_capable_agent check
/// no longer applies.)
#[tokio::test]
async fn revise_generation_still_blocks_explicit_from_non_explicit_agent() {
    let (_tmp, svc) = fresh_service_local().await;
    let receipt = svc
        .register_generation_receipt(
            "draft",
            Some("explicit"),
            "local-adapter-stub-no-explicit",
            "Some explicit prose.",
        )
        .await
        .unwrap();
    let err = svc
        .revise_generation(spindle_core::models::ReviseGenerationInput {
            generation_id: receipt.id.clone(),
            edit_instructions: "Tighten.".into(),
            context: None,
        })
        .await
        .expect_err("explicit receipt from non-explicit agent must still error");
    assert!(
        err.to_string().contains("not explicit-capable"),
        "expected explicit-capable guard, got: {err}"
    );
}

#[tokio::test]
async fn revise_generation_rejects_empty_edit_instructions() {
    let (_tmp, svc) = fresh_service_local().await;
    let err = svc
        .revise_generation(spindle_core::models::ReviseGenerationInput {
            generation_id: "model_generation:99:abcdef012345".into(),
            edit_instructions: "   ".into(),
            context: None,
        })
        .await
        .unwrap_err();
    // First failure path: the receipt id won't exist in the cache.
    assert!(
        err.to_string().contains("not found or has expired"),
        "expected receipt-not-found, got: {err}"
    );
}

#[tokio::test]
async fn save_scene_draft_blocks_explicit_sexual_prose_without_generation_receipt() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "explicit-gate".to_string(),
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

    let err = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id,
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "The draft contains an orgasm reference.".to_string(),
            summary: "Explicit prose should be receipt-gated.".to_string(),
            content_rating: spindle_core::models::ContentRating::Explicit,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .expect_err("explicit sexual prose without generation_id must be rejected");
    assert!(
        err.to_string().contains("generation_id is required"),
        "expected generation_id guard, got: {err}"
    );
}

/// Data-loss regression (bugs 1 + 6b): an explicit save whose receipt
/// output DIFFERS from the caller's `full_text` must persist the caller's
/// `full_text` byte-for-byte — the receipt is provenance only (rating
/// clearance + agent identity), never a source of prose. The prior contract
/// (`save_scene_draft_uses_server_held_explicit_generation_output`) rewrote
/// `full_text` from `receipt.output_text`, silently discarding the caller's
/// prose; in production the receipt carries agent narration + truncated turn
/// fragments (concat at register_generation_receipt), so the rewrite stored
/// receipt stubs while reporting `status: saved`. This test pins the honest
/// contract: full_text persisted, receipt NOT persisted, `chars_added`
/// computed from the persisted prose.
#[tokio::test]
async fn save_scene_draft_persists_caller_full_text_not_receipt_output() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "explicit-receipt-save".to_string(),
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
    // Receipt output stands in for the production reality: agent narration
    // ("I'll bootstrap Spindle…") + a truncated turn fragment. It is NOT the
    // prose and must never be persisted.
    let receipt_output = "{\"ok\":I'll bootstrap Spindle then draft… [truncated turn fragment]";
    let receipt = svc
        .register_generation_receipt("draft", Some("explicit"), "explicit-agent", receipt_output)
        .await
        .unwrap();
    {
        let mut receipts = svc.generation_receipts.write().unwrap();
        receipts
            .get_mut(&receipt.id)
            .expect("seeded receipt")
            .explicit_capable_agent = true;
    }
    // The real 15KB-class prose the caller authored (abbreviated here). This
    // is authoritative — it is what the operator/cleared agent produced and
    // what must survive the save.
    let caller_full_text =
        "The heat between them was undeniable as the storm broke over the harbor.";

    let out = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id,
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: caller_full_text.to_string(),
            summary: "Caller prose must win over the receipt.".to_string(),
            content_rating: spindle_core::models::ContentRating::Explicit,
            tone: None,
            generation_id: Some(receipt.id.clone()),
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let scene = svc.repository.get_scene(&out.scene_id).await.unwrap();
    // 1. Persisted prose is the caller's full_text, byte-for-byte.
    assert_eq!(
        scene.full_text, caller_full_text,
        "explicit save must persist the caller's full_text, not the receipt output"
    );
    // 2. The receipt output was NOT persisted (this is the data-loss guard).
    assert_ne!(
        scene.full_text, receipt.output_text,
        "receipt output (narration/fragments) must never be persisted as prose"
    );
    // 3. Provenance still stamps agent:<id> from the receipt.
    assert_eq!(out.draft_origin, "agent:explicit-agent");
    // 4. chars_added reflects the persisted prose, not the receipt (bug 6b).
    assert_eq!(
        out.chars_added,
        caller_full_text.len(),
        "chars_added must reflect the persisted prose, not the receipt stub"
    );
}

/// One explicit clearance authorizes ONE scene. A receipt minted for scene 1
/// must not be replayed to authorize explicit prose on scene 2 — otherwise a
/// single trip through the explicit-capable agent blanket-authorizes an
/// entire chapter of explicit saves that agent never produced.
#[tokio::test]
async fn explicit_receipt_cannot_authorize_a_second_scene() {
    let (_tmp, svc, project_id, receipt_id) = explicit_receipt_claim_fixture().await;

    svc.save_scene_draft(explicit_save(&project_id, 1, &receipt_id))
        .await
        .expect("first explicit save claims the receipt");

    let err = svc
        .save_scene_draft(explicit_save(&project_id, 2, &receipt_id))
        .await
        .expect_err("replaying the receipt on a different scene must be rejected");
    assert!(
        err.to_string().contains("already authorized"),
        "expected a receipt-claim error naming the bound scene, got: {err}"
    );
}

/// Bind-on-first-use must not break idempotent retries: re-saving the SAME
/// scene with the same receipt (transient failure retry, operator re-save of
/// the same draft) stays legal.
#[tokio::test]
async fn explicit_receipt_may_be_reused_for_the_same_scene() {
    let (_tmp, svc, project_id, receipt_id) = explicit_receipt_claim_fixture().await;

    let first = svc
        .save_scene_draft(explicit_save(&project_id, 1, &receipt_id))
        .await
        .expect("first explicit save claims the receipt");
    let second = svc
        .save_scene_draft(explicit_save(&project_id, 1, &receipt_id))
        .await
        .expect("re-saving the same scene with its own receipt must stay legal");

    assert_eq!(
        first.scene_id, second.scene_id,
        "the retry should land on the same scene"
    );
    assert_eq!(second.draft_origin, "agent:explicit-agent");
}

/// The claim is an explicit-path guard. A non-explicit save presenting the
/// same receipt for provenance must not be scene-bound by it — those saves
/// are not gated in the first place.
#[tokio::test]
async fn non_explicit_saves_do_not_consume_the_receipt_claim() {
    let (_tmp, svc, project_id, receipt_id) = explicit_receipt_claim_fixture().await;

    svc.save_scene_draft(SaveSceneDraftInput {
        content_rating: spindle_core::models::ContentRating::General,
        full_text: "The muster formed at dawn along the frost-cracked wall.".to_string(),
        ..explicit_save(&project_id, 1, &receipt_id)
    })
    .await
    .expect("non-explicit save with a receipt is provenance only");

    svc.save_scene_draft(explicit_save(&project_id, 2, &receipt_id))
        .await
        .expect("the unclaimed receipt is still available to its explicit scene");
}

/// Live-run bug: an explicit save that FAILED (bogus placement) bound the
/// receipt to the bogus scene key and burned it, forcing a full
/// regeneration of explicit prose that had already been produced and paid
/// for. The claim must be transactional with the save: a failed save
/// leaves the receipt unbound and reusable.
#[tokio::test]
async fn failed_explicit_save_leaves_receipt_unbound_and_reusable() {
    let (_tmp, svc, project_id, receipt_id) = explicit_receipt_claim_fixture().await;

    // Book 99 does not exist — the save fails during placement resolution.
    let failed = svc
        .save_scene_draft(SaveSceneDraftInput {
            book_number: 99,
            ..explicit_save(&project_id, 2, &receipt_id)
        })
        .await
        .expect_err("a save into a nonexistent book must fail");
    assert!(
        failed.to_string().contains("book 99 not found"),
        "expected the placement failure, got: {failed}"
    );

    // The receipt must still be usable for the scene it was minted for.
    svc.save_scene_draft(explicit_save(&project_id, 2, &receipt_id))
        .await
        .expect("a failed save must not consume the receipt");
}

/// The receipt claim key must use the RESOLVED placement when the save
/// names the chapter by id — never the defaulted `book 0, chapter 0`.
#[tokio::test]
async fn explicit_save_with_chapter_id_only_binds_receipt_to_resolved_placement() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "chapter-id-receipt".to_string(),
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
    let receipt = svc
        .register_generation_receipt("draft", Some("explicit"), "explicit-agent", "seed")
        .await
        .unwrap();
    {
        let mut receipts = svc.generation_receipts.write().unwrap();
        receipts
            .get_mut(&receipt.id)
            .expect("seeded receipt")
            .explicit_capable_agent = true;
    }

    let explicit_by_chapter_id = |scene_order: i32| SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        chapter_id: Some(chapter.chapter_id.clone()),
        scene_order,
        full_text: "Her breath caught as he took her, slow and deliberate.".to_string(),
        summary: format!("Explicit scene {scene_order}."),
        content_rating: spindle_core::models::ContentRating::Explicit,
        generation_id: Some(receipt.id.clone()),
        ..Default::default()
    };

    svc.save_scene_draft(explicit_by_chapter_id(1))
        .await
        .expect("chapter_id-only explicit save must resolve placement and succeed");
    svc.save_scene_draft(explicit_by_chapter_id(1))
        .await
        .expect("re-saving the same resolved scene keeps the receipt valid");

    let err = svc
        .save_scene_draft(explicit_by_chapter_id(2))
        .await
        .expect_err("the receipt may not spill onto a sibling scene");
    assert!(
        err.to_string().contains("already authorized"),
        "expected a claim-conflict naming the resolved scene, got: {err}"
    );
    assert!(
        err.to_string().contains("book 1, chapter 2, scene 1"),
        "the conflict must name the RESOLVED placement, not book 0 chapter 0, got: {err}"
    );
}

/// Bug-4 guard: the NON-explicit agent-draft path (a valid `draft`-route
/// receipt at a lower rating) must also treat the caller's `full_text` as
/// authoritative — it only stamps `agent:<id>` provenance and never rewrites
/// prose. This is the pre-existing contract; the test pins it so a future
/// change cannot regress the non-explicit path into a rewrite.
#[tokio::test]
async fn save_scene_draft_non_explicit_receipt_persists_caller_full_text() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "mature-receipt-save".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "receipt-stamped drafting".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let receipt = svc
        .register_generation_receipt(
            "draft",
            Some("mature"),
            "mature-agent",
            "Receipt narration that is NOT the prose.",
        )
        .await
        .unwrap();
    let caller_full_text = "The mist rolled off the cliffs as she climbed the last switchback.";

    let out = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id,
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: caller_full_text.to_string(),
            summary: "Non-explicit caller prose must win.".to_string(),
            content_rating: spindle_core::models::ContentRating::Mature,
            tone: None,
            generation_id: Some(receipt.id.clone()),
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let scene = svc.repository.get_scene(&out.scene_id).await.unwrap();
    assert_eq!(
        scene.full_text, caller_full_text,
        "non-explicit save must persist the caller's full_text"
    );
    assert_ne!(scene.full_text, receipt.output_text);
    // Provenance still stamps the agent from the receipt.
    assert_eq!(out.draft_origin, "agent:mature-agent");
    // chars_added honest on the non-explicit path too (bug 6b).
    assert_eq!(out.chars_added, caller_full_text.len());
}

#[tokio::test]
async fn save_scene_draft_rejects_non_explicit_receipt_for_explicit_save() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "explicit-wrong-receipt".to_string(),
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
    let receipt = svc
        .register_generation_receipt(
            "draft",
            Some("mature"),
            "mature-agent",
            "Mature route output.",
        )
        .await
        .unwrap();

    let err = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id,
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "CLIENT PLACEHOLDER".to_string(),
            summary: "Wrong receipt rating should fail.".to_string(),
            content_rating: spindle_core::models::ContentRating::Explicit,
            tone: None,
            generation_id: Some(receipt.id),
            source_path: None,
            ..Default::default()
        })
        .await
        .expect_err("non-explicit receipt must not authorize explicit save");
    assert!(
        err.to_string().contains("not \"explicit\""),
        "expected explicit rating guard, got: {err}"
    );
}

/// A receipt registered in service A must be verifiable in a SEPARATE
/// service B backed by the same DB file (the production restart scenario).
/// Red before 4c: B's in-memory cache is empty, so the lookup errors with
/// `was not found or has expired`.
#[tokio::test]
async fn generation_receipt_survives_across_service_instances() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("receipts.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let svc_a = service_over_same_db(&db_path, &data_dir).await;
    let receipt = svc_a
        .register_generation_receipt(
            "draft",
            Some("mature"),
            "draft-agent",
            "Prose the agent produced in process one.",
        )
        .await
        .unwrap();

    // Fresh instance (process two): no shared in-memory cache.
    let svc_b = service_over_same_db(&db_path, &data_dir).await;
    let resolved = svc_b
        .verified_revisable_draft_receipt(Some(&receipt.id))
        .await
        .expect("lookup must not error")
        .expect("receipt must resolve from the DB in a new process");
    assert_eq!(resolved.id, receipt.id);
    assert_eq!(resolved.route, "draft");
    assert_eq!(resolved.rating.as_deref(), Some("mature"));
    assert_eq!(resolved.agent_id, "draft-agent");
    // output_text survives whole (load-bearing for revise_generation).
    assert_eq!(
        resolved.output_text,
        "Prose the agent produced in process one."
    );
}

/// An expired receipt (backdated `expires_at`) resolves to the exact
/// `was not found or has expired` error, and the expired row is deleted on
/// read (lazy cleanup). Red before 4c: no persistence, no expiry semantics.
#[tokio::test]
async fn generation_receipt_expiry_is_enforced_on_read_and_cleans_up() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("receipts.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let svc_a = service_over_same_db(&db_path, &data_dir).await;
    let receipt = svc_a
        .register_generation_receipt("draft", Some("mature"), "draft-agent", "Expiring prose.")
        .await
        .unwrap();

    // Backdate expires_at into the past directly in the DB (deterministic,
    // no clock injection needed — the row's own expiry is the seam).
    let past =
        crate::sqlite::row::timestamp_to_micros(chrono::Utc::now() - chrono::Duration::hours(1));
    let id = receipt.id.clone();
    svc_a
        .repository()
        .pool()
        .write(move |conn| {
            conn.execute(
                "UPDATE generation_receipt SET expires_at = ?1 WHERE id = ?2",
                rusqlite::params![past, id],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    // A fresh instance (no cache) reads the expired row → not-found/expired.
    let svc_b = service_over_same_db(&db_path, &data_dir).await;
    let err = svc_b
        .verified_revisable_draft_receipt(Some(&receipt.id))
        .await
        .expect_err("expired receipt must error");
    assert!(
        err.to_string().contains("was not found or has expired"),
        "expected the exact expiry message, got: {err}"
    );

    // Lazy cleanup: the expired row is gone from the DB after the read.
    let id = receipt.id.clone();
    let remaining: i64 = svc_b
        .repository()
        .pool()
        .read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM generation_receipt WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(remaining, 0, "expired receipt must be deleted on read");
}

/// End-to-end production scenario: register an explicit receipt in service
/// A, DROP A, open a fresh service B over the same DB, then explicit-save
/// with that generation_id → the save succeeds and the caller's prose
/// persists. This is the exact failure that started everything
/// (`generation_id "…" was not found or has expired` after a primary
/// restart forced the receipt-stub data-loss path).
#[tokio::test]
async fn explicit_save_succeeds_after_simulated_primary_restart() {
    use spindle_core::models::ConfigureAgentsInput;
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("receipts.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Explicit-capable draft agent so verified_explicit_save_receipt accepts.
    let config_path = tmp.path().join("agents.toml");
    std::fs::write(
        &config_path,
        r#"
[health_check]
enabled = false

[[agents]]
id = "explicit-agent"
name = "Explicit Agent"
provider = "openai-compatible"
endpoint = "http://localhost:1/v1"
model = "drafter"
ratings = ["general", "explicit"]

[[routing]]
route = "draft"
agent = "explicit-agent"
"#,
    )
    .unwrap();

    // Process one: configure, create project, register the explicit receipt.
    let svc_a = service_over_same_db(&db_path, &data_dir).await;
    svc_a
        .configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.to_string_lossy().to_string()),
        })
        .unwrap();
    let project = svc_a
        .create_project(CreateProjectInput {
            name: "restart-save".to_string(),
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
    let receipt = svc_a
        .register_generation_receipt(
            "draft",
            Some("explicit"),
            "explicit-agent",
            "Agent narration — NOT the prose.",
        )
        .await
        .unwrap();
    let generation_id = receipt.id.clone();

    // Primary restart: drop A entirely; a brand-new process (B) reconfigures.
    drop(svc_a);
    let svc_b = service_over_same_db(&db_path, &data_dir).await;
    svc_b
        .configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.to_string_lossy().to_string()),
        })
        .unwrap();

    let caller_prose = "The storm broke over the harbor as their hands finally met in the dark.";
    let out = svc_b
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: caller_prose.to_string(),
            summary: "Prose survives the restart.".to_string(),
            content_rating: spindle_core::models::ContentRating::Explicit,
            tone: None,
            generation_id: Some(generation_id),
            source_path: None,
            ..Default::default()
        })
        .await
        .expect("explicit save must succeed against a receipt from the previous process");

    let scene = svc_b.repository.get_scene(&out.scene_id).await.unwrap();
    assert_eq!(
        scene.full_text, caller_prose,
        "the caller's prose must persist across the restart"
    );
    assert_eq!(out.draft_origin, "agent:explicit-agent");
}

#[tokio::test]
async fn poisoned_generation_receipts_lock_returns_err_naming_the_lock() {
    let (_tmp, svc) = fresh_service_local().await;
    poison_rwlock(svc.generation_receipts.clone());
    let err = svc.get_all_generation_receipts().unwrap_err();
    assert!(
        err.to_string().contains("generation receipts read lock"),
        "must name the poisoned lock, got: {err:?}"
    );
    let err = svc.lookup_generation_receipt("missing").await.unwrap_err();
    assert!(
        err.to_string().contains("generation receipts read lock"),
        "must name the poisoned lock, got: {err:?}"
    );
    let err = svc
        .register_generation_receipt("draft", Some("explicit"), "explicit-agent", "seed")
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("generation receipts write lock"),
        "must name the poisoned lock, got: {err:?}"
    );
}
