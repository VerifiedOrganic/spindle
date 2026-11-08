use super::*;

pub(super) fn relax_session_default_fields(tool_name: &str, value: &mut Value) {
    let Some(schema_obj) = value.as_object_mut() else {
        return;
    };
    let Some(required) = schema_obj.get_mut("required").and_then(Value::as_array_mut) else {
        return;
    };

    required.retain(|field| match field.as_str() {
        Some("project_id") if tool_supports_session_project_default(tool_name) => false,
        Some("branch_id") if tool_supports_session_branch_default(tool_name) => false,
        _ => true,
    });

    if required.is_empty() {
        schema_obj.remove("required");
    }
}

pub(super) fn tool<I, O>(name: &'static str, description: &'static str) -> Tool
where
    I: schemars::JsonSchema + 'static,
    O: schemars::JsonSchema + 'static,
{
    let settings = SchemaSettings::openapi3().with(|s| {
        s.inline_subschemas = true;
    });
    let generator = settings.into_generator();
    let schema = generator.into_root_schema_for::<I>();
    let mut value = serde_json::to_value(&schema).expect("schema to json");
    relax_session_default_fields(name, &mut value);
    sanitize_for_gemini(&mut value);
    // Scrub schemars' non-standard numeric `format` annotations so strict
    // JSON-Schema clients do not warn per count-typed field. This is the single
    // finalization chokepoint: every tool served over stdio and HTTP flows
    // through here for both its input and output schema.
    strip_nonstandard_formats(&mut value);
    strip_null_enum_values(&mut value);
    let object = value
        .as_object()
        .cloned()
        .unwrap_or_else(|| panic!("expected object schema for tool input"));

    let tool = Tool::new(name, description, object).with_output_schema::<O>();
    scrub_output_schema_formats(tool)
}

/// Apply [`strip_nonstandard_formats`] to a tool's already-generated output
/// schema (rmcp builds it via its own draft-2020-12 generator, so it never
/// passes through the input-schema scrub above). Returns the tool unchanged
/// when it has no output schema.
pub(super) fn scrub_output_schema_formats(tool: Tool) -> Tool {
    let Some(output) = tool.output_schema.as_ref() else {
        return tool;
    };
    let mut value = Value::Object((**output).clone());
    strip_nonstandard_formats(&mut value);
    strip_null_enum_values(&mut value);
    let object = value
        .as_object()
        .cloned()
        .unwrap_or_else(|| panic!("expected object schema for tool output"));
    tool.with_raw_output_schema(std::sync::Arc::new(object))
}

/// Recursively strip schemars' non-standard numeric `format` annotations from a
/// JSON Schema value so strict JSON-Schema clients (Kimi Code and any
/// spec-compliant validator) do not warn on every count-typed field.
///
/// Denylist rule (value-based, not key-position-based): remove any object's
/// `"format"` key whose value is a schemars numeric format —
/// `float`, `double`, or `^u?int\d*$` (`int`, `uint`, `int8`..`int64`,
/// `uint8`..`uint64`). Standard formats (`date-time`, `uuid`, `uri`, `email`,
/// `regex`, …) and any other future format are preserved. A JSON Schema
/// document cannot contain a data object with a coincidental numeric-format
/// `"format"` value, so matching on the value is safe. Structure is otherwise
/// untouched — types, properties, `$defs`, `required`, descriptions stay
/// byte-identical.
pub(super) fn strip_nonstandard_formats(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map
                .get("format")
                .and_then(Value::as_str)
                .is_some_and(is_nonstandard_numeric_format)
            {
                map.remove("format");
            }
            for child in map.values_mut() {
                strip_nonstandard_formats(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_nonstandard_formats(item);
            }
        }
        _ => {}
    }
}

/// Remove `null` entries from `"enum"` arrays anywhere in a schema tree.
///
/// schemars emits `"enum": [..., null]` for `Option<Enum>` fields. MCP
/// consumers vary: Anthropic tolerates the null entry; Moonshot's validator
/// rejects it (observed live: `enum value (<nil>) does not match any type in
/// [string]` at `properties.canonical_facts.items.properties.scope.enum`).
/// Optionality is already expressed by the field's absence from `required`,
/// and serde accepts explicit nulls for `Option` fields regardless of the
/// advertised schema, so dropping the null tightens the advertisement without
/// changing server behavior. An enum emptied by the removal is dropped.
pub(super) fn strip_null_enum_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let emptied = if let Some(Value::Array(items)) = map.get_mut("enum") {
                items.retain(|v| !v.is_null());
                items.is_empty()
            } else {
                false
            };
            if emptied {
                map.remove("enum");
            }
            for child in map.values_mut() {
                strip_null_enum_values(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_null_enum_values(item);
            }
        }
        _ => {}
    }
}

/// Returns true for schemars' non-standard numeric `format` values: `float`,
/// `double`, or `int`/`uint` optionally followed by only digits (`int8`,
/// `uint16`, `int32`, `uint64`, …). Matches the regex `^(u?int\d*|float|double)$`.
pub(super) fn is_nonstandard_numeric_format(s: &str) -> bool {
    if s == "float" || s == "double" {
        return true;
    }
    let rest = match s.strip_prefix("uint").or_else(|| s.strip_prefix("int")) {
        Some(rest) => rest,
        None => return false,
    };
    rest.chars().all(|c| c.is_ascii_digit())
}

/// Post-process a JSON Schema value to be compatible with Gemini's strict subset.
/// Removes `$defs`, resolves remaining `anyOf`/`oneOf` nullable patterns,
/// strips unsupported keywords, and converts array `type` to single string.
pub(super) fn sanitize_for_gemini(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // Remove unsupported top-level keywords
            map.remove("$schema");
            map.remove("$defs");
            map.remove("definitions");
            map.remove("$ref");
            map.remove("format");
            map.remove("default");
            map.remove("const");
            map.remove("nullable");
            map.remove("examples");
            map.remove("$id");

            // Convert type arrays like ["string", "null"] to just "string"
            if let Some(ty) = map.get_mut("type")
                && let Some(arr) = ty.as_array().cloned()
            {
                let non_null: Vec<_> = arr
                    .into_iter()
                    .filter(|v| v.as_str() != Some("null"))
                    .collect();
                if non_null.len() == 1 {
                    *ty = non_null.into_iter().next().unwrap();
                }
            }

            // Flatten anyOf where one branch is null (nullable pattern)
            if let Some(any_of) = map.remove("anyOf")
                && let Some(branches) = any_of.as_array()
            {
                let non_null: Vec<_> = branches
                    .iter()
                    .filter(|b| {
                        b.get("type").and_then(Value::as_str) != Some("null")
                            && b.get("const") != Some(&Value::Null)
                            && !b.as_object().is_some_and(|o| o.is_empty())
                    })
                    .collect();
                if non_null.len() == 1 {
                    // Merge the single non-null branch into this schema
                    if let Some(obj) = non_null[0].as_object() {
                        for (k, v) in obj {
                            map.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                } else if !non_null.is_empty() {
                    // Multiple real branches — keep as anyOf (best effort)
                    map.insert(
                        "anyOf".to_string(),
                        Value::Array(non_null.into_iter().cloned().collect()),
                    );
                }
            }

            // Flatten oneOf similarly
            if let Some(one_of) = map.remove("oneOf")
                && let Some(branches) = one_of.as_array()
            {
                let non_null: Vec<_> = branches
                    .iter()
                    .filter(|b| {
                        b.get("type").and_then(Value::as_str) != Some("null")
                            && b.get("const") != Some(&Value::Null)
                    })
                    .collect();
                if non_null.len() == 1 {
                    if let Some(obj) = non_null[0].as_object() {
                        for (k, v) in obj {
                            map.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                } else if !non_null.is_empty() {
                    // Convert tagged enum oneOf to a plain object schema
                    // (Gemini can't handle oneOf, so we just accept any object)
                    map.insert("type".to_string(), Value::String("object".to_string()));
                }
            }

            // Ensure object schemas with properties have a type
            if map.contains_key("properties") && !map.contains_key("type") {
                map.insert("type".to_string(), Value::String("object".to_string()));
            }

            // Recurse into all remaining values
            for v in map.values_mut() {
                sanitize_for_gemini(v);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                sanitize_for_gemini(v);
            }
        }
        _ => {}
    }
}

pub(super) fn parse_arguments<T>(arguments: Option<&rmcp::model::JsonObject>) -> anyhow::Result<T>
where
    T: DeserializeOwned + schemars::JsonSchema,
{
    let mut value = match arguments {
        Some(args) => Value::Object(args.clone()),
        None => Value::Object(Default::default()),
    };
    let schema = SchemaSettings::openapi3()
        .with(|s| {
            s.meta_schema = None;
        })
        .into_generator()
        .into_root_schema_for::<T>();
    coerce_value_for_schema(&mut value, schema.as_value(), schema.as_value());
    Ok(serde_json::from_value(value)?)
}

pub(super) fn coerce_value_for_schema(value: &mut Value, schema: &Value, root_schema: &Value) {
    let schema = resolve_schema_refs(schema, root_schema);
    let Some(schema_obj) = schema.as_object() else {
        return;
    };

    if let Some(all_of) = schema_obj.get("allOf").and_then(Value::as_array) {
        for subschema in all_of {
            coerce_value_for_schema(value, subschema, root_schema);
        }
    }

    if matches!(value, Value::String(_))
        && let Some(coerced) = coerce_string_value_for_schema(value, schema_obj, root_schema)
    {
        *value = coerced;
    }

    if value.is_object() {
        for keyword in ["anyOf", "oneOf"] {
            if let Some(candidates) = schema_obj.get(keyword).and_then(Value::as_array) {
                for candidate in candidates {
                    coerce_value_for_schema(value, candidate, root_schema);
                }
            }
        }
    }

    match value {
        Value::Object(map) => {
            let properties = schema_obj.get("properties").and_then(Value::as_object);
            let additional = schema_obj.get("additionalProperties");
            for (key, child) in map {
                if let Some(schema) = properties.and_then(|properties| properties.get(key)) {
                    coerce_value_for_schema(child, schema, root_schema);
                } else if let Some(schema) = additional {
                    coerce_value_for_schema(child, schema, root_schema);
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema_obj.get("items") {
                for item in items {
                    coerce_value_for_schema(item, item_schema, root_schema);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn coerce_string_value_for_schema(
    value: &Value,
    schema_obj: &serde_json::Map<String, Value>,
    root_schema: &Value,
) -> Option<Value> {
    let Value::String(raw) = value else {
        return None;
    };

    if schema_allows_type(schema_obj, "string") {
        return None;
    }

    for keyword in ["anyOf", "oneOf"] {
        if let Some(candidates) = schema_obj.get(keyword).and_then(Value::as_array) {
            if candidates.iter().any(|candidate| {
                resolve_schema_refs(candidate, root_schema)
                    .as_object()
                    .is_some_and(|candidate| schema_allows_type(candidate, "string"))
            }) {
                return None;
            }
            for candidate in candidates {
                let candidate = resolve_schema_refs(candidate, root_schema);
                if let Some(candidate_obj) = candidate.as_object()
                    && let Some(coerced) =
                        coerce_string_value_for_schema(value, candidate_obj, root_schema)
                {
                    return Some(coerced);
                }
            }
        }
    }

    if schema_obj
        .get("nullable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && raw.eq_ignore_ascii_case("null")
    {
        return Some(Value::Null);
    }

    if schema_allows_type(schema_obj, "integer") {
        if let Ok(parsed) = raw.parse::<i64>() {
            return Some(Value::Number(Number::from(parsed)));
        }
        if let Ok(parsed) = raw.parse::<u64>() {
            return Some(Value::Number(Number::from(parsed)));
        }
    }

    if schema_allows_type(schema_obj, "number")
        && let Ok(parsed) = raw.parse::<f64>()
        && let Some(number) = Number::from_f64(parsed)
    {
        return Some(Value::Number(number));
    }

    if schema_allows_type(schema_obj, "boolean") {
        match raw.as_str() {
            "true" => return Some(Value::Bool(true)),
            "false" => return Some(Value::Bool(false)),
            _ => {}
        }
    }

    if schema_allows_type(schema_obj, "null") && raw.eq_ignore_ascii_case("null") {
        return Some(Value::Null);
    }

    if (schema_allows_type(schema_obj, "array") || schema_allows_type(schema_obj, "object"))
        && let Ok(parsed) = serde_json::from_str::<Value>(raw)
        && ((parsed.is_array() && schema_allows_type(schema_obj, "array"))
            || (parsed.is_object() && schema_allows_type(schema_obj, "object")))
    {
        return Some(parsed);
    }

    None
}

pub(super) fn schema_allows_type(
    schema_obj: &serde_json::Map<String, Value>,
    target: &str,
) -> bool {
    schema_obj.get("type").is_some_and(|value| match value {
        Value::String(kind) => kind == target,
        Value::Array(kinds) => kinds.iter().any(|kind| kind.as_str() == Some(target)),
        _ => false,
    })
}

pub(super) fn resolve_schema_refs<'a>(schema: &'a Value, root_schema: &'a Value) -> &'a Value {
    let mut current = schema;
    for _ in 0..8 {
        let Some(reference) = current
            .as_object()
            .and_then(|schema| schema.get("$ref"))
            .and_then(Value::as_str)
        else {
            break;
        };

        let Some(pointer) = reference.strip_prefix('#') else {
            break;
        };
        let Some(target) = root_schema.pointer(pointer) else {
            break;
        };
        current = target;
    }
    current
}

pub(super) fn structured_result<T>(value: &T) -> anyhow::Result<CallToolResult>
where
    T: Serialize,
{
    let mut structured = serde_json::to_value(value)?;
    flatten_record_ids(&mut structured);
    Ok(CallToolResult::structured(structured))
}

pub(super) fn structured_error_result(error: &anyhow::Error) -> CallToolResult {
    CallToolResult::error(vec![Content::text(format!("Error: {error:#}"))])
}
