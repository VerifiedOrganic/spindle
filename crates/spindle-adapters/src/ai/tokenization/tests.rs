use super::*;
use crate::ai::tests::spawn_http_capture_server;
use crate::tokenization::{ProbeMode, tests::fixture};

async fn router(endpoint: &str, config: &TokenizerConfig, dir: &Path) -> ModelRouter {
    let path = dir.join("agents.toml");
    let profile = toml::to_string(config).unwrap();
    let profile = profile.replace("[framing]", "[agents.tokenizer.framing]");
    let contents = format!(
        r#"[health_check]
enabled = false
[[agents]]
id = "fixture-agent"
name = "Fixture"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "fixture"
max_context = 32768
ratings = ["general"]
[agents.tokenizer]
{profile}
[[routing]]
route = "draft"
agent = "fixture-agent"
system_prompt = "Test"
"#
    );
    std::fs::write(&path, contents).unwrap();
    let pool = crate::sqlite::SqlitePool::open(&dir.join("usage.db"))
        .await
        .unwrap();
    ModelRouter::for_tokenizer_tools(path.to_str().unwrap(), Some(pool)).unwrap()
}
fn response(input: usize, output: usize) -> (u16, String) {
    (200,serde_json::json!({"model":"fixture","done":true,"prompt_eval_count":input,"eval_count":output,"message":{"content":if output==0 {""} else {"DISCARD_ME"}}}).to_string())
}
#[tokio::test]
async fn calibration_is_cached_persisted_and_counting_is_offline() {
    let (dir, mut config) = fixture();
    config.probe_mode = ProbeMode::CloudOneToken;
    let (endpoint, captured, server) =
        spawn_http_capture_server([9, 10, 8, 646].map(|n| response(n, 1)).to_vec());
    let router = router(&endpoint, &config, dir.path()).await;
    let cert = router
        .calibrate_tokenizer("draft", "general")
        .await
        .unwrap();
    assert_eq!(server.join().unwrap(), 4);
    assert_eq!(cert.samples[0].output_tokens, 1);
    router
        .calibrate_tokenizer("draft", "general")
        .await
        .unwrap();
    let reopened =
        ModelRouter::for_tokenizer_tools(dir.path().join("agents.toml").to_str().unwrap(), None)
            .unwrap();
    assert_eq!(
        reopened
            .count_tokenizer_messages(
                "draft",
                "general",
                &DraftMessages {
                    system: "hello".into(),
                    user: "snow".into()
                }
            )
            .unwrap(),
        5
    );
    assert!(
        reopened
            .count_tokenizer_messages(
                "draft",
                "mature",
                &DraftMessages {
                    system: "hello".into(),
                    user: "snow".into()
                }
            )
            .is_err()
    );
    for raw in captured.lock().unwrap().iter() {
        let body: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert_eq!(body["options"]["num_predict"], 1);
        assert_eq!(body["truncate"], false);
    }
    let pool = router.usage_pool.as_ref().unwrap();
    let count: i64 = pool.read(|conn| conn.query_row("SELECT count(*) FROM model_call WHERE prompt_template_id='token_count.ollama_probe' AND json_extract(payload,'$.model_name')='fixture'", [], |row| row.get(0))).await.unwrap();
    assert_eq!(count, 4);
    let cache = std::fs::read_dir(&config.cache_dir).unwrap();
    for entry in cache {
        assert!(
            !std::fs::read_to_string(entry.unwrap().path())
                .unwrap()
                .contains("DISCARD_ME")
        );
    }
}
#[tokio::test]
async fn missing_counts_fail_closed_and_uncertain_retry_never_dispatches_again() {
    let (dir, config) = fixture();
    let (endpoint, _, server) = spawn_http_capture_server(vec![(
        200,
        r#"{"model":"fixture","done":true,"eval_count":0}"#.into(),
    )]);
    let router = router(&endpoint, &config, dir.path()).await;
    assert!(
        router
            .calibrate_tokenizer("draft", "general")
            .await
            .unwrap_err()
            .to_string()
            .contains("prompt_eval_count")
    );
    assert_eq!(server.join().unwrap(), 1);
    assert!(
        router
            .calibrate_tokenizer("draft", "general")
            .await
            .is_err()
    );
    assert!(!config.calibration_file.exists());
}
#[tokio::test]
async fn zero_output_requires_provider_proof_and_no_generated_text() {
    for invalid in [response(0,0),response(9,1),(200,serde_json::json!({"model":"fixture","done":true,"prompt_eval_count":9,"eval_count":0,"message":{"content":"unexpected"}}).to_string())] {
        let (dir,config)=fixture();
        let (endpoint,_,server)=spawn_http_capture_server(vec![invalid]);
        let router=router(&endpoint,&config,dir.path()).await;
        assert!(router.calibrate_tokenizer("draft","general").await.is_err());
        assert_eq!(server.join().unwrap(),1);
        assert!(!config.calibration_file.exists());
    }
}

#[tokio::test]
async fn concurrent_exact_messages_share_one_probe_and_restart_reuses_it() {
    let (dir, mut config) = fixture();
    config.probe_mode = ProbeMode::CloudOneToken;
    let (endpoint, _, server) = spawn_http_capture_server(vec![response(5, 1)]);
    let router = router(&endpoint, &config, dir.path()).await;
    let runtime = router.runtime.read().unwrap().clone();
    let route = resolve_cleared_route(&runtime, "draft", Some("general")).unwrap();
    let scope = router.counter_scope(&runtime, route).unwrap();
    let messages = DraftMessages {
        system: "hello".into(),
        user: "snow".into(),
    };
    let (a, b) = tokio::join!(
        router.probe_count(&scope, &messages, Some("general"), None),
        router.probe_count(&scope, &messages, Some("general"), None)
    );
    assert_eq!(a.unwrap().model_call_id, b.unwrap().model_call_id);
    assert_eq!(server.join().unwrap(), 1);
    let fresh = ModelRouter::for_tokenizer_tools(
        dir.path().join("agents.toml").to_str().unwrap(),
        router.usage_pool.clone(),
    )
    .unwrap();
    assert_eq!(
        fresh
            .probe_count(&scope, &messages, Some("general"), None)
            .await
            .unwrap()
            .input_tokens,
        5
    );
    let changed = DraftMessages {
        system: "hello ".into(),
        user: "snow".into(),
    };
    assert_ne!(
        messages_hash(&messages).unwrap(),
        messages_hash(&changed).unwrap()
    );
}

#[tokio::test]
async fn cancellation_after_dispatch_keeps_recovery_marker() {
    use std::io::Read;
    let (dir, config) = fixture();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let router = router(&endpoint, &config, dir.path()).await;
    let runtime = router.runtime.read().unwrap().clone();
    let route = resolve_cleared_route(&runtime, "draft", Some("general")).unwrap();
    let scope = router.counter_scope(&runtime, route).unwrap();
    let messages = calibration_messages().remove(0);
    let (received_tx, received_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0; 8192];
        assert!(stream.read(&mut buf).unwrap() > 0);
        received_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    let task_router = router.clone();
    let task_scope = scope.clone();
    let task_messages = messages.clone();
    let task = tokio::spawn(async move {
        task_router
            .probe_count(&task_scope, &task_messages, Some("general"), None)
            .await
    });
    received_rx.await.unwrap();
    task.abort();
    let _ = task.await;
    let pending: i64 = router.usage_pool.as_ref().unwrap().read(|conn|
        conn.query_row("SELECT count(*) FROM model_call WHERE prompt_template_id='token_count.ollama_probe' AND json_extract(payload,'$.outcome')='pending' AND json_extract(payload,'$.usage') IS NULL", [], |row| row.get(0))
    ).await.unwrap();
    assert_eq!(pending, 1);
    assert!(
        router
            .probe_count(&scope, &messages, Some("general"), None)
            .await
            .is_err()
    );
    release_tx.send(()).unwrap();
    server.join().unwrap();
    assert!(!config.calibration_file.exists());
}

#[tokio::test]
async fn configured_packet_counter_checks_final_messages_before_prose_dispatch() {
    use crate::drafting::{DraftTokenizer, assemble_packet_with_system};
    use spindle_core::drafting::EditorialPolicy;
    for (drift, lost_context, injected) in [
        (0, false, false),
        (0, false, true),
        (1, false, false),
        (0, true, false),
    ] {
        let (dir, mut config) = fixture();
        config.probe_mode = ProbeMode::CloudOneToken;
        let encoder = PinnedTokenizer::load("fixture", config.clone()).unwrap();
        let samples = calibration_messages()
            .iter()
            .zip([9, 10, 8, 646])
            .map(|(m, n)| ProbeReceipt {
                model: "fixture".into(),
                messages_hash: messages_hash(m).unwrap(),
                input_tokens: n,
                output_tokens: 1,
                model_call_id: format!("fixture-{n}"),
            })
            .collect();
        let certificate = encoder.certificate("test", samples).unwrap();
        let encoder = encoder.verify(&certificate, "test").unwrap();
        let packet = crate::drafting::tests::fixture();
        let budget = crate::drafting::tests::budget();
        let prepared =
            assemble_packet_with_system(packet.clone(), &budget, Some(&encoder), "Test").unwrap();
        let count = encoder.count_messages(&prepared.messages).unwrap();
        let mut responses = [9, 10, 8, 646].map(|n| response(n, 1)).to_vec();
        responses.push(response(count + drift, 1));
        if drift == 0 {
            responses.push((200,serde_json::json!({"model":"fixture","done":true,"done_reason":"stop","prompt_eval_count":count-usize::from(lost_context),"eval_count":4,"message":{"content":"Exact draft.\r\n"}}).to_string()));
        }
        let (endpoint, captured, server) = spawn_http_capture_server(responses);
        let router = router(&endpoint, &config, dir.path()).await;
        router
            .calibrate_tokenizer("draft", "general")
            .await
            .unwrap();
        let injected = injected
            .then_some(&crate::drafting::tests::VerifiedFixtureTokenizer as &dyn DraftTokenizer);
        let result = router
            .draft_packet(EditorialPolicy::CritiqueV1, packet, &budget, injected)
            .await;
        if drift == 0 && !lost_context {
            let (actual, response) = result.unwrap();
            assert_eq!(actual.messages, prepared.messages);
            assert_eq!(response.output, "Exact draft.\r\n");
            assert_eq!(server.join().unwrap(), 6);
            let captured = captured.lock().unwrap();
            let probe: serde_json::Value = serde_json::from_str(&captured[4]).unwrap();
            let generation: serde_json::Value = serde_json::from_str(&captured[5]).unwrap();
            assert_eq!(probe["messages"], generation["messages"]);
            // The deployment's OpenAI default could be only 4096. Native prose
            // must override it with the admitted 32768 window, like the probe.
            assert!(budget.context_window > 4096);
            assert_eq!(generation["options"]["num_ctx"], 32768);
            assert_eq!(
                probe["options"]["num_ctx"],
                generation["options"]["num_ctx"]
            );
            assert_eq!(generation["truncate"], false);
            assert_eq!(generation["shift"], false);
            assert_eq!(probe["shift"], false);
            assert_eq!(response.model_name, "fixture-agent");
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("calibration mismatch")
            );
            assert_eq!(server.join().unwrap(), if lost_context { 6 } else { 5 });
        }
    }
}

#[tokio::test]
async fn native_packet_fails_closed_when_deployment_rejects_larger_window() {
    let (dir, config) = fixture();
    let (endpoint, captured, server) = spawn_http_capture_server(vec![(
        400,
        serde_json::json!({"error":"configured num_ctx 32768 exceeds deployment limit 4096"})
            .to_string(),
    )]);
    let router = router(&endpoint, &config, dir.path()).await;
    let runtime = router.runtime.read().unwrap().clone();
    let mut route = resolve_cleared_route(&runtime, "draft", Some("general"))
        .unwrap()
        .clone();
    route.max_tokens = Some(200);
    let scope = router.counter_scope(&runtime, &route).unwrap();
    let error = router
        .run_ollama_packet(
            &scope,
            &route,
            &DraftMessages {
                system: "hello".into(),
                user: "snow".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("HTTP 400"));
    assert_eq!(server.join().unwrap(), 1);
    let requests = captured.lock().unwrap();
    let body: serde_json::Value = serde_json::from_str(&requests[0]).unwrap();
    assert_eq!(body["options"]["num_ctx"], 32768);
    assert_eq!(body["truncate"], false);
    assert_eq!(body["shift"], false);
}

#[tokio::test]
async fn seed_and_eval_messages_share_calibrated_native_transport_and_reject_seed_mismatch() {
    use crate::drafting::{DraftTokenizer, assemble_packet_with_system};
    use spindle_core::drafting::EditorialPolicy;

    let (dir, mut config) = fixture();
    config.probe_mode = ProbeMode::CloudOneToken;
    let encoder = PinnedTokenizer::load("fixture", config.clone()).unwrap();
    let samples = calibration_messages()
        .iter()
        .zip([9, 10, 8, 646])
        .map(|(m, n)| ProbeReceipt {
            model: "fixture".into(),
            messages_hash: messages_hash(m).unwrap(),
            input_tokens: n,
            output_tokens: 1,
            model_call_id: format!("fixture-{n}"),
        })
        .collect();
    let certificate = encoder.certificate("test", samples).unwrap();
    let encoder = encoder.verify(&certificate, "test").unwrap();

    let mut packet = crate::drafting::tests::fixture();
    packet.project_id = "project:proj_eval".into();
    packet.branch_id = "bible_branch:proj_eval:main".into();
    let mut budget = crate::drafting::tests::budget();
    budget.generation_seed = Some(101);
    let prepared =
        assemble_packet_with_system(packet.clone(), &budget, Some(&encoder), "Test").unwrap();
    let packet_count = encoder.count_messages(&prepared.messages).unwrap();

    let legacy_messages = DraftMessages {
        system: "Legacy system".into(),
        user: "Legacy user prompt".into(),
    };
    let legacy_count = encoder.count_messages(&legacy_messages).unwrap();

    let mut responses = [9, 10, 8, 646].map(|n| response(n, 1)).to_vec();
    // 1. Packet probe + packet generation (seed 101)
    responses.push(response(packet_count, 1));
    responses.push((
        200,
        serde_json::json!({
            "model": "fixture",
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": packet_count,
            "eval_count": 6,
            "message": {"content": "Packet scene draft."}
        })
        .to_string(),
    ));
    // 2. Legacy baseline probe + legacy generation (seed 101)
    responses.push(response(legacy_count, 1));
    responses.push((
        200,
        serde_json::json!({
            "model": "fixture",
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": legacy_count,
            "eval_count": 7,
            "message": {"content": "Legacy scene draft."}
        })
        .to_string(),
    ));

    let (endpoint, captured, server) = spawn_http_capture_server(responses);
    let router = router(&endpoint, &config, dir.path()).await;
    router
        .calibrate_tokenizer("draft", "general")
        .await
        .unwrap();

    let pool = router.usage_pool.clone().unwrap();
    let repository = crate::sqlite::repository::Repository::with_model_router(
        pool,
        dir.path().join("data"),
        router.clone(),
    );
    let service = crate::sqlite::service::SqliteSpindleService::new(repository);
    service
        .ensure_eval_project_branch(&packet.project_id, &packet.branch_id)
        .await
        .unwrap();

    let artifact_path = dir.path().join("packet_seed_101.json");
    let artifact = service
        .draft_packet_to_artifact(
            EditorialPolicy::CritiqueV1,
            packet.clone(),
            &budget,
            None,
            &artifact_path,
        )
        .await
        .unwrap();
    assert_eq!(
        artifact
            .prepared_packet
            .as_ref()
            .unwrap()
            .packet
            .assembly
            .as_ref()
            .unwrap()
            .generation_seed,
        Some(101)
    );
    // Reopening with the same seed succeeds from disk without another HTTP call.
    let reloaded = service
        .draft_packet_to_artifact(
            EditorialPolicy::CritiqueV1,
            packet.clone(),
            &budget,
            None,
            &artifact_path,
        )
        .await
        .unwrap();
    assert_eq!(
        reloaded.receipt.generation_id,
        artifact.receipt.generation_id
    );
    // Reopening the same artifact path with a different seed fails closed.
    let mut different_seed_budget = budget.clone();
    different_seed_budget.generation_seed = Some(202);
    assert!(
        service
            .draft_packet_to_artifact(
                EditorialPolicy::CritiqueV1,
                packet,
                &different_seed_budget,
                None,
                &artifact_path,
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("seed mismatch")
    );

    let (eval_budget, settings) = router
        .eval_draft_budget_and_settings("draft", "general", Some(101))
        .unwrap();
    assert_eq!(settings["seed"], 101);
    assert_eq!(settings["transport"], "ollama_native_chat");
    assert_eq!(settings["context_window"], 32768);
    let mut legacy_budget = eval_budget;
    legacy_budget.output_reserve = budget.output_reserve;
    let (prompt_hash, counted_input, legacy_resp) = router
        .draft_eval_messages("draft", "general", &legacy_messages, &legacy_budget, None)
        .await
        .unwrap();
    assert_eq!(counted_input, legacy_count);
    assert_eq!(prompt_hash, messages_hash(&legacy_messages).unwrap());
    assert_eq!(legacy_resp.output, "Legacy scene draft.");
    assert_eq!(legacy_resp.provider_model.as_deref(), Some("fixture"));

    assert_eq!(server.join().unwrap(), 8);
    let captured = captured.lock().unwrap();
    let packet_gen: serde_json::Value = serde_json::from_str(&captured[5]).unwrap();
    let legacy_gen: serde_json::Value = serde_json::from_str(&captured[7]).unwrap();
    assert_eq!(packet_gen["options"]["seed"], 101);
    assert_eq!(legacy_gen["options"]["seed"], 101);
    assert_eq!(
        packet_gen["options"]["num_ctx"],
        legacy_gen["options"]["num_ctx"]
    );
    assert_eq!(packet_gen["truncate"], false);
    assert_eq!(legacy_gen["truncate"], false);
    assert_eq!(packet_gen["shift"], false);
    assert_eq!(legacy_gen["shift"], false);
}
