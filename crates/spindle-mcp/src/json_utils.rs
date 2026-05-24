use serde_json::Value;

/// Recursively flatten SurrealDB `RecordId` objects into `"table:key"` strings.
///
/// SurrealDB serializes `RecordId` as `{"tb": "table", "id": {"String": "key"}}`.
/// LLMs cannot easily navigate this nesting, so we collapse it to `"table:key"`.
pub(crate) fn flatten_record_ids(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.len() == 2
                && let (Some(Value::String(table)), Some(id_val)) = (map.get("tb"), map.get("id"))
            {
                let key_str = match id_val {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    Value::Object(inner) => {
                        if inner.len() == 1 {
                            inner.values().next().and_then(|v| match v {
                                Value::String(s) => Some(s.clone()),
                                Value::Number(n) => Some(n.to_string()),
                                _ => None,
                            })
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(key) = key_str {
                    *value = Value::String(format!("{table}:{key}"));
                    return;
                }
            }
            for v in map.values_mut() {
                flatten_record_ids(v);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                flatten_record_ids(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flattens_surreal_record_id_with_nested_string_key() {
        let mut value = json!({
            "id": {"tb": "world_rule", "id": {"String": "5cegdv7f9a1w03uo5ft0"}},
            "project_id": {"tb": "project", "id": {"String": "abc123"}},
            "rule_name": "Pull Sequence",
            "description": "How pulls work"
        });
        flatten_record_ids(&mut value);
        assert_eq!(value["id"], json!("world_rule:5cegdv7f9a1w03uo5ft0"));
        assert_eq!(value["project_id"], json!("project:abc123"));
        assert_eq!(value["rule_name"], json!("Pull Sequence"));
    }

    #[test]
    fn flattens_surreal_record_id_with_plain_string_key() {
        let mut value = json!({"tb": "character", "id": "mara"});
        flatten_record_ids(&mut value);
        assert_eq!(value, json!("character:mara"));
    }

    #[test]
    fn flattens_record_ids_in_arrays() {
        let mut value = json!([
            {"tb": "world_rule", "id": {"String": "abc"}},
            {"tb": "world_rule", "id": {"String": "def"}}
        ]);
        flatten_record_ids(&mut value);
        assert_eq!(value, json!(["world_rule:abc", "world_rule:def"]));
    }

    #[test]
    fn preserves_non_record_id_objects() {
        let mut value = json!({
            "rule_name": "Pull Sequence",
            "nested": {"foo": "bar", "baz": 42},
            "tags": ["a", "b"]
        });
        let expected = value.clone();
        flatten_record_ids(&mut value);
        assert_eq!(value, expected);
    }

    #[test]
    fn flattens_numeric_record_id_keys() {
        let mut value = json!({"tb": "scene", "id": {"Number": 7}});
        flatten_record_ids(&mut value);
        assert_eq!(value, json!("scene:7"));
    }
}
