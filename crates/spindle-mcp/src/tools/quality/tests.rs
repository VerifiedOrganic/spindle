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
    let mut all = BTreeMap::new();
    for profile in budgets().compatibility.keys() {
        router.tool_profile = (profile != "default").then(|| profile.clone());
        for tool in router.list_tools() {
            let tool = serde_json::to_value(tool).unwrap();
            let name = tool["name"].as_str().unwrap().to_owned();
            if let Some(previous) = all.insert(name.clone(), tool.clone()) {
                assert_eq!(previous, tool, "profile schema drift: {name}");
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
    assert!(
        !report.is_empty(),
        "do not claim legacy schemas are lint clean"
    );
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
