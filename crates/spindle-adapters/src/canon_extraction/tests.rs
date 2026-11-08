use super::*;
use spindle_core::models::StoryPlacement;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn binding(text: &str) -> ClaimBindingV1 {
    ClaimBindingV1 {
        schema_version: 1,
        project_id: "project:fixture".into(),
        branch_id: "branch:main".into(),
        scene_id: None,
        revision_id: "accepted:1".into(),
        accepted_text_sha256: exact_text_sha256(text),
        position: StoryPlacement {
            book_number: 1,
            chapter_number: 2,
            scene_order: Some(1),
            note: None,
        },
        extraction_run_id: "extract:1".into(),
        model: "pending".into(),
        agent_id: None,
        model_call_id: None,
        prompt_version: EXTRACT_PROMPT_VERSION.into(),
    }
}
fn context() -> CanonExtractionContext {
    CanonExtractionContext {
        rating: ContentRating::General,
        packet_hash: "frozen-packet-hash".into(),
        known_entities: [("character:veyra".into(), vec!["Veyra".into()])].into(),
        preceding_ratified_canon: vec![],
    }
}
fn payload(text: &str) -> ExtractionPayloadV1 {
    let mut coverage = CLAIM_FAMILIES
        .into_iter()
        .map(|f| (f, FamilyCoverage::InspectedNone))
        .collect::<BTreeMap<_, _>>();
    coverage.insert(ClaimFamily::LifeAction, FamilyCoverage::ClaimsPresent);
    ExtractionPayloadV1 {
        schema_version: 1,
        accepted_text_sha256: exact_text_sha256(text),
        summary: "Veyra dies.".into(),
        tone: None,
        coverage,
        claims: vec![ProposedClaimV1 {
            claim_id: "claim:1".into(),
            subject: ClaimEntity::Resolved {
                id: "character:veyra".into(),
            },
            assertion: ClaimAssertion::LifeStatus {
                status: LifeStatus::Dead,
            },
            within_scene_order: None,
            in_world_time: None,
            modality: ClaimModality::Narration,
            temporal_scope: TemporalScope::Current,
            speaker: None,
            experiencer: None,
            confidence: Some(0.99),
            evidence: vec![ClaimEvidence {
                start_byte: 0,
                end_byte: text.len(),
                quote: text.into(),
            }],
        }],
    }
}
fn router(endpoint: &str) -> (tempfile::TempDir, ModelRouter) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("agents.toml");
    std::fs::write(
        &path,
        format!(
            r#"
[health_check]
enabled = false
[[agents]]
id = "extractor"
name = "Fixture extractor"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "fixture-model"
ratings = ["general"]
[[routing]]
route = "extract"
agent = "extractor"
max_tokens = 8000
"#
        ),
    )
    .unwrap();
    let router = ModelRouter::local_only();
    router.configure(Some(path.to_str().unwrap())).unwrap();
    (tmp, router)
}
async fn read_request(stream: &mut tokio::net::TcpStream) -> serde_json::Value {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let (offset, length) = loop {
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .unwrap()
            .unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..index]).to_lowercase();
            let length = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            break (index + 4, length);
        }
    };
    while bytes.len() < offset + length {
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .unwrap()
            .unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&chunk[..read]);
    }
    let request: serde_json::Value =
        serde_json::from_slice(&bytes[offset..offset + length]).unwrap();
    request
}
async fn server(output: String) -> (String, tokio::task::JoinHandle<serde_json::Value>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let request = read_request(&mut stream).await;
        let body = serde_json::json!({"choices":[{"message":{"content":output},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}).to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        request
    });
    (format!("http://{address}/v1"), task)
}
#[tokio::test]
async fn no_route_is_pending_not_empty_success() {
    let text = "Veyra died.";
    let error = extract_shadow_claims(&ModelRouter::local_only(), &binding(text), text, &context())
        .await
        .unwrap_err();
    assert_eq!(error.state, ExtractionFailureState::Pending);
    assert_eq!(error.code, ExtractionFailureCode::NoRoute);
    assert!(error.retryable);
}
#[test]
fn host_and_provider_use_identical_evidence_and_coverage_gate() {
    let text = "Veyra died.";
    assert_eq!(
        accept_host_extraction(payload(text), binding(text), text, &known(&context()))
            .unwrap()
            .claims
            .len(),
        1
    );
    let mut invalid = payload(text);
    invalid.claims[0].evidence[0].quote = "invented quote".into();
    let error =
        accept_host_extraction(invalid, binding(text), text, &known(&context())).unwrap_err();
    assert_eq!(error.code, ExtractionFailureCode::InvalidClaims);
    assert_eq!(
        error.validation.unwrap().code,
        ExtractionValidationCode::Evidence
    );
    assert!(!error.retryable);
}
#[test]
fn input_budget_and_hash_fail_before_dispatch_without_truncation() {
    let text = "Veyra died.";
    let mut wrong = binding(text);
    wrong.accepted_text_sha256 = "wrong".into();
    assert_eq!(
        prompt(&wrong, text, &context()).unwrap_err().code,
        ExtractionFailureCode::InvalidInput
    );
    let text = "x".repeat(MAX_INPUT_BYTES);
    assert_eq!(
        prompt(&binding(&text), &text, &context()).unwrap_err().code,
        ExtractionFailureCode::InputBudget
    );
}
#[tokio::test]
async fn independent_http_call_preserves_exact_scene_and_caps_output() {
    let text = "Veyra died.\r\n";
    let (endpoint, task) = server(serde_json::to_string(&payload(text)).unwrap()).await;
    let (_tmp, router) = router(&endpoint);
    let report = extract_shadow_claims(&router, &binding(text), text, &context())
        .await
        .unwrap();
    assert_eq!(report.binding.accepted_text_sha256, exact_text_sha256(text));
    assert_eq!(report.binding.model, "fixture-model");
    let request = task.await.unwrap();
    assert_eq!(request["max_tokens"], 2000);
    let user = request["messages"][1]["content"].as_str().unwrap();
    let data: serde_json::Value =
        serde_json::from_str(user.split("DATA:\n").nth(1).unwrap()).unwrap();
    assert_eq!(data["accepted_text"], text);
    assert!(data.get("drafter_package").is_none());
    let normalized = normalize_drafter_package(
        &DrafterPackageV1 {
            continuity_notes: vec!["all checked".into()],
            ..Default::default()
        },
        &report.binding,
        &known(&context()),
    );
    assert_eq!(
        compare_claim_sources(&report.claims, &normalized.claims)[0].kind,
        ClaimComparisonKind::ExtractorOnly
    );
}
#[tokio::test]
async fn malformed_provider_payload_is_failed_not_empty_success() {
    let text = "Veyra died.";
    let (endpoint, task) = server("not JSON".into()).await;
    let (_tmp, router) = router(&endpoint);
    let error = extract_shadow_claims(&router, &binding(text), text, &context())
        .await
        .unwrap_err();
    assert_eq!(error.state, ExtractionFailureState::Failed);
    assert_eq!(error.code, ExtractionFailureCode::MalformedPayload);
    assert!(error.retryable);
    task.await.unwrap();
}

fn request(text: &str) -> spindle_core::drafting::extraction::AcceptedTextExtractionRequestV1 {
    use spindle_core::drafting::extraction::*;
    let b = binding(text);
    AcceptedTextExtractionRequestV1 {
        task_id: "extract:accepted:1".into(),
        accepted: AcceptedDraftV1 {
            placement: b.position,
            artifact_id: b.revision_id,
            project_id: b.project_id,
            branch_id: b.branch_id,
            scene_id: b.scene_id,
            packet_hash: context().packet_hash,
            text: text.into(),
            text_sha256: b.accepted_text_sha256,
            rating: ContentRating::General,
            receipt: DraftReceiptBinding {
                generation_id: "generation:original".into(),
                agent_id: "writer".into(),
                normalized_output_sha256: exact_text_sha256(text.replace("\r\n", "\n").trim()),
            },
        },
        selected_canon: Vec::new(),
        known_entities: vec![ExtractionEntityRef {
            id: "character:veyra".into(),
            kind: "character".into(),
            display_name: "Veyra".into(),
        }],
    }
}
#[tokio::test]
async fn consumer_port_caches_success_across_restart_and_rejects_changed_task() {
    use spindle_core::drafting::extraction::AcceptedTextExtractor;
    let text = "Veyra died.\r\n";
    let (endpoint, task) = server(serde_json::to_string(&payload(text)).unwrap()).await;
    let (_tmp, router) = router(&endpoint);
    let cache = tempfile::tempdir().unwrap();
    let extractor = IndependentCanonExtractor::new(router, cache.path());
    let request = request(text);
    let first = extractor.extract(&request).await.unwrap();
    task.await.unwrap();
    assert!(
        first.character_states.is_empty()
            && first.canonical_facts.is_empty()
            && first.relationship_updates.is_empty()
            && first.beats.is_empty()
    );
    let audit = first.canon_audit.as_ref().unwrap();
    assert_eq!(audit.report.binding.position, request.accepted.placement);
    assert_eq!(
        audit.report.binding.revision_id,
        request.accepted.artifact_id
    );
    assert_eq!(audit.report.claims.len(), 1);
    let restarted = IndependentCanonExtractor::new(ModelRouter::local_only(), cache.path());
    let cached = restarted.extract(&request).await.unwrap();
    assert_eq!(
        serde_json::to_value(&cached).unwrap(),
        serde_json::to_value(&first).unwrap()
    );
    for change in 0..3 {
        let mut changed = request.clone();
        match change {
            0 => changed.accepted.placement.chapter_number += 1,
            1 => changed.accepted.receipt.generation_id = "different receipt".into(),
            _ => changed.accepted.rating = ContentRating::Explicit,
        }
        assert_eq!(
            restarted.extract(&changed).await.unwrap_err().code,
            "task_identity_changed"
        );
    }
    // Persisted audit is bound to the exact accepted artifact, including receipt.
    use spindle_core::drafting::extraction::PlainTextDraftArtifactV1;
    let accepted = &request.accepted;
    let mut artifact = PlainTextDraftArtifactV1 {
        placement: accepted.placement.clone(),
        prepared_packet: None,
        source_packet_sha256: "source-packet".into(),
        schema_version: "plain_text_draft_v1".into(),
        artifact_id: accepted.artifact_id.clone(),
        project_id: accepted.project_id.clone(),
        branch_id: accepted.branch_id.clone(),
        scene_id: accepted.scene_id.clone(),
        packet_hash: accepted.packet_hash.clone(),
        text: accepted.text.clone(),
        text_sha256: accepted.text_sha256.clone(),
        rating: accepted.rating.clone(),
        truncated: false,
        receipt: accepted.receipt.clone(),
        accepted_text_sha256: Some(accepted.text_sha256.clone()),
        extraction_request: Some(request.clone()),
        extraction_attempts: 1,
        extraction_failure: None,
        extraction: Some(first),
    };
    assert!(crate::drafting::artifact::validate_artifact(&artifact).is_ok());
    artifact.receipt.generation_id = "altered generation".into();
    assert!(crate::drafting::artifact::validate_artifact(&artifact).is_err());
    artifact.receipt = accepted.receipt.clone();
    artifact
        .extraction
        .as_mut()
        .unwrap()
        .canon_audit
        .as_mut()
        .unwrap()
        .report
        .claims[0]
        .proposed
        .evidence[0]
        .quote = "invented".into();
    assert!(crate::drafting::artifact::validate_artifact(&artifact).is_err());
}
#[tokio::test]
async fn failed_task_can_retry_independently_without_a_prose_reroll() {
    use spindle_core::drafting::extraction::AcceptedTextExtractor;
    let text = "Veyra died.";
    let (endpoint, task) = server("invalid JSON".into()).await;
    let (_tmp, router) = router(&endpoint);
    let cache = tempfile::tempdir().unwrap();
    let request = request(text);
    let extractor = IndependentCanonExtractor::new(router, cache.path());
    let failed = extractor.extract(&request).await.unwrap_err();
    assert!(failed.retryable);
    assert_eq!(failed.code, "extraction_failed_malformed_payload");
    task.await.unwrap();
    let (endpoint, task) = server(serde_json::to_string(&payload(text)).unwrap()).await;
    let (_tmp, router) = self::router(&endpoint);
    let retry = IndependentCanonExtractor::new(router, cache.path())
        .extract(&request)
        .await
        .unwrap();
    assert_eq!(retry.accepted_text_sha256, request.accepted.text_sha256);
    assert_eq!(
        retry.canon_audit.unwrap().report.claims[0]
            .proposed
            .evidence[0]
            .quote,
        text
    );
    task.await.unwrap();
}
#[tokio::test]
async fn cache_tampering_and_unresolved_lock_never_become_clean_success() {
    use spindle_core::drafting::extraction::AcceptedTextExtractor;
    let text = "Veyra died.";
    let request = request(text);
    let cache = tempfile::tempdir().unwrap();
    let filename = exact_text_sha256(&request.task_id);
    std::fs::write(cache.path().join(format!("{filename}.lock")), "").unwrap();
    let extractor = IndependentCanonExtractor::new(ModelRouter::local_only(), cache.path());
    assert_eq!(
        extractor.extract(&request).await.unwrap_err().code,
        "extraction_busy_or_recovery_needed"
    );
    std::fs::remove_file(cache.path().join(format!("{filename}.lock"))).unwrap();
    std::fs::write(
        cache.path().join(format!("{filename}.json")),
        "invalid cached JSON",
    )
    .unwrap();
    assert_eq!(
        extractor.extract(&request).await.unwrap_err().code,
        "cache_invalid"
    );
}

#[tokio::test]
async fn pending_task_accepts_host_fallback_and_keeps_unknown_coverage_visible() {
    use spindle_core::drafting::extraction::AcceptedTextExtractor;
    let text = "Veyra died.";
    let request = request(text);
    let cache = tempfile::tempdir().unwrap();
    let extractor = IndependentCanonExtractor::new(ModelRouter::local_only(), cache.path());
    assert!(extractor.extract(&request).await.unwrap_err().retryable);
    let mut invalid = payload(text);
    invalid.claims[0].evidence[0].quote = "invalid".into();
    assert_eq!(
        extractor
            .submit_host_report(&request, invalid)
            .unwrap_err()
            .code,
        "invalid_host_claims"
    );
    let mut host = payload(text);
    host.coverage.insert(
        ClaimFamily::Injury,
        FamilyCoverage::Unknown {
            reason: "referent uncertain".into(),
        },
    );
    let submitted = extractor
        .submit_host_report(&request, host.clone())
        .unwrap();
    assert!(submitted.continuity_notes[0].contains("families with unknown coverage: 1"));
    assert_eq!(
        submitted.canon_audit.as_ref().unwrap().report.binding.model,
        "host"
    );
    assert!(
        submitted
            .canon_audit
            .as_ref()
            .unwrap()
            .report
            .binding
            .model_call_id
            .is_none()
    );
    assert!(submitted.canonical_facts.is_empty());
    assert!(
        !submitted
            .canon_audit
            .as_ref()
            .unwrap()
            .report
            .coverage_is_complete()
    );
    let cached = extractor.extract(&request).await.unwrap();
    assert_eq!(
        serde_json::to_value(cached).unwrap(),
        serde_json::to_value(&submitted).unwrap()
    );
    assert!(extractor.submit_host_report(&request, host.clone()).is_ok());
    host.claims[0].assertion = ClaimAssertion::LifeStatus {
        status: LifeStatus::Alive,
    };
    assert_eq!(
        extractor
            .submit_host_report(&request, host)
            .unwrap_err()
            .code,
        "task_already_completed"
    );
}

#[tokio::test]
async fn extraction_has_one_http_attempt_on_rate_limit() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let captured = count.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            captured.fetch_add(1, Ordering::SeqCst);
            let body = r#"{"error":{"message":"fixture rate limit"}}"#;
            stream.write_all(format!("HTTP/1.1 429 Too Many Requests\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let (_tmp, router) = router(&endpoint);
    let text = "Veyra died.";
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        extract_shadow_claims(&router, &binding(text), text, &context()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(result.code, ExtractionFailureCode::Provider);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn extraction_attribution_is_prose_free_and_never_mutates_canon() {
    let text = "Veyra died.";
    // Fresh migrations can queue behind other workspace fixtures; start the
    // bounded HTTP listener only after the database is ready to dispatch.
    let db = tempfile::tempdir().unwrap();
    let pool = crate::sqlite::SqlitePool::open(&db.path().join("extraction.sqlite"))
        .await
        .unwrap();
    let (endpoint, task) = server(serde_json::to_string(&payload(text)).unwrap()).await;
    let (_tmp, router) = router(&endpoint);
    let router = router.with_usage_pool(pool.clone());
    let report = extract_shadow_claims(&router, &binding(text), text, &context())
        .await
        .unwrap();
    task.await.unwrap();
    let call_id = report.binding.model_call_id.unwrap();
    let absent = ModelRouter::local_only().with_usage_pool(pool.clone());
    assert_eq!(
        extract_shadow_claims(&absent, &binding(text), text, &context())
            .await
            .unwrap_err()
            .code,
        ExtractionFailureCode::NoRoute
    );
    pool.read(move |conn| {
        let (route, template, version, packet, cap, payload): (String, String, String, String, i64, String) = conn.query_row(
            "SELECT route, prompt_template_id, prompt_version, context_packet_hash, max_tokens, payload FROM model_call WHERE id = ?1",
            [&call_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
        assert_eq!(route, "extract"); assert_eq!(template, "canon-independent-extract");
        assert_eq!(version, EXTRACT_PROMPT_VERSION); assert_eq!(packet, "frozen-packet-hash"); assert_eq!(cap, 2000);
        assert!(!payload.contains("Veyra") && !payload.contains("Veyra died."));
        let counts: (i64, i64) = conn.query_row("SELECT COUNT(*), SUM(CASE WHEN json_extract(payload, '$.outcome') = 'failed' THEN 1 ELSE 0 END) FROM model_call", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        assert_eq!(counts, (2, 1));
        let canon: i64 = conn.query_row("SELECT (SELECT COUNT(*) FROM canonical_fact) + (SELECT COUNT(*) FROM character_state) + (SELECT COUNT(*) FROM knowledge_fact) + (SELECT COUNT(*) FROM quantity_state) + (SELECT COUNT(*) FROM relates_to)", [], |r| r.get(0))?;
        assert_eq!(canon, 0);
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn extraction_refuses_cli_backend_without_executing_it() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("executed");
    let script = tmp.path().join("agent.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf executed > '{}'\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let config = tmp.path().join("agents.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[health_check]
enabled = false
[[agents]]
id = "cli-extractor"
name = "CLI extractor"
provider = "cli"
endpoint = "{}"
model = "fixture"
ratings = ["general"]
[[routing]]
route = "extract"
agent = "cli-extractor"
"#,
            script.display()
        ),
    )
    .unwrap();
    let router = ModelRouter::local_only();
    router.configure(Some(config.to_str().unwrap())).unwrap();
    let text = "Veyra died.";
    let error = extract_shadow_claims(&router, &binding(text), text, &context())
        .await
        .unwrap_err();
    assert_eq!(error.code, ExtractionFailureCode::UnsupportedBackend);
    assert_eq!(error.state, ExtractionFailureState::Pending);
    assert!(!marker.exists());
}

#[tokio::test]
async fn p1_artifact_driver_resumes_budget_pending_task_from_host_cache() {
    use crate::drafting::artifact::{
        accept_artifact, extract_accepted, load_artifact, save_artifact,
    };
    use spindle_core::drafting::extraction::PlainTextDraftArtifactV1;
    let text = "Veyra died.";
    let mut request = request(text);
    request.selected_canon.push(PacketItem {
        source: spindle_core::drafting::PacketSource {
            branch_id: request.accepted.branch_id.clone(),
            source_id: "canonical_fact:prior".into(),
            revision_hash: "prior-version".into(),
        },
        proposition_id: "prior".into(),
        text: "Preceding ratified evidence. ".repeat(2000),
        relevance: 1,
        recency: 1,
    });
    let accepted = &request.accepted;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accepted.json");
    let artifact = PlainTextDraftArtifactV1 {
        placement: accepted.placement.clone(),
        prepared_packet: None,
        source_packet_sha256: "source".into(),
        schema_version: "plain_text_draft_v1".into(),
        artifact_id: accepted.artifact_id.clone(),
        project_id: accepted.project_id.clone(),
        branch_id: accepted.branch_id.clone(),
        scene_id: None,
        packet_hash: accepted.packet_hash.clone(),
        text: text.into(),
        text_sha256: accepted.text_sha256.clone(),
        rating: ContentRating::General,
        truncated: false,
        receipt: accepted.receipt.clone(),
        accepted_text_sha256: None,
        extraction_request: None,
        extraction_attempts: 0,
        extraction_failure: None,
        extraction: None,
    };
    save_artifact(&path, &artifact).unwrap();
    accept_artifact(&path, &accepted.text_sha256).unwrap();
    let extractor =
        IndependentCanonExtractor::new(ModelRouter::local_only(), dir.path().join("cache"));
    let pending = extract_accepted(
        &path,
        &extractor,
        request.selected_canon.clone(),
        request.known_entities.clone(),
    )
    .await
    .unwrap();
    let failure = pending.extraction_failure.unwrap();
    assert_eq!(failure.code, "extraction_pending_input_budget");
    assert!(failure.retryable);
    extractor
        .submit_host_report(&request, payload(text))
        .unwrap();
    let complete = extract_accepted(
        &path,
        &extractor,
        request.selected_canon.clone(),
        request.known_entities.clone(),
    )
    .await
    .unwrap();
    assert!(complete.extraction_failure.is_none());
    assert_eq!(complete.text, text);
    assert_eq!(
        complete.receipt.generation_id,
        request.accepted.receipt.generation_id
    );
    assert_eq!(
        complete
            .extraction
            .as_ref()
            .unwrap()
            .canon_audit
            .as_ref()
            .unwrap()
            .report
            .binding
            .model,
        "host"
    );
    assert!(load_artifact(&path).unwrap().extraction.is_some());
}
