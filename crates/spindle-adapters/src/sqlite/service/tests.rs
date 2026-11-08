use super::*;
use crate::sqlite::SqlitePool;
use spindle_core::models::{
    ReaderContract, ResearchAddClaimInput, ResearchAddNoteInput, ResearchAddSourceInput,
    ResearchPackForSceneInput, ResearchSearchInput,
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

/// Variant that pins the model router to the local-only adapter set.
/// Use when a test exercises a code path that calls `model_router.complete`
/// and needs a deterministic, network-free response (the default
/// `Repository::new` reads `~/.config/spindle/agent-config.toml` if
/// present, which on dev machines points at a real HTTP endpoint).
async fn fresh_service_local() -> (TempDir, SqliteSpindleService) {
    use crate::ai::ModelRouter;
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::with_model_router(pool, data_dir, ModelRouter::local_only());
    (tmp, SqliteSpindleService::new(repo))
}

async fn project_with_scene(
    svc: &SqliteSpindleService,
    name: &str,
) -> (CreateProjectOutput, String, String) {
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
    let full_text = format!("{name} opens with a quiet scene.");
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: full_text.clone(),
            summary: "opening scene".to_string(),
            content_rating: spindle_core::models::ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    (project, scene.scene_id, full_text)
}

async fn seed_phase_four_cache(
    svc: &SqliteSpindleService,
    project_id: &str,
    branch_id: &str,
    scene_id: &str,
    full_text: &str,
    cache_id: PhaseFourCacheId,
) -> String {
    let scene_text_hash = generation_sha256_hex(full_text.as_bytes());
    svc.repository
        .upsert_validator_finding(crate::sqlite::repository::UpsertValidatorFindingParams {
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            scene_id: scene_id.to_string(),
            scene_text_hash: scene_text_hash.clone(),
            context_hash: None,
            validator_id: cache_id.as_str().to_string(),
            finding_id: "__cache__".to_string(),
            severity: "info".to_string(),
            message: "warm cache".to_string(),
            byte_range: None,
            details_json: Some(serde_json::json!({ "issues": [] })),
        })
        .await
        .unwrap();
    assert_active_phase_four_cache(svc, branch_id, scene_id, &scene_text_hash, cache_id, 1).await;
    scene_text_hash
}

async fn assert_active_phase_four_cache(
    svc: &SqliteSpindleService,
    branch_id: &str,
    scene_id: &str,
    scene_text_hash: &str,
    cache_id: PhaseFourCacheId,
    expected: usize,
) {
    let rows = svc
        .repository
        .list_active_validator_findings_by_scene_hash(
            branch_id,
            scene_id,
            scene_text_hash,
            &[cache_id.as_str().to_string()],
        )
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        expected,
        "unexpected active cache count for {}",
        cache_id.as_str()
    );
}

/// Seed one chapter with a placeholder scene (literal "placeholder",
/// 1 word), a too-thin scene (5 words), and a healthy scene — the exact
/// shape that shipped to EPUB in the live run while every gate reported
/// clean.
async fn chapter_with_stub_scenes(svc: &SqliteSpindleService, project_id: &str) {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};
    for (scene_order, body) in [
        (1, "placeholder"),
        (2, "She waited for the dawn."),
        (
            3,
            "The wardens lined the wall at first light, their breath fogging in the cold as the bells began to ring across the city.",
        ),
    ] {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order,
            full_text: body.into(),
            summary: format!("scene {scene_order}"),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    }
}

/// Variant of `fresh_service` whose data dir is a real `.spindle`
/// directory under a tempdir, so `artifacts_dir_for_data_dir` resolves to
/// the workspace root (the tempdir) exactly as it does in a real project.
async fn fresh_service_workspace() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join(".spindle");
    std::fs::create_dir_all(&data_dir).unwrap();
    let pool = SqlitePool::open(&data_dir.join("spindle.db"))
        .await
        .unwrap();
    let repo = Repository::new(pool, data_dir);
    (tmp, SqliteSpindleService::new(repo))
}

// ── Reader-facing artifacts (export_recap / export_series_bible §3.8) ────

/// Create a bare project for the reader-artifact tests.
async fn recap_project(svc: &SqliteSpindleService, name: &str) -> String {
    svc.create_project(CreateProjectInput {
        name: name.into(),
        project_type: "novel".into(),
        genre: "fantasy".into(),
        reader_contract: ReaderContract {
            promise: "p".into(),
            style_notes: Vec::new(),
            boundaries: Vec::new(),
        },
    })
    .await
    .unwrap()
    .project_id
}

/// Save one committed scene and return its scene id. Ensures the chapter row
/// exists first (`save_scene_draft` requires it), so non-sequential chapters
/// can be seeded directly in a test.
async fn recap_scene(
    svc: &SqliteSpindleService,
    project_id: &str,
    book: i32,
    chapter: i32,
    order: i32,
    text: &str,
) -> String {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};
    svc.repository
        .ensure_chapter(project_id, book, chapter)
        .await
        .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.to_string(),
        book_number: book,
        chapter_number: chapter,
        chapter_id: None,
        scene_order: order,
        full_text: text.into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap()
    .scene_id
}

/// Register a secret canonical fact whose value carries `sentinel`, returning
/// its id. Uses a committed scene as the anchor.
async fn recap_secret_fact(
    svc: &SqliteSpindleService,
    project_id: &str,
    scene_id: &str,
    holder_id: &str,
    sentinel: &str,
) -> String {
    use spindle_core::models::{RegisterCanonicalFactInput, SecrecyScope};
    svc.register_canonical_fact(RegisterCanonicalFactInput {
        project_id: project_id.to_string(),
        scene_id: Some(scene_id.to_string()),
        book_number: 1,
        chapter_number: 1,
        fact_type: None,
        key: Some("Vex:trueheir".into()),
        value: Some(sentinel.to_string()),
        context: None,
        subject_table: None,
        subject_id: None,
        predicate: None,
        value_kind: None,
        value_text: Some(sentinel.to_string()),
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
            holder_ids: vec![holder_id.to_string()],
            concealment_note: None,
        }),
    })
    .await
    .unwrap()
    .canonical_fact_id
}

/// Create a character with the given name, returning its id.
async fn create_character_named(
    svc: &SqliteSpindleService,
    project_id: &str,
    name: &str,
) -> String {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CreateCharacterInput,
    };
    svc.create_character(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: project_id.to_string(),
        name: name.into(),
        summary: format!("{name} of the docks."),
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
        initial_state: Some(CharacterStatePatch {
            emotional_state: std::collections::BTreeMap::new(),
            goals: None,
            status: None,
            notes: None,
            source_summary: None,
        }),
    })
    .await
    .unwrap()
    .character_id
}

/// A `review` endpoint that sleeps `delay` before answering, serving each
/// connection on its own thread so concurrent persona calls really overlap.
/// Stands in for a reasoning model whose single call outlives an MCP
/// request timeout.
fn spawn_slow_review_server(delay: std::time::Duration) -> String {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind slow server");
    let address = listener.local_addr().expect("listener address");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            std::thread::spawn(move || {
                let mut buffer = [0_u8; 16384];
                let _ = stream.read(&mut buffer);
                std::thread::sleep(delay);
                let body = serde_json::json!({
                        "choices": [{
                            "message": {
                                "content": "STRENGTHS:\n- slow model strength\n\nCONCERNS:\n- slow model concern"
                            },
                            "finish_reason": "stop"
                        }]
                    })
                    .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            });
        }
    });
    format!("http://{address}/v1")
}

/// Seed a project with one saved scene and a review route pointing at a
/// deliberately slow model. Returns (tmp, service, project_id, scene_id).
async fn slow_review_fixture(
    delay: std::time::Duration,
) -> (TempDir, SqliteSpindleService, String, String) {
    use spindle_core::models::{ConfigureAgentsInput, ContentRating, SaveSceneDraftInput};

    let (tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "slow-review".into(),
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
    let saved = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "The wall held through the night and the horns did not sound again.".into(),
            summary: "Quiet watch.".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let endpoint = spawn_slow_review_server(delay);
    let config_path = tmp.path().join("slow-review.toml");
    std::fs::write(
        &config_path,
        format!(
            r####"
[health_check]
enabled = false

[[agents]]
id = "slow-review"
name = "Slow Review"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "slow-review"
ratings = ["general", "teen", "mature", "explicit"]

[[routing]]
route = "review"
agent = "slow-review"
"####
        ),
    )
    .unwrap();
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.display().to_string()),
    })
    .unwrap();

    (tmp, svc, proj.project_id, saved.scene_id)
}

async fn await_review_rounds(
    svc: &SqliteSpindleService,
    branch_id: &str,
    scene_id: &str,
    want_rounds: i64,
) {
    for _ in 0..200 {
        if let Some(row) = svc
            .repository
            .get_dual_persona_review(branch_id, scene_id)
            .await
            .unwrap()
            && row.rounds_completed >= want_rounds
            && row.status == "current"
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("review never reached {want_rounds} completed rounds");
}

/// THE REGRESSION: a review whose model is slower than the caller's patience
/// must still finish and persist.
///
/// Every round used to run inline with a single persist at the very end, so
/// when the MCP client gave up at ~60s the whole run was discarded — the
/// table held zero rows despite every model call having been paid for, and
/// the retry restarted from nothing. Now the call reports `in_progress` and
/// the detached job banks each round as it lands.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_review_completes_and_persists_after_caller_stops_waiting() {
    use spindle_core::models::RunDualPersonaReviewInput;

    // Each call outlives REVIEW_JOB_JOIN_WAIT (150ms under cfg!(test)).
    let (_tmp, svc, project_id, scene_id) =
        slow_review_fixture(std::time::Duration::from_millis(300)).await;
    let branch_id = svc
        .repository
        .get_active_branch(&project_id)
        .await
        .unwrap()
        .id;

    let first = svc
        .run_dual_persona_review(RunDualPersonaReviewInput {
            project_id: project_id.clone(),
            branch_id: None,
            scene_id: scene_id.clone(),
            rounds: Some(2),
        })
        .await
        .expect("the call must return, not hang, when the model is slow");

    // The caller is answered promptly with an honest in-progress status
    // rather than blocking until the client's own timeout fires.
    assert_eq!(
        first.status, "in_progress",
        "a review slower than the join wait must report in_progress"
    );
    assert!(
        first.rounds_completed < 2,
        "the run cannot already be complete at this delay"
    );

    // The detached job keeps going and banks both rounds.
    await_review_rounds(&svc, &branch_id, &scene_id, 2).await;

    // Exactly one row for the scene — no duplicate from the retry path.
    let all = svc
        .repository
        .list_dual_persona_reviews_by_project(&project_id)
        .await
        .unwrap();
    assert_eq!(all.len(), 1, "a review must never fan out into extra rows");
}

/// Retrying after a client timeout must COLLECT the finished work, not
/// stack another `rounds` on top of it. Without the in-flight/target-rounds
/// check, a rounds=2 retry over 2 banked rounds would run 2 MORE and bill
/// the operator twice for a review they already have.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retrying_a_timed_out_review_collects_instead_of_topping_up() {
    use spindle_core::models::RunDualPersonaReviewInput;

    let (_tmp, svc, project_id, scene_id) =
        slow_review_fixture(std::time::Duration::from_millis(200)).await;
    let branch_id = svc
        .repository
        .get_active_branch(&project_id)
        .await
        .unwrap()
        .id;
    let request = || RunDualPersonaReviewInput {
        project_id: project_id.clone(),
        branch_id: None,
        scene_id: scene_id.clone(),
        rounds: Some(2),
    };

    // First call times out its join wait and leaves the job running.
    let first = svc.run_dual_persona_review(request()).await.unwrap();
    assert_eq!(first.status, "in_progress");
    await_review_rounds(&svc, &branch_id, &scene_id, 2).await;

    // The retry the operator would naturally make. It must hand back the
    // banked review, not launch rounds 3 and 4.
    let second = svc.run_dual_persona_review(request()).await.unwrap();
    assert_eq!(
        second.rounds_completed, 2,
        "retry must collect the 2 banked rounds, not top up to 4"
    );
    assert_eq!(second.status, "current");
    assert_eq!(second.review_rounds.len(), 2);

    let row = svc
        .repository
        .get_dual_persona_review(&branch_id, &scene_id)
        .await
        .unwrap()
        .expect("row");
    assert_eq!(row.rounds_completed, 2, "the persisted row must not grow");
}

/// Two callers racing for the same scene must share one job — the model is
/// the expensive resource and a duplicate run doubles its cost for nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_review_calls_join_one_job() {
    use spindle_core::models::RunDualPersonaReviewInput;

    let (_tmp, svc, project_id, scene_id) =
        slow_review_fixture(std::time::Duration::from_millis(200)).await;
    let branch_id = svc
        .repository
        .get_active_branch(&project_id)
        .await
        .unwrap()
        .id;
    let request = || RunDualPersonaReviewInput {
        project_id: project_id.clone(),
        branch_id: None,
        scene_id: scene_id.clone(),
        rounds: Some(1),
    };

    let (a, b) = tokio::join!(
        svc.run_dual_persona_review(request()),
        svc.run_dual_persona_review(request())
    );
    a.unwrap();
    b.unwrap();

    await_review_rounds(&svc, &branch_id, &scene_id, 1).await;
    let row = svc
        .repository
        .get_dual_persona_review(&branch_id, &scene_id)
        .await
        .unwrap()
        .expect("row");
    assert_eq!(
        row.rounds_completed, 1,
        "two racing callers must produce ONE round, not two"
    );
}

/// Seed a project plus an explicit-capable `draft` receipt for the
/// receipt-claim tests.
async fn explicit_receipt_claim_fixture() -> (TempDir, SqliteSpindleService, String, String) {
    let (tmp, svc) = fresh_service_local().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "receipt-claim".to_string(),
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
        .register_generation_receipt("draft", Some("explicit"), "explicit-agent", "seed output")
        .await
        .unwrap();
    {
        let mut receipts = svc.generation_receipts.write().unwrap();
        receipts
            .get_mut(&receipt.id)
            .expect("seeded receipt")
            .explicit_capable_agent = true;
    }
    (tmp, svc, project.project_id, receipt.id)
}

fn explicit_save(project_id: &str, scene_order: i32, generation_id: &str) -> SaveSceneDraftInput {
    SaveSceneDraftInput {
        project_id: project_id.to_string(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order,
        full_text: "Her breath caught as he took her, slow and deliberate.".to_string(),
        summary: format!("Explicit scene {scene_order}."),
        content_rating: spindle_core::models::ContentRating::Explicit,
        tone: None,
        generation_id: Some(generation_id.to_string()),
        source_path: None,
        ..Default::default()
    }
}

// ── Bug 4c: generation receipts persist in the DB with a TTL ─────────────

/// Open a second, INDEPENDENT service instance over the same DB file — the
/// cross-process simulation. A fresh `SqlitePool` + `Repository` +
/// `SqliteSpindleService` shares no in-memory receipt cache with the first,
/// so anything a `verified_*` lookup resolves must have come from the DB.
async fn service_over_same_db(
    db_path: &std::path::Path,
    data_dir: &std::path::Path,
) -> SqliteSpindleService {
    use crate::ai::ModelRouter;
    let pool = SqlitePool::open(db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool, data_dir.to_path_buf(), ModelRouter::local_only());
    SqliteSpindleService::new(repo)
}

/// Shared fixture for the previous-scene-tail (T-103) tests: a project with
/// one protagonist and one location so scenes can be saved and context
/// resolved. Returns (tmp guard, service, project_id, character_id,
/// location_id).
async fn previous_scene_tail_fixture() -> (TempDir, SqliteSpindleService, String, String, String) {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CreateCharacterInput, CreateLocationInput, WorldStateInput,
    };

    let (tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "PrevTail".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Scenes hand off cleanly.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let character = svc
        .create_character(CreateCharacterInput {
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
                status: None,
                notes: None,
                source_summary: None,
            }),
        })
        .await
        .unwrap();
    let location = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A blackened wall holding back the dark.".into(),
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
    (
        tmp,
        svc,
        proj.project_id,
        character.character_id,
        location.location_id,
    )
}

/// Save an explicit scene 1 and plan scene 2 at `planned_rating`, then
/// resolve scene 2's context. Shared by the cross-rating elision tests so
/// each only asserts on the tail it gets back.
async fn explicit_neighbor_tail_for_planned_scene_two(
    planned_rating: Option<spindle_core::models::ContentRating>,
) -> Option<spindle_core::models::PreviousSceneTail> {
    use spindle_core::models::{
        ContentRating, ContextFormat, GetSceneContextInput, PlanChapterInput,
        PlanChapterSceneInput, SaveSceneDraftInput,
    };

    let (_tmp, svc, project_id, character_id, location_id) = previous_scene_tail_fixture().await;

    // Scene 1 is explicit and its closing prose is exactly what must not
    // reach a non-explicit drafting agent.
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
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: EXPLICIT_NEIGHBOR_PROSE.into(),
        summary: "Mara and the smith finally give in.".into(),
        content_rating: ContentRating::Explicit,
        tone: None,
        generation_id: Some(receipt.id.clone()),
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // The plan is the only place scene 2's rating exists — it has not been
    // drafted yet, which is exactly the state a drafting agent asks for
    // context in.
    if let Some(rating) = planned_rating {
        svc.plan_chapter(PlanChapterInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            pov_character_id: Some(character_id.clone()),
            synopsis: "Aftermath at the gate.".into(),
            target_theme_ids: Vec::new(),
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes: vec![
                PlanChapterSceneInput {
                    scene_order: 1,
                    summary: "Mara and the smith finally give in.".into(),
                    beat_structure: Vec::new(),
                    character_ids: vec![character_id.clone()],
                    location_id: Some(location_id.clone()),
                    content_rating: Some(ContentRating::Explicit),
                    purpose: "Consummate the arc.".into(),
                    research_required: None,
                    research_tags: Vec::new(),
                    explicit_query: None,
                },
                PlanChapterSceneInput {
                    scene_order: 2,
                    summary: "Dawn muster on the wall.".into(),
                    beat_structure: Vec::new(),
                    character_ids: vec![character_id.clone()],
                    location_id: Some(location_id.clone()),
                    content_rating: Some(rating),
                    purpose: "Return to the siege plot.".into(),
                    research_required: None,
                    research_tags: Vec::new(),
                    explicit_query: None,
                },
            ],
        })
        .await
        .unwrap();
    }

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    ctx.novel.previous_scene_tail
}

/// Live-run bug fixture: book 1 chapter 4 has a three-scene spine, and
/// `generate_alternatives` mints TWO unresolved alternatives over scene
/// order 2. On the broken build `list_chapter_scenes` returned five rows
/// (three of them at order 2), which led an operator to position-delete
/// the REAL scene 2 while cleaning up phantoms.
async fn spine_chapter_with_unresolved_alternatives() -> (
    TempDir,
    SqliteSpindleService,
    CreateProjectOutput,
    Vec<String>,
    Vec<String>,
) {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, ContentRating,
        CreateCharacterInput, CreateLocationInput, GenerateAlternativesInput, SaveSceneDraftInput,
        WorldStateInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "SpineContract".into(),
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
    for chapter_number in 2..=4 {
        svc.create_chapter(spindle_core::models::CreateChapterInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(chapter_number),
            title: None,
        })
        .await
        .unwrap();
    }
    let mut spine_ids = Vec::new();
    for (scene_order, body) in [
        (1, "Mara reached the gate at dusk."),
        (2, "The gate opened and the wardens stepped aside."),
        (3, "Mara crossed the threshold into the dark."),
    ] {
        let out = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 4,
                chapter_id: None,
                scene_order,
                full_text: body.into(),
                summary: format!("scene {scene_order}"),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
                ..Default::default()
            })
            .await
            .unwrap();
        spine_ids.push(out.scene_id);
    }

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
    let alts = svc
        .generate_alternatives(GenerateAlternativesInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 4,
            scene_order: 2,
            character_ids: vec![mara.character_id],
            location_id: gate.location_id,
            alternatives: Some(2),
            variation_strategy: "approach".into(),
        })
        .await
        .unwrap();
    let alt_ids = alts
        .alternatives
        .iter()
        .map(|alt| alt.scene_id.clone())
        .collect::<Vec<_>>();
    (_tmp, svc, proj, spine_ids, alt_ids)
}

/// Ad-hoc RAII guard that scopes `SPINDLE_DATA_DIR` to a fresh temp dir.
/// Restored on drop so other tests aren't affected.
struct ScopedDataDir {
    previous: Option<std::ffi::OsString>,
    _tmp: TempDir,
}

impl ScopedDataDir {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let previous = std::env::var_os("SPINDLE_DATA_DIR");
        // SAFETY: only one import_manuscript test sets this; tests are
        // run serially per-thread by tokio's default current-thread
        // executor, and the guard restores on drop.
        unsafe { std::env::set_var("SPINDLE_DATA_DIR", tmp.path()) };
        Self {
            previous,
            _tmp: tmp,
        }
    }
}

impl Drop for ScopedDataDir {
    fn drop(&mut self) {
        unsafe {
            if let Some(prev) = self.previous.take() {
                std::env::set_var("SPINDLE_DATA_DIR", prev);
            } else {
                std::env::remove_var("SPINDLE_DATA_DIR");
            }
        }
    }
}

/// Helper: project + opening scene + an Economy + a named price fact scoped
/// to that economy, plus a location to draft against. Returns the ids needed
/// to call `get_scene_context`.
async fn project_with_economy_price_fact(
    svc: &SqliteSpindleService,
    name: &str,
) -> (String, String) {
    use spindle_core::models::{
        CanonicalFactScope, ContentRating, CreateEconomyInput, CreateLocationInput,
        RegisterCanonicalFactInput, SaveSceneDraftInput,
    };

    let proj = svc
        .create_project(CreateProjectInput {
            name: name.into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "economies stay consistent".into(),
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
            full_text: "Merchants haggled over bread.".into(),
            summary: "market".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let economy = svc
        .create_economy(CreateEconomyInput {
            project_id: proj.project_id.clone(),
            name: "River Mark".into(),
            realm: None,
            summary: "Grain-based barter with iron scarcity.".into(),
            scarce_resources: vec!["iron".into()],
            trade_goods: vec!["grain".into()],
            currency: Some("silver marks".into()),
            notes: Vec::new(),
        })
        .await
        .unwrap();
    // A recurring named price, registered as a typed numeric canonical fact
    // scoped to the economy subject (the "activate price facts" recipe).
    svc.register_canonical_fact(RegisterCanonicalFactInput {
        project_id: proj.project_id.clone(),
        scene_id: Some(scene.scene_id.clone()),
        book_number: 1,
        chapter_number: 1,
        fact_type: Some("typed_fact".into()),
        key: Some("economy.bread_price".into()),
        value: Some("5 silver".into()),
        context: None,
        subject_table: Some("economy".into()),
        subject_id: Some(economy.economy_id.clone()),
        predicate: Some("bread_price".into()),
        value_kind: Some("number".into()),
        value_text: None,
        value_number: Some(5.0),
        value_unit: Some("silver".into()),
        value_json: None,
        aliases: vec!["loaf".into(), "bread".into()],
        scope: Some(CanonicalFactScope::Evolving),
        valid_from: None,
        valid_until: None,
        legacy_untyped: Some(false),
        supersedes_fact_id: None,
        secrecy: None,
    })
    .await
    .unwrap();
    let location = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Market".into(),
            kind: "city".into(),
            realm: None,
            summary: "A market square.".into(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();
    (proj.project_id, location.location_id)
}

/// Helper: project + scene + a wealth scheme (destitute<comfortable<wealthy,
/// max_band_jump 1). Returns the project id.
async fn project_with_wealth_scheme(svc: &SqliteSpindleService, name: &str) -> String {
    use spindle_core::models::{QuantityBand, QuantityScheme, SetProjectQuantitySchemeInput};

    let (project, _scene_id, _text) = project_with_scene(svc, name).await;
    svc.set_project_quantity_scheme(SetProjectQuantitySchemeInput {
        project_id: project.project_id.clone(),
        scheme: QuantityScheme {
            measure: "wealth".into(),
            denominations: Vec::new(),
            bands: vec![
                QuantityBand {
                    name: "destitute".into(),
                    lower_bound: None,
                },
                QuantityBand {
                    name: "comfortable".into(),
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
    project.project_id
}

fn wealth_commit(
    project_id: &str,
    chapter: i32,
    band: &str,
    reason: Option<&str>,
) -> spindle_core::models::CommitQuantityStateInput {
    spindle_core::models::CommitQuantityStateInput {
        project_id: project_id.to_string(),
        subject_table: "character".into(),
        subject_id: "character:hero".into(),
        measure: "wealth".into(),
        book_number: 1,
        chapter_number: chapter,
        scene_order: 1,
        scene_id: None,
        state: spindle_core::models::QuantityState {
            band: Some(band.to_string()),
            change_reason: reason.map(str::to_string),
            ..Default::default()
        },
    }
}

fn quantity_drift_input(project_id: &str) -> spindle_core::models::CheckConsistencyInput {
    spindle_core::models::CheckConsistencyInput {
        deep_scan_offset: None,
        project_id: project_id.to_string(),
        scope: spindle_core::models::ConsistencyScopeInput::full(),
        checks: vec!["quantity_drift".into()],
        severity_filter: Vec::new(),
        deep_check: Some(false),
        subjects: Vec::new(),
        format: None,
        budget_tokens: None,
    }
}

async fn seed_character(svc: &SqliteSpindleService, project_id: &str, name: &str) -> String {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, CreateCharacterInput,
    };
    svc.create_character(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: project_id.to_string(),
        name: name.into(),
        summary: format!("{name} summary"),
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
    })
    .await
    .unwrap()
    .character_id
}

/// Shared fixture for the pacing feed-forward (T-109) tests: a project with
/// one protagonist, one location, and a book-1 pacing curve so scenes can be
/// saved, annotated, and scene context resolved. Chapters and intensity
/// annotations are seeded by the caller. Returns
/// (tmp guard, service, project_id, character_id, location_id).
async fn pacing_feed_forward_fixture() -> (TempDir, SqliteSpindleService, String, String, String) {
    use spindle_core::models::CreatePacingCurveInput;
    use std::collections::BTreeMap;

    let (tmp, svc, project_id, character_id, location_id) = previous_scene_tail_fixture().await;
    svc.create_pacing_curve(CreatePacingCurveInput {
        project_id: project_id.clone(),
        book_number: 1,
        act_breakpoints: BTreeMap::new(),
        scene_type_density: BTreeMap::new(),
        intensity_points: Vec::new(),
    })
    .await
    .unwrap();
    (tmp, svc, project_id, character_id, location_id)
}

/// Seed chapter `chapter` (book 1) with a single scene, optionally annotated
/// with `intensity`. `None` intensity leaves the annotation intensity NULL.
async fn seed_annotated_chapter(
    svc: &SqliteSpindleService,
    project_id: &str,
    chapter: i32,
    intensity: Option<f64>,
) {
    use spindle_core::models::{
        AnnotateSceneBeatsInput, ContentRating, CreateChapterInput, SaveSceneDraftInput,
    };

    svc.create_chapter(CreateChapterInput {
        project_id: project_id.to_string(),
        book_number: Some(1),
        book_id: None,
        chapter_number: Some(chapter),
        title: None,
    })
    .await
    .unwrap();
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: chapter,
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
    svc.annotate_scene_beats(AnnotateSceneBeatsInput {
        project_id: project_id.to_string(),
        scene_id: scene.scene_id.clone(),
        beats: Vec::new(),
        motif_ids: Vec::new(),
        theme_ids: Vec::new(),
        conflict_ids: Vec::new(),
        intensity,
    })
    .await
    .unwrap();
}

async fn scene_context_for_chapter(
    svc: &SqliteSpindleService,
    project_id: &str,
    character_id: &str,
    location_id: &str,
    chapter: i32,
) -> spindle_core::models::SceneContextOutput {
    use spindle_core::models::{ContextFormat, GetSceneContextInput};

    svc.get_scene_context(GetSceneContextInput {
        project_id: project_id.to_string(),
        book_number: 1,
        chapter_number: chapter,
        chapter_id: None,
        scene_order: 1,
        character_ids: vec![character_id.to_string()],
        max_character_count: None,
        location_id: location_id.to_string(),
        format: Some(ContextFormat::Json),
        budget_tokens: Some(8000),
        token_budget: None,
        sections: None,
    })
    .await
    .unwrap()
}

/// Seed a chapter with two scenes and return their ids, for the
/// scene_id-addressed remediation tests.
async fn two_scene_chapter(svc: &SqliteSpindleService, project_id: &str) -> (String, String) {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};
    let mut ids = Vec::new();
    for (scene_order, body) in [
        (1, "First scene prose that is long enough."),
        (2, "Second scene prose that is long enough."),
    ] {
        let out = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project_id.to_string(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order,
                full_text: body.into(),
                summary: format!("scene {scene_order}"),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
                ..Default::default()
            })
            .await
            .unwrap();
        ids.push(out.scene_id);
    }
    (ids.remove(0), ids.remove(0))
}

// -------------------------------------------------------------------------
// T-102: chapter-plan targeted threads hydrated into scene context /
// chapter briefing (active_threads).
// -------------------------------------------------------------------------

/// Shared fixture: project + one character + one location. Returns
/// (project_id, character_id, location_id).
async fn active_threads_base(svc: &SqliteSpindleService, name: &str) -> (String, String, String) {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CreateCharacterInput, CreateLocationInput, WorldStateInput,
    };
    let proj = svc
        .create_project(CreateProjectInput {
            name: name.into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Threads must advance.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let character = svc
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
            initial_state: Some(CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: None,
                status: None,
                notes: None,
                source_summary: None,
            }),
        })
        .await
        .unwrap();
    let location = svc
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
    (
        proj.project_id,
        character.character_id,
        location.location_id,
    )
}

// =========================================================================
// T-104 — motif_usage_audit + theme_placement_audit
// =========================================================================

/// Shared fixture: fresh project with a scene drafted at (book 1, chapter
/// `chapter`, scene 1). Returns (tempdir, service, project_id, scene_id).
async fn motif_theme_fixture_scene(
    chapter: i32,
) -> (tempfile::TempDir, SqliteSpindleService, String, String) {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};

    let (tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "mtaudit".into(),
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
            chapter_number: chapter,
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
    (tmp, svc, proj.project_id, scene.scene_id)
}

// =========================================================================
// V0022 — plot_line_convergence_audit / arc_milestone_audit /
// conflict_escalation_audit
// =========================================================================

/// Fresh project + a scene at (book 1, chapter `chapter`, scene 1).
/// Returns (tempdir, service, project_id, scene_id, character_id).
async fn convergence_fixture_scene(
    chapter: i32,
) -> (
    tempfile::TempDir,
    SqliteSpindleService,
    String,
    String,
    String,
) {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, ContentRating,
        CreateCharacterInput, SaveSceneDraftInput,
    };

    let (tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "convaudit".into(),
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
            chapter_number: chapter,
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
    let character = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Protagonist".into(),
            summary: "the lead".into(),
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
    (
        tmp,
        svc,
        proj.project_id,
        scene.scene_id,
        character.character_id,
    )
}

fn placement(book: i32, chapter: i32) -> spindle_core::models::StoryPlacement {
    spindle_core::models::StoryPlacement {
        book_number: book,
        chapter_number: chapter,
        scene_order: None,
        note: None,
    }
}

// ── arc_milestone_audit ──────────────────────────────────────────────

async fn seed_arc_with_milestone(
    svc: &SqliteSpindleService,
    project_id: &str,
    character_id: &str,
    milestone_placement: Option<spindle_core::models::StoryPlacement>,
    reached_at: Option<spindle_core::models::StoryPlacement>,
) -> String {
    use spindle_core::models::{CharacterArcMilestone, CreateCharacterArcInput};
    let arc = svc
        .create_character_arc(CreateCharacterArcInput {
            project_id: project_id.to_string(),
            character_id: character_id.to_string(),
            arc_type: "positive".into(),
            starting_state: "naive".into(),
            ending_state: "wise".into(),
            milestones: vec![CharacterArcMilestone {
                label: "the reckoning".into(),
                placement: milestone_placement,
                description: "faces the truth".into(),
                unlocks: Vec::new(),
                reached_at,
            }],
            thematic_purpose: "growth".into(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();
    arc.character_arc_id
}

async fn seed_scene_at(svc: &SqliteSpindleService, project_id: &str, chapter: i32) {
    use spindle_core::models::{ContentRating, CreateChapterInput, SaveSceneDraftInput};
    svc.create_chapter(CreateChapterInput {
        project_id: project_id.to_string(),
        book_number: Some(1),
        book_id: None,
        chapter_number: Some(chapter),
        title: None,
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.to_string(),
        book_number: 1,
        chapter_number: chapter,
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
}

// ── conflict_escalation_audit ────────────────────────────────────────

async fn seed_conflict_with_markers(
    svc: &SqliteSpindleService,
    project_id: &str,
    markers: serde_json::Value,
) -> String {
    use spindle_core::models::{CreateConflictInput, UpdateEntityInput};
    let conflict = svc
        .create_conflict(CreateConflictInput {
            project_id: project_id.to_string(),
            name: "the rivalry".into(),
            conflict_type: "interpersonal".into(),
            stakes: "the throne".into(),
            escalation_stages: vec!["tension".into(), "clash".into(), "war".into()],
            expected_total_cycles: None,
            try_fail_cycles: Vec::new(),
            stated_consequences: Vec::new(),
        })
        .await
        .unwrap();
    if !markers.is_null() {
        svc.update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "conflict".into(),
            entity_id: conflict.conflict_id.clone(),
            changes: serde_json::json!({ "escalation_demonstrated": markers }),
        })
        .await
        .unwrap();
    }
    conflict.conflict_id
}

// ── T-106: promise-payoff deep-check candidate detection ──────────────
// Model-backed Tier-2 check. Proposes (never writes) that an unresolved
// promise the prose already paid off should be confirmed via
// `update_promise_status`. Gated to `deep_check == true`. All tests are
// hermetic and drive `model_router.complete` through `fresh_service_local`
// (or a config-pinned unroutable HTTP route for the honest-skip path).

/// Seed a project + one scoped scene whose prose carries `prose`, and an
/// active (planted) promise with a declared payoff. Returns
/// `(project_id, promise_id)`.
async fn seed_promise_payoff_project(
    svc: &SqliteSpindleService,
    name: &str,
    prose: &str,
) -> (String, String) {
    use spindle_core::models::{
        ContentRating, CreateNarrativePromiseInput, SaveSceneDraftInput, StoryPlacement,
    };
    let proj = svc
        .create_project(CreateProjectInput {
            name: name.into(),
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
        full_text: prose.into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    let promise = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: proj.project_id.clone(),
            promise_type: "setup".into(),
            description: "the buried crown will be reclaimed".into(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(0),
                note: None,
            },
            planned_payoff: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 2,
                scene_order: Some(0),
                note: None,
            }),
            notes: Vec::new(),
        })
        .await
        .unwrap();
    (proj.project_id, promise.narrative_promise_id)
}

fn promise_payoff_input(
    project_id: &str,
    deep: bool,
) -> spindle_core::models::CheckConsistencyInput {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};
    CheckConsistencyInput {
        deep_scan_offset: None,
        project_id: project_id.to_string(),
        scope: ConsistencyScopeInput::full(),
        checks: vec!["promise_payoff_detection".to_string()],
        severity_filter: Vec::new(),
        deep_check: Some(deep),
        subjects: Vec::new(),
        format: None,
        budget_tokens: None,
    }
}

// ── Canon miner (mine_scene_canon — evolution §3.1, ADR 0001) ────────────

/// Overwrite a saved scene's prose in place (via the upserting draft path,
/// which keys on book/chapter/scene_order) so a test can seed a specific
/// mining prompt without threading a full draft flow.
async fn set_scene_prose(svc: &SqliteSpindleService, scene_id: &str, prose: &str) {
    let scene = svc.repository().get_scene(scene_id).await.unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: scene.project_id.clone(),
        book_number: scene.book_number,
        chapter_number: scene.chapter_number,
        chapter_id: None,
        scene_order: scene.scene_order,
        full_text: prose.to_string(),
        summary: scene.summary.clone(),
        content_rating: match scene.content_rating.as_str() {
            "teen" => spindle_core::models::ContentRating::Teen,
            "mature" => spindle_core::models::ContentRating::Mature,
            "explicit" => spindle_core::models::ContentRating::Explicit,
            _ => spindle_core::models::ContentRating::General,
        },
        tone: scene.tone.clone(),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
}

/// Create a project and plan `chapter_number` with the given synopsis
/// (carrying whatever replan sentinel the test wants), returning the project.
async fn project_with_target_plan(
    svc: &SqliteSpindleService,
    name: &str,
    chapter_number: i32,
    synopsis: &str,
) -> CreateProjectOutput {
    use spindle_core::models::{PlanChapterInput, PlanChapterSceneInput};
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
    svc.plan_chapter(PlanChapterInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number,
        pov_character_id: None,
        synopsis: synopsis.to_string(),
        target_theme_ids: Vec::new(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "planned opening".to_string(),
            beat_structure: Vec::new(),
            character_ids: Vec::new(),
            location_id: None,
            content_rating: None,
            purpose: "establish".to_string(),
            research_required: None,
            research_tags: Vec::new(),
            explicit_query: None,
        }],
    })
    .await
    .unwrap();
    project
}

/// Save a summary for the source chapter so the source chapter is eligible.
async fn save_source_summary(svc: &SqliteSpindleService, project_id: &str, chapter: i32) {
    svc.save_summary(SaveSummaryInput {
        project_id: project_id.to_string(),
        book_number: 1,
        chapter_number: chapter,
        entity_type: None,
        entity_id: None,
        summary: format!("Chapter {chapter} resolved the siege early."),
        key_events: vec!["The gate fell".to_string()],
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    })
    .await
    .unwrap();
}

/// Panic while holding an `RwLock` write guard on a second thread, so the
/// lock stays poisoned for every later acquisition. Each test poisons a
/// fresh service, so the sticky poison cannot leak across tests.
fn poison_rwlock<T: Send + Sync>(lock: std::sync::Arc<std::sync::RwLock<T>>) {
    std::thread::scope(|s| {
        let handle = s.spawn(|| {
            let _guard = lock.write().unwrap();
            panic!("intentional lock poison for test");
        });
        assert!(handle.join().is_err(), "poisoning thread must panic");
    });
}

fn poison_mutex<T: Send + 'static>(lock: std::sync::Arc<std::sync::Mutex<T>>) {
    let handle = std::thread::spawn(move || {
        let _guard = lock.lock().unwrap();
        panic!("intentional lock poison for test");
    });
    assert!(handle.join().is_err(), "poisoning thread must panic");
}

#[path = "branch_snapshots/tests.rs"]
mod branch_snapshots;
#[path = "canon_events/tests.rs"]
mod canon_events;
#[path = "canon_mining/tests.rs"]
mod canon_mining;
#[path = "canon_ratification/tests.rs"]
mod canon_ratification;
#[path = "consistency/cache/tests.rs"]
mod consistency_cache;
#[path = "consistency/contracts/tests.rs"]
mod consistency_contracts;
#[path = "consistency/narrative/tests.rs"]
mod consistency_narrative;
#[path = "consistency/quantity/tests.rs"]
mod consistency_quantity;
#[path = "consistency/temporal/tests.rs"]
mod consistency_temporal;
#[path = "context/tests.rs"]
mod context;
#[path = "drafting/tests.rs"]
mod drafting;
#[path = "entity_updates/tests.rs"]
mod entity_updates;
#[path = "export/tests.rs"]
mod export;
#[path = "generation/tests.rs"]
mod generation;
#[path = "imports/tests.rs"]
mod imports;
#[path = "plan_amendments/tests.rs"]
mod plan_amendments;
#[path = "reader_sim/tests.rs"]
mod reader_sim;
#[path = "research/tests.rs"]
mod research;
#[path = "reviews/tests.rs"]
mod reviews;
#[path = "scene_changes/tests.rs"]
mod scene_changes;
#[path = "scenes/tests.rs"]
mod scenes;
#[path = "source_bridge/tests.rs"]
mod source_bridge;

/// Prose used as the explicit neighbour in the elision tests. The assertion
/// below keys on this exact string never appearing in a non-explicit
/// scene's context.
const EXPLICIT_NEIGHBOR_PROSE: &str = "She pulled him down and took him inside her, \
         the forge heat nothing to the heat of his mouth on her breast as she came.";

const TELEPORT_PROSE: &str = "He woke in the grey morning and ate his bread at the \
         long table. The midnight harbour lay black and still around him as he stood on the quay.";
