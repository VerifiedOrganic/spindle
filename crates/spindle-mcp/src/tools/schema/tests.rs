use super::*;

#[tokio::test(flavor = "current_thread")]
async fn tool_schemas_contain_no_gemini_incompatible_keywords() {
    let router = router().await;
    let tools = router.list_tools();
    let forbidden = ["$ref", "$defs", "definitions", "$schema", "$id"];
    for tool in &tools {
        let schema_json = serde_json::to_string(&tool.input_schema).expect("serialize");
        for keyword in &forbidden {
            assert!(
                !schema_json.contains(keyword),
                "tool '{}' schema contains forbidden keyword '{}':\n{}",
                tool.name,
                keyword,
                serde_json::to_string_pretty(&tool.input_schema).unwrap()
            );
        }
    }
}

/// Regression pin for the Moonshot rejection: `enum value (<nil>) does
/// not match any type in [string]`. No served schema may carry a null
/// enum entry, input or output, anywhere in the tree.
#[tokio::test]
async fn served_tool_schemas_carry_no_null_enum_values() {
    let router = router().await;
    let tools = router.list_tools();
    assert!(!tools.is_empty(), "expected registered tools");

    let mut offenders: Vec<String> = Vec::new();
    for tool in &tools {
        let input = Value::Object((*tool.input_schema).clone());
        collect_null_enum_paths(&input, &format!("{}#input", tool.name), &mut offenders);
        if let Some(output) = &tool.output_schema {
            let output = Value::Object((**output).clone());
            collect_null_enum_paths(&output, &format!("{}#output", tool.name), &mut offenders);
        }
    }

    assert!(
        offenders.is_empty(),
        "found {} enum arrays containing null in served schemas; sample:\n{}",
        offenders.len(),
        offenders
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn strip_null_enum_values_drops_nulls_and_empty_enums() {
    let mut value = serde_json::json!({
        "properties": {
            "scope": {"type": "string", "enum": ["book", "chapter", null]},
            "only_null": {"enum": [null]},
            "clean": {"type": "string", "enum": ["a", "b"]},
            "nested": {"items": {"anyOf": [{"enum": ["x", null]}]}}
        }
    });
    strip_null_enum_values(&mut value);
    assert_eq!(
        value["properties"]["scope"]["enum"],
        serde_json::json!(["book", "chapter"])
    );
    assert!(
        value["properties"]["only_null"].get("enum").is_none(),
        "an enum emptied by null-removal must be dropped entirely"
    );
    assert_eq!(
        value["properties"]["clean"]["enum"],
        serde_json::json!(["a", "b"])
    );
    assert_eq!(
        value["properties"]["nested"]["items"]["anyOf"][0]["enum"],
        serde_json::json!(["x"])
    );
}

/// REGRESSION PIN: every registered tool's input AND output schema, as
/// served over MCP, must contain zero denylisted numeric `format` values
/// anywhere in the JSON tree. This catches any future count-heavy tool that
/// reintroduces schemars' non-standard formats at the serving seam.
#[tokio::test(flavor = "current_thread")]
async fn served_tool_schemas_carry_no_nonstandard_numeric_formats() {
    let router = router().await;
    let tools = router.list_tools();
    assert!(!tools.is_empty(), "expected registered tools");

    let mut offenders: Vec<String> = Vec::new();
    for tool in &tools {
        let input = Value::Object((*tool.input_schema).clone());
        let mut found = Vec::new();
        collect_format_values(&input, &format!("{}#input", tool.name), &mut found);
        if let Some(output) = &tool.output_schema {
            let output = Value::Object((**output).clone());
            collect_format_values(&output, &format!("{}#output", tool.name), &mut found);
        }
        for (path, fmt) in found {
            if is_denylisted_numeric_format(&fmt) {
                offenders.push(format!("{path} = \"{fmt}\""));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "found {} non-standard numeric `format` annotations in served schemas; sample:\n{}",
        offenders.len(),
        offenders
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// INTEGRATION-SHAPE PIN: `import_hydrate_bible`'s output schema carries
/// `ImportHydrationRecordCount` (a `$defs` subschema of `usize` counts) —
/// exactly the shape whose `#/$defs/ImportHydrationRecordCount/properties/created`
/// path Kimi Code warned about. Assert the previously-observed paths are clean
/// while the surrounding structure (the $def, its properties) survives intact.
#[tokio::test(flavor = "current_thread")]
async fn import_hydrate_bible_output_schema_is_format_clean() {
    let router = router().await;
    let tools = router.list_tools();
    let tool = tools
        .iter()
        .find(|t| t.name == "import_hydrate_bible")
        .expect("import_hydrate_bible tool");
    let output = tool
        .output_schema
        .as_ref()
        .expect("import_hydrate_bible has an output schema");
    let output = Value::Object((**output).clone());

    // Structure preserved: the $def and its count properties still exist.
    let created = output
        .pointer("/$defs/ImportHydrationRecordCount/properties/created")
        .expect("ImportHydrationRecordCount.created property must survive scrubbing");
    assert_eq!(
        created.get("type").and_then(Value::as_str),
        Some("integer"),
        "count property keeps its integer type"
    );
    // The offending format is gone from every observed path.
    assert!(
        created.get("format").is_none(),
        "created must not carry a non-standard `format`: {created}"
    );

    // Whole-tree sweep on this specific schema.
    let mut found = Vec::new();
    collect_format_values(&output, "#", &mut found);
    let offenders: Vec<_> = found
        .into_iter()
        .filter(|(_, f)| is_denylisted_numeric_format(f))
        .collect();
    assert!(
        offenders.is_empty(),
        "import_hydrate_bible output schema still has numeric formats: {offenders:?}"
    );
}

#[test]
fn strip_nonstandard_formats_removes_denylisted_numeric_formats() {
    // Nested $defs, anyOf branches, arrays-of-schemas, items,
    // additionalProperties — the scrubber must reach all of them.
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "created": {"type": "integer", "format": "uint"},
            "ratio": {"type": "number", "format": "double"},
            "count32": {"type": "integer", "format": "uint32"},
            "signed": {"type": "integer", "format": "int64"},
            "tags": {
                "type": "array",
                "items": {"type": "integer", "format": "uint8"}
            },
            "maybe": {
                "anyOf": [
                    {"type": "integer", "format": "int16"},
                    {"type": "null"}
                ]
            },
            "extra": {
                "type": "object",
                "additionalProperties": {"type": "integer", "format": "usize"}
            }
        },
        "$defs": {
            "Row": {
                "type": "object",
                "properties": {
                    "n": {"type": "integer", "format": "uint64"},
                    "f": {"type": "number", "format": "float"}
                }
            }
        }
    });

    strip_nonstandard_formats(&mut schema);

    // No denylisted format survives anywhere.
    let mut found = Vec::new();
    collect_format_values(&schema, "#", &mut found);
    let survivors: Vec<_> = found
        .iter()
        .filter(|(_, f)| is_denylisted_numeric_format(f))
        .collect();
    assert!(
        survivors.is_empty(),
        "denylisted formats survived scrubbing: {survivors:?}"
    );

    // Note: "usize" is not `^u?int\d*$`, so it is intentionally NOT stripped
    // by the denylist. Assert it is untouched (schemars does not emit it, but
    // this documents the exact denylist boundary).
    assert_eq!(
        schema.pointer("/properties/extra/additionalProperties/format"),
        Some(&Value::String("usize".to_string())),
        "usize is outside the numeric denylist and must be preserved"
    );

    // Structure is otherwise byte-identical: types, properties, $defs remain.
    assert_eq!(
        schema.pointer("/properties/created/type"),
        Some(&Value::String("integer".to_string()))
    );
    assert!(schema.pointer("/$defs/Row/properties/n").is_some());
    assert!(schema.pointer("/properties/maybe/anyOf/0/type").is_some());
    assert!(schema.pointer("/properties/tags/items/type").is_some());
}

#[test]
fn strip_nonstandard_formats_preserves_standard_formats() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "when": {"type": "string", "format": "date-time"},
            "id": {"type": "string", "format": "uuid"},
            "link": {"type": "string", "format": "uri"},
            "mail": {"type": "string", "format": "email"},
            "pat": {"type": "string", "format": "regex"}
        }
    });
    let before = schema.clone();
    strip_nonstandard_formats(&mut schema);
    assert_eq!(
        schema, before,
        "standard string formats must survive untouched"
    );
}

#[test]
fn strip_nonstandard_formats_ignores_unrelated_format_valued_data() {
    // A `format` key whose value is NOT a denylisted numeric string must be
    // left alone — e.g. a description literal or an unrelated enum member.
    // (A schema document cannot contain a data object with a coincidental
    // numeric-format-valued "format" key, so the value-based rule is safe.)
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "output": {"type": "string", "format": "markdown"},
            "note": {"type": "string", "description": "format: uint is a warning"}
        }
    });
    let before = schema.clone();
    strip_nonstandard_formats(&mut schema);
    assert_eq!(
        schema, before,
        "non-denylisted format values and description text must be preserved"
    );
}

#[test]
fn structured_result_flattens_nested_record_ids() {
    let payload = serde_json::json!({
        "id": {"tb": "scene", "id": {"String": "abc123"}},
        "nested": {
            "character_id": {"tb": "character", "id": "mara"}
        },
        "items": [
            {"tb": "world_rule", "id": {"String": "law-1"}}
        ]
    });

    let result = structured_result(&payload).expect("structured result");

    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.expect("structured content");
    assert_eq!(structured["id"], serde_json::json!("scene:abc123"));
    assert_eq!(
        structured["nested"]["character_id"],
        serde_json::json!("character:mara")
    );
    assert_eq!(
        structured["items"][0],
        serde_json::json!("world_rule:law-1")
    );
}

#[test]
fn parse_arguments_coerces_stringified_arrays_for_schema_arrays() {
    let args = serde_json::json!({
        "project_id": "project:test",
        "book_number": "1",
        "chapter_number": "2",
        "scene_order": "3",
        "character_ids": "[\"character:alpha\",\"character:beta\"]",
        "location_id": "location:arena",
        "sections": "[\"scene\",\"world_rules\"]"
    });
    let args = args
        .as_object()
        .cloned()
        .expect("tool args should be object");

    let parsed: GetSceneContextInput =
        parse_arguments(Some(&args)).expect("arguments should coerce");

    assert_eq!(parsed.book_number, 1);
    assert_eq!(parsed.chapter_number, 2);
    assert_eq!(parsed.scene_order, 3);
    assert_eq!(
        parsed.character_ids,
        vec!["character:alpha".to_string(), "character:beta".to_string()]
    );
    assert_eq!(
        parsed.sections,
        Some(vec!["scene".to_string(), "world_rules".to_string()])
    );
}

#[test]
fn parse_arguments_keeps_mixed_type_update_entity_changes_object() {
    // Regression: a mixed-type payload — one string field plus one array
    // field in the same `changes` object — was rejected upstream with
    // "update_entity changes must be a JSON object" despite changes being
    // a valid object. Schema coercion must not reshape or drop a
    // heterogeneous change map, and a stringified-object form (some
    // clients stringify nested objects) must be parsed back.
    let args = serde_json::json!({
        "entity_type": "character",
        "entity_id": "character:mara",
        "changes": {
            "summary": "A smuggler with a salt blade.",
            "aliases": ["The Salt Blade", "Gull"]
        }
    });
    let args = args
        .as_object()
        .cloned()
        .expect("tool args should be object");
    let parsed: UpdateEntityInput = parse_arguments(Some(&args)).expect("arguments should parse");
    assert_eq!(parsed.entity_type, "character");
    let changes = parsed
        .changes
        .as_object()
        .expect("changes must remain a JSON object");
    assert_eq!(
        changes["summary"],
        serde_json::json!("A smuggler with a salt blade.")
    );
    assert_eq!(
        changes["aliases"],
        serde_json::json!(["The Salt Blade", "Gull"])
    );

    // Stringified-object form coerces back to an object.
    let args = serde_json::json!({
        "entity_type": "character",
        "entity_id": "character:mara",
        "changes": "{\"summary\":\"S\",\"aliases\":[\"A\"]}"
    });
    let args = args
        .as_object()
        .cloned()
        .expect("tool args should be object");
    let parsed: UpdateEntityInput =
        parse_arguments(Some(&args)).expect("stringified changes should coerce");
    assert!(
        parsed.changes.is_object(),
        "stringified changes must coerce to an object"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn tool_schemas_relax_session_default_project_and_branch_fields() {
    let router = router().await;
    let tools = router.list_tools();
    let required_fields = |tool: &Tool| {
        tool.input_schema
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };

    let create_character = tools
        .iter()
        .find(|tool| tool.name == "create_character")
        .expect("create_character tool");
    let create_character_required = required_fields(create_character);
    assert!(
        !create_character_required
            .iter()
            .any(|entry| entry.as_str() == Some("project_id"))
    );

    let update_writer_position = tools
        .iter()
        .find(|tool| tool.name == "update_writer_position")
        .expect("update_writer_position tool");
    let update_required = required_fields(update_writer_position);
    assert!(
        !update_required
            .iter()
            .any(|entry| entry.as_str() == Some("project_id"))
    );
    assert!(
        !update_required
            .iter()
            .any(|entry| entry.as_str() == Some("branch_id"))
    );

    let batch_create_terms = tools
        .iter()
        .find(|tool| tool.name == "batch_create_terms")
        .expect("batch_create_terms tool");
    let batch_terms_required = required_fields(batch_create_terms);
    assert!(
        !batch_terms_required
            .iter()
            .any(|entry| entry.as_str() == Some("project_id"))
    );

    let batch_set_voice_profiles = tools
        .iter()
        .find(|tool| tool.name == "batch_set_character_voice_profiles")
        .expect("batch_set_character_voice_profiles tool");
    let batch_voice_required = required_fields(batch_set_voice_profiles);
    assert!(
        !batch_voice_required
            .iter()
            .any(|entry| entry.as_str() == Some("project_id"))
    );
    assert!(
        !batch_voice_required
            .iter()
            .any(|entry| entry.as_str() == Some("branch_id"))
    );

    let set_active_project = tools
        .iter()
        .find(|tool| tool.name == "set_active_project")
        .expect("set_active_project tool");
    let set_active_required = required_fields(set_active_project);
    assert!(
        set_active_required
            .iter()
            .any(|entry| entry.as_str() == Some("project_id"))
    );
}
