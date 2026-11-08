use super::*;
#[path = "support.rs"]
mod support;
use support::*;

#[test]
fn schema_lint_reports_nested_and_referenced_properties() {
    let schema = serde_json::json!({"type":"object", "properties": {
        "optional": {"$ref":"#/$defs/Nested", "description":"An optional nested record."}
    }, "$defs": {"Nested": {"type":"object", "properties": {"count":{"type":"integer"}}}}});
    let issues = lint_schema(&schema);
    assert!(
        issues
            .iter()
            .any(|issue| issue.pointer == "/$defs/Nested/properties/count"
                && issue.rule == "missing_description")
    );
}

#[test]
fn schema_lint_handles_cycles_and_reports_unresolved_refs() {
    let cyclic = serde_json::json!({"type":"object", "properties":{"child":{"description":"Recursive child record.","$ref":"#"}}});
    assert!(lint_schema(&cyclic).is_empty());
    let broken = serde_json::json!({"$ref":"#/$defs/Missing"});
    assert!(
        lint_schema(&broken)
            .iter()
            .any(|issue| issue.rule == "unresolved_ref")
    );
}

#[test]
fn schema_lint_flags_placeholder_jargon_and_string_vocabulary() {
    let schema = serde_json::json!({"type":"object", "properties": {
        "mode":{"type":"string", "description":"mode"},
        "mining_policy":{"type":"string", "description":"Evolution §3 policy."}
    }});
    let issues = lint_schema(&schema);
    for rule in [
        "placeholder_description",
        "implementation_jargon",
        "string_vocabulary_review",
    ] {
        assert!(
            issues.iter().any(|issue| issue.rule == rule),
            "missing {rule}"
        );
    }
}

#[test]
fn canonical_budget_rejects_oversized_descriptions_and_output() {
    for field in ["description", "outputSchema"] {
        let mut tool = serde_json::json!({"name":"small", "description":"A useful operation.","inputSchema":{"type":"object"}});
        tool[field] = if field == "description" {
            Value::String("x".repeat(5000))
        } else {
            serde_json::json!({"description":"x".repeat(5000)})
        };
        assert!(
            check_budget(
                &[tool],
                &Budget {
                    tools: 1,
                    input_bytes: 4096,
                    full_bytes: 4096
                }
            )
            .is_err(),
            "{field}"
        );
    }
}

#[test]
fn schema_meta_validation_rejects_invalid_keyword_types() {
    assert!(
        jsonschema::meta::validate(&serde_json::json!({"type":"string", "minLength":"many"}))
            .is_err()
    );
    assert!(
        jsonschema::meta::validate(&serde_json::json!({"type":"string", "minLength":1})).is_ok()
    );
}

#[test]
fn canonical_tools_have_no_lint_or_size_exemptions() {
    let clean = serde_json::json!({"name":"choose", "description":"Choose how to draft the next scene.", "inputSchema":{
        "type":"object", "properties":{"mode":{"type":"string","description":"Select host or routed generation.", "enum":["host","agent"]}}
    }});
    assert!(canonical_tool_issues(&clean).is_empty());
    let mut large = clean.clone();
    large["outputSchema"] = serde_json::json!({"description":"x".repeat(4096)});
    assert!(
        canonical_tool_issues(&large)
            .iter()
            .any(|issue| issue.rule == "discovery_size")
    );
    let mut poor = clean;
    poor["inputSchema"]["properties"]["mode"]
        .as_object_mut()
        .unwrap()
        .remove("description");
    assert!(
        canonical_tool_issues(&poor)
            .iter()
            .any(|issue| issue.rule == "missing_description")
    );
}

#[test]
fn explicit_closed_sets_require_enums_and_existing_fields() {
    let schema = serde_json::json!({"type":"object", "properties":{"mode":{"type":"string"}, "kind":{"type":"string","enum":["host","agent"]}}});
    let fields = [
        "/properties/mode",
        "/properties/kind",
        "/properties/missing",
    ]
    .map(str::to_owned);
    let issues = closed_field_issues(&schema, &fields);
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].rule, "closed_set_missing_enum");
    assert_eq!(issues[1].rule, "closed_set_missing_field");
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Budgets {
    version: usize,
    compatibility: std::collections::BTreeMap<String, Budget>,
    canonical: std::collections::BTreeMap<String, Budget>,
}

fn budgets() -> Budgets {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/profile-budgets.json"
    )))
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn profile_token_budgets() {
    let budgets = budgets();
    assert_eq!(budgets.version, 1);
    assert_eq!(
        budgets.compatibility.len(),
        crate::tools::profile_names().len() + 1
    );
    assert_eq!(budgets.canonical.len(), 10);
    let mut router = router().await;
    for (profile, budget) in &budgets.compatibility {
        router.tool_profile = (profile != "default").then(|| profile.clone());
        let tools: Vec<Value> = router
            .list_tools()
            .iter()
            .map(|t| serde_json::to_value(t).unwrap())
            .collect();
        check_budget(&tools, budget).unwrap_or_else(|error| panic!("{profile}: {error}"));
        let (_, input, full) = measure(&tools);
        println!(
            "{profile}: input {input} bytes/{} proxy tokens; full {full} bytes/{} proxy tokens",
            input.div_ceil(4),
            full.div_ceil(4)
        );
    }
    for budget in budgets.canonical.values() {
        let output = serde_json::json!({"name":"large", "description":"Useful operation.", "inputSchema":{"type":"object"},"outputSchema":{"description":"x".repeat(budget.full_bytes)}});
        assert!(check_budget(&[output], budget).is_err());
        let input = serde_json::json!({"name":"large", "description":"x".repeat(budget.input_bytes),"inputSchema":{"type":"object"}});
        assert!(check_budget(&[input], budget).is_err());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn tool_schema_lint() {
    use std::collections::BTreeMap;
    let closed: BTreeMap<String, Vec<String>> =
        serde_json::from_str(include_str!("closed-fields.json")).unwrap();
    let mut report: BTreeMap<String, Vec<Issue>> = BTreeMap::new();
    let mut router = router().await;
    let all: BTreeMap<_, _> = router
        .list_tools()
        .into_iter()
        .map(|tool| (tool.name.to_string(), serde_json::to_value(tool).unwrap()))
        .collect();
    for profile in budgets().compatibility.keys() {
        router.tool_profile = (profile != "default").then(|| profile.clone());
        for tool in router.list_tools() {
            let original = all_tool_schemas()
                .iter()
                .find(|original| original.name == tool.name)
                .unwrap()
                .clone();
            let expected = project_tool_discovery(router.tool_profile.as_deref(), original);
            assert_eq!(
                tool, expected,
                "unexpected discovery drift in {profile}/{}",
                tool.name
            );
            let value = serde_json::to_value(tool).unwrap();
            for key in ["inputSchema", "outputSchema"] {
                if let Some(schema) = value.get(key) {
                    jsonschema::meta::validate(schema)
                        .unwrap_or_else(|error| panic!("{profile}/{key}: {error}"));
                }
            }
        }
    }
    for name in closed.keys() {
        assert!(
            all.contains_key(name),
            "stale closed-set annotation: {name}"
        );
    }
    for (name, tool) in all {
        for key in ["inputSchema", "outputSchema"] {
            if let Some(schema) = tool.get(key) {
                jsonschema::meta::validate(schema)
                    .unwrap_or_else(|error| panic!("{name}/{key}: invalid JSON Schema: {error}"));
            }
        }
        let mut issues = lint_tool(&tool);
        if let Some(pointers) = closed.get(&name) {
            issues.extend(
                closed_field_issues(&tool["inputSchema"], pointers)
                    .into_iter()
                    .map(|mut issue| {
                        issue.pointer = format!("/inputSchema{}", issue.pointer);
                        issue
                    }),
            );
        }
        issues.sort();
        if !issues.is_empty() {
            report.insert(name, issues);
        }
    }
    let grouped: BTreeMap<_, BTreeMap<&str, Vec<&str>>> = report
        .iter()
        .map(|(name, issues)| {
            let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for issue in issues {
                groups.entry(issue.rule).or_default().push(&issue.pointer);
            }
            (name, groups)
        })
        .collect();
    let mut actual = String::from("{\n");
    for (index, (name, issues)) in grouped.iter().enumerate() {
        if index != 0 {
            actual.push_str(",\n");
        }
        actual.push_str(&format!(
            "  {}: {}",
            serde_json::to_string(name).unwrap(),
            serde_json::to_string(issues).unwrap()
        ));
    }
    actual.push_str("\n}\n");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/tools/quality/legacy-schema-inventory.json");
    if std::env::var("SPINDLE_UPDATE_SCHEMA_INVENTORY").as_deref() == Ok("1") {
        std::fs::write(&path, &actual).unwrap();
    }
    let expected = std::fs::read_to_string(path).unwrap();
    if expected != actual {
        let previous: BTreeMap<String, BTreeMap<String, Vec<String>>> =
            serde_json::from_str(&expected).unwrap();
        for (name, groups) in &grouped {
            for (rule, pointers) in groups {
                let old = previous.get(*name).and_then(|rules| rules.get(*rule));
                let added: Vec<_> = pointers
                    .iter()
                    .filter(|p| !old.is_some_and(|old| old.iter().any(|v| v == **p)))
                    .take(20)
                    .collect();
                if !added.is_empty() {
                    eprintln!("{name}: new {rule} findings: {added:?}");
                }
            }
        }
        panic!(
            "legacy diagnostic inventory changed; inspect new pointers above and removed findings in generated diff; canonical lint has no waivers"
        );
    }
    for forbidden_rule in [
        "closed_set_missing_enum",
        "closed_set_missing_field",
        "implementation_jargon",
        "provider_promise",
        "placeholder_description",
        "unresolved_ref",
    ] {
        let offenders: Vec<_> = grouped
            .iter()
            .filter_map(|(name, groups)| {
                groups.get(forbidden_rule).map(|pointers| (name, pointers))
            })
            .collect();
        assert!(
            offenders.is_empty(),
            "{forbidden_rule} must have zero findings across all tools: {offenders:?}"
        );
    }
    assert!(
        !report.is_empty(),
        "do not claim legacy schemas are lint clean"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn closed_set_enum_values_round_trip_through_parsers() {
    for mode in ["hybrid", "agent", "  Hybrid ", "AGENT", "agent_draft"] {
        let exec: AuthoringExecuteNextInput = parse_arguments(
            serde_json::json!({
                "project_id": "project:demo",
                "mode": mode,
            })
            .as_object(),
        )
        .unwrap();
        let serialized = serde_json::to_value(&exec).unwrap();
        let round_trip: AuthoringExecuteNextInput = serde_json::from_value(serialized).unwrap();
        assert_eq!(round_trip.mode, exec.mode);
    }
    assert!(
        parse_arguments::<AuthoringExecuteNextInput>(
            serde_json::json!({
                "project_id": "project:demo",
                "mode": "invalid_mode",
            })
            .as_object(),
        )
        .is_err()
    );

    for mining_policy in ["disabled", "propose_all", "  DISABLED ", "PROPOSE_ALL"] {
        for checkpoint_policy in ["manual", "auto_advisory", "auto_strict", "  Manual "] {
            for replan_policy in ["disabled", "propose_all", "  Disabled ", "PROPOSE_ALL"] {
                let prep: AuthoringPrepareRunInput = parse_arguments(
                    serde_json::json!({
                        "project_id": "project:demo",
                        "book_number": 1,
                        "start_chapter": 1,
                        "mining_policy": mining_policy,
                        "checkpoint_policy": checkpoint_policy,
                        "replan_policy": replan_policy,
                    })
                    .as_object(),
                )
                .unwrap();
                let prep_round_trip: AuthoringPrepareRunInput =
                    serde_json::from_value(serde_json::to_value(&prep).unwrap()).unwrap();
                assert_eq!(prep_round_trip.mining_policy, prep.mining_policy);
                assert_eq!(prep_round_trip.checkpoint_policy, prep.checkpoint_policy);
                assert_eq!(prep_round_trip.replan_policy, prep.replan_policy);

                let start: AuthoringStartRunInput = parse_arguments(
                    serde_json::json!({
                        "project_id": "project:demo",
                        "book_number": 1,
                        "start_chapter": 1,
                        "checkpoint_interval": 2,
                        "mode": "hybrid",
                        "mining_policy": mining_policy,
                        "checkpoint_policy": checkpoint_policy,
                        "replan_policy": replan_policy,
                    })
                    .as_object(),
                )
                .unwrap();
                let start_round_trip: AuthoringStartRunInput =
                    serde_json::from_value(serde_json::to_value(&start).unwrap()).unwrap();
                assert_eq!(start_round_trip.mode, start.mode);
                assert_eq!(start_round_trip.mining_policy, start.mining_policy);
                assert_eq!(start_round_trip.checkpoint_policy, start.checkpoint_policy);
                assert_eq!(start_round_trip.replan_policy, start.replan_policy);
            }
        }
    }

    let router = router().await;
    for (field, invalid) in [
        ("mode", "unsupported_mode"),
        ("mining_policy", "auto_accept"),
        ("checkpoint_policy", "auto"),
        ("replan_policy", "auto_accept"),
    ] {
        let mut args = serde_json::json!({
            "project_id": "project:demo",
            "book_number": 1,
            "start_chapter": 1,
            "checkpoint_interval": 2,
        });
        args[field] = Value::String(invalid.to_owned());
        let output = structured_json(
            router
                .call_tool("authoring_start_run", args.as_object())
                .await
                .unwrap(),
        );
        assert_eq!(output["status"], "blocked", "{field}={invalid}");
        assert!(
            output["message"].as_str().unwrap().contains(field)
                && output["message"].as_str().unwrap().contains(invalid),
            "unexpected rejection message for {field}: {output}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn http_profile_discovery_matches_registry_budgets() {
    use rmcp::ServiceExt as _;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    struct ServerTask(
        tokio::task::JoinHandle<()>,
        tokio_util::sync::CancellationToken,
    );
    impl Drop for ServerTask {
        fn drop(&mut self) {
            self.1.cancel();
            self.0.abort();
        }
    }
    let mut registry = router().await;
    for (profile, budget) in budgets().compatibility {
        let selected = (profile != "default").then(|| profile.clone());
        registry.tool_profile = selected.clone();
        let server = crate::server::SpindleMcpServer::with_tool_profile(
            registry.service().clone(),
            selected,
        );
        let cancellation = tokio_util::sync::CancellationToken::new();
        let http = StreamableHttpService::new(
            move || Ok(server.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().with_cancellation_token(cancellation.clone()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new().nest_service("/mcp", http);
        let _task = ServerTask(
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            }),
            cancellation,
        );
        let transport = StreamableHttpClientTransport::from_uri(format!("http://{address}/mcp"));
        let client = timeout(Duration::from_secs(20), ().serve(transport))
            .await
            .unwrap()
            .unwrap();
        let tools = timeout(Duration::from_secs(20), client.list_all_tools())
            .await
            .unwrap()
            .unwrap();
        let actual: Vec<Value> = tools
            .iter()
            .map(|t| serde_json::to_value(t).unwrap())
            .collect();
        let expected: Vec<Value> = registry
            .list_tools()
            .iter()
            .map(|t| serde_json::to_value(t).unwrap())
            .collect();
        assert_eq!(actual, expected, "HTTP profile {profile}");
        assert_eq!(measure(&actual), measure(&expected));
        check_budget(&actual, &budget).unwrap();
        timeout(Duration::from_secs(10), client.cancel())
            .await
            .unwrap()
            .unwrap();
    }
}

#[test]
fn schema_lint_checks_tool_descriptions_and_escapes_pointer_names() {
    let tool = serde_json::json!({"name":"research", "description":"Research using Gemini.", "inputSchema":{
        "type":"object", "properties":{"a/b~c":{"type":"string"}}
    }});
    let issues = lint_tool(&tool);
    assert!(
        issues
            .iter()
            .any(|i| i.pointer == "/description" && i.rule == "provider_promise")
    );
    assert!(
        issues
            .iter()
            .any(|i| i.pointer == "/inputSchema/properties/a~1b~0c")
    );
}

#[test]
fn skill_load_budgets() {
    let spindle = spindle_adapters::get_skill("spindle").expect("spindle skill");
    let spindle_bytes = spindle.markdown.len();
    let spindle_tokens = spindle_bytes.div_ceil(4);
    assert!(
        spindle_bytes <= 4_000 && spindle_tokens <= 1_000,
        "spindle entry skill exceeded budget: {spindle_bytes} bytes ({spindle_tokens} proxy tokens)"
    );
    for forbidden in ["Codex", "GPT-6", "Grok"] {
        assert!(
            !spindle.markdown.contains(forbidden),
            "spindle entry skill must stay host-neutral, found {forbidden:?}"
        );
    }

    let scene_writer = spindle_adapters::get_skill("scene-writer").expect("scene-writer skill");
    let scene_writer_bytes = scene_writer.markdown.len();
    let scene_writer_tokens = scene_writer_bytes.div_ceil(4);
    assert!(
        scene_writer_bytes <= 10_000 && scene_writer_tokens <= 2_500,
        "scene-writer core exceeded budget: {scene_writer_bytes} bytes ({scene_writer_tokens} proxy tokens)"
    );
    assert!(
        !scene_writer
            .markdown
            .to_ascii_lowercase()
            .contains("read everything before drafting"),
        "scene-writer core must not instruct reading everything before drafting"
    );

    let combined_bytes = spindle_bytes + scene_writer_bytes;
    let combined_tokens = combined_bytes.div_ceil(4);
    assert!(
        combined_bytes <= 16_000 && combined_tokens <= 4_000,
        "spindle + scene-writer core exceeded budget: {combined_bytes} bytes ({combined_tokens} proxy tokens)"
    );

    let scene_writer_refs: Vec<&str> = scene_writer
        .manifest
        .references
        .iter()
        .map(|r| r.name)
        .collect();
    for expected_ref in [
        "scene-writer/authoring-runs",
        "scene-writer/craft-and-voice",
        "scene-writer/drafting-packets",
        "scene-writer/offloaded-generation",
        "scene-writer/research-and-sources",
    ] {
        assert!(
            scene_writer_refs.contains(&expected_ref),
            "missing scene-writer reference {expected_ref} in {scene_writer_refs:?}"
        );
    }

    for skill in spindle_adapters::list_skills() {
        if skill.name == "spindle" || skill.name == "scene-writer" {
            continue;
        }
        let bytes = skill.markdown.len();
        let tokens = bytes.div_ceil(4);
        assert!(
            bytes <= 12_000 && tokens <= 3_000,
            "skill {} core exceeded budget: {bytes} bytes ({tokens} proxy tokens)",
            skill.name
        );
    }

    for reference in spindle_adapters::list_references() {
        let bytes = reference.markdown.len();
        let tokens = bytes.div_ceil(4);
        assert!(
            bytes <= 12_000 && tokens <= 3_000,
            "reference {} exceeded budget: {bytes} bytes ({tokens} proxy tokens)",
            reference.name
        );
    }
}
