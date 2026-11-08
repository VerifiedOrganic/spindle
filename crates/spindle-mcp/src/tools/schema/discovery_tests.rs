use super::*;

#[tokio::test(flavor = "current_thread")]
async fn generated_discovery_omits_selected_context_schemas_but_keeps_receipts() {
    let mut router = router().await;
    let legacy = router.list_tools();
    router.tool_profile = Some("workflow-write".to_owned());
    let generated = router.list_tools();
    let context = generated
        .iter()
        .find(|t| t.name == "get_scene_context")
        .unwrap();
    assert!(context.output_schema.is_none());
    assert!(
        context
            .description
            .as_deref()
            .unwrap()
            .contains("bible://contracts/tools/v1/get_scene_context")
    );
    let old = legacy
        .iter()
        .find(|t| t.name == "get_scene_context")
        .unwrap();
    assert_eq!(old.input_schema, context.input_schema);
    assert!(old.output_schema.is_some());
    assert!(
        generated
            .iter()
            .find(|t| t.name == "save_scene_draft")
            .unwrap()
            .output_schema
            .is_some()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn optional_response_contracts_work_with_resources_and_tool_only_hosts() {
    let router = router().await;
    let resources = crate::resources::ResourceRouter::new(router.service().clone());
    let uri = "bible://contracts/tools/v1/get_scene_context";
    let response = resources.read_resource(uri).await.unwrap();
    let rmcp::model::ResourceContents::TextResourceContents { text, .. } = &response.contents[0]
    else {
        panic!("JSON Schema resource expected")
    };
    let schema: Value = serde_json::from_str(text).unwrap();
    jsonschema::meta::validate(&schema).unwrap();
    let original = router
        .list_tools()
        .into_iter()
        .find(|t| t.name == "get_scene_context")
        .unwrap();
    assert_eq!(
        schema,
        Value::Object((**original.output_schema.as_ref().unwrap()).clone())
    );
    let args = serde_json::json!({"name":"contracts/tools/v1/get_scene_context"});
    let fallback = structured_json(
        router
            .call_tool("get_reference", args.as_object())
            .await
            .unwrap(),
    );
    assert!(
        fallback["markdown"]
            .as_str()
            .unwrap()
            .contains(&serde_json::to_string_pretty(&schema).unwrap())
    );
    for path in [
        "bible://contracts/tools/v2/get_scene_context",
        "bible://contracts/tools/v1/../get_scene_context",
        "bible://contracts/tools/v1/not_a_tool",
    ] {
        assert!(resources.read_resource(path).await.is_err(), "{path}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn all_optional_contracts_are_exact_valid_and_listed() {
    let router = router().await;
    let resources = crate::resources::ResourceRouter::new(router.service().clone());
    let index = resources
        .read_resource("bible://contracts/tools/v1")
        .await
        .unwrap();
    let rmcp::model::ResourceContents::TextResourceContents { text, .. } = &index.contents[0]
    else {
        panic!("index JSON expected")
    };
    let index: ResponseContractIndex = serde_json::from_str(text).unwrap();
    assert_eq!(index.version, 1);
    assert_eq!(index.contracts.len(), response_contract_names().len());
    for entry in index.contracts {
        let schema = response_contract(&entry.tool_name).unwrap();
        jsonschema::meta::validate(&Value::Object(schema.clone())).unwrap();
        let args = serde_json::json!({"name":entry.reference_name});
        let result = structured_json(
            router
                .call_tool("get_reference", args.as_object())
                .await
                .unwrap(),
        );
        assert!(
            result["markdown"]
                .as_str()
                .unwrap()
                .contains(&serde_json::to_string_pretty(schema).unwrap())
        );
    }
    let listing = structured_json(router.call_tool("list_skills", None).await.unwrap());
    assert!(
        listing["references"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "contracts/tools/v1")
    );
    for name in [
        "contracts/tools/v2/get_scene_context",
        "contracts/tools/v1/../get_scene_context",
        "contracts/tools/v1/not_a_tool",
    ] {
        let args = serde_json::json!({"name":name});
        assert_eq!(
            router
                .call_tool("get_reference", args.as_object())
                .await
                .unwrap()
                .is_error,
            Some(true)
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn output_omission_does_not_change_the_actual_structured_result() {
    let mut router = router().await;
    let args = serde_json::json!({"name":"Contract test", "project_type":"novel", "genre":"fantasy", "reader_contract":{"promise":"A test story", "style_notes":[], "boundaries":[]}});
    let project = structured_json(
        router
            .call_tool("create_project", args.as_object())
            .await
            .unwrap(),
    );
    let args = serde_json::json!({"project_id":project["project_id"], "name":"Harbor", "kind":"city", "summary":"A coastal city."});
    let location = structured_json(
        router
            .call_tool("create_location", args.as_object())
            .await
            .unwrap(),
    );
    let args = serde_json::json!({"project_id":project["project_id"], "table":"location", "entity_id":location["location_id"]});
    let before = structured_json(
        router
            .call_tool("get_entity", args.as_object())
            .await
            .unwrap(),
    );
    router.tool_profile = Some("workflow-write".to_owned());
    let after = structured_json(
        router
            .call_tool("get_entity", args.as_object())
            .await
            .unwrap(),
    );
    assert_eq!(before, after);
    let schema = Value::Object(response_contract("get_entity").unwrap().clone());
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&after)
        .unwrap();
}

#[test]
fn projection_is_idempotent_and_preserves_all_validation_keywords() {
    for original in all_tool_schemas() {
        let projected = project_tool_discovery(Some("workflow-write"), original.clone());
        assert_eq!(
            project_tool_discovery(Some("workflow-write"), projected.clone()),
            projected
        );
        for profile in [
            None,
            Some("legacy-all"),
            Some("write"),
            Some("minimal"),
            Some("authoring"),
            Some("import"),
        ] {
            assert_eq!(project_tool_discovery(profile, original.clone()), *original);
        }
        let mut normalized = projected.clone();
        if original.name == "get_reference" {
            let mut input = (*projected.input_schema).clone();
            input["properties"]["name"]["description"] =
                original.input_schema["properties"]["name"]["description"].clone();
            assert_eq!(
                input, *original.input_schema,
                "only the reference-name description may change"
            );
            normalized.input_schema = original.input_schema.clone();
        }
        if response_contract_names().contains(&original.name.as_ref()) {
            assert!(original.output_schema.is_some());
            assert!(projected.output_schema.is_none());
            let meta = projected.meta.as_ref().unwrap();
            assert_eq!(meta["spindle/responseContract"]["version"], 1);
            assert!(
                projected
                    .description
                    .as_deref()
                    .unwrap()
                    .contains(meta["spindle/responseContract"]["uri"].as_str().unwrap())
            );
            normalized.output_schema = original.output_schema.clone();
            normalized.description = original.description.clone();
            normalized.meta = original.meta.clone();
        }
        if original.name == "research_query" {
            assert!(
                projected
                    .description
                    .as_deref()
                    .unwrap()
                    .contains("configured research route")
            );
            assert!(!projected.description.as_deref().unwrap().contains("Gemini"));
            normalized.description = original.description.clone();
        }
        assert_eq!(
            normalized, *original,
            "unexpected descriptor change for {}",
            original.name
        );
    }
    for name in [
        "save_scene_draft",
        "revise_scene",
        "commit_scene_changes",
        "authoring_save_scene_draft",
        "import_manuscript",
    ] {
        assert!(
            response_contract(name).is_none(),
            "receipt schemas stay inline: {name}"
        );
    }
}
