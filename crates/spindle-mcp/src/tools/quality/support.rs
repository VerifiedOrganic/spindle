use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(super) struct Issue {
    pub pointer: String,
    pub rule: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Budget {
    pub tools: usize,
    pub input_bytes: usize,
    pub full_bytes: usize,
}

pub(super) fn measure(tools: &[Value]) -> (usize, usize, usize) {
    let inputs: Vec<_> = tools.iter().map(|tool| serde_json::json!({
        "name": tool["name"], "description": tool["description"], "inputSchema": tool["inputSchema"]
    })).collect();
    (
        tools.len(),
        serde_json::to_vec(&inputs).unwrap().len(),
        serde_json::to_vec(tools).unwrap().len(),
    )
}

pub(super) fn check_budget(tools: &[Value], budget: &Budget) -> Result<(), String> {
    let (count, input, full) = measure(tools);
    if count > budget.tools || input > budget.input_bytes || full > budget.full_bytes {
        let mut sizes: Vec<_> = tools
            .iter()
            .map(|tool| {
                (
                    serde_json::to_vec(tool).unwrap().len(),
                    tool["name"].as_str().unwrap_or("?"),
                )
            })
            .collect();
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        return Err(format!(
            "catalog {count} tools/{input} input bytes/{full} full bytes exceeds {budget:?}; largest tools: {sizes:?}"
        ));
    }
    Ok(())
}

fn escaped(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn resolve<'a>(mut node: &'a Value, root: &'a Value) -> Option<&'a Value> {
    let mut seen = BTreeSet::new();
    while let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        if !seen.insert(reference) {
            return Some(node);
        }
        node = root.pointer(reference.strip_prefix('#')?)?;
    }
    Some(node)
}

fn description<'a>(node: &'a Value, root: &'a Value) -> Option<&'a str> {
    node.get("description")
        .and_then(Value::as_str)
        .or_else(|| resolve(node, root)?.get("description")?.as_str())
}

fn has_enum(node: &Value, root: &Value) -> bool {
    let Some(node) = resolve(node, root) else {
        return false;
    };
    node.get("enum")
        .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()))
        || node.get("const").is_some()
}

fn is_string(node: &Value, root: &Value) -> bool {
    resolve(node, root).is_some_and(|node| match &node["type"] {
        Value::String(s) => s == "string",
        Value::Array(types) => types.iter().any(|t| t == "string"),
        _ => false,
    })
}

fn description_issues(
    text: Option<&str>,
    field: Option<&str>,
    pointer: &str,
    issues: &mut Vec<Issue>,
) {
    let Some(text) = text.filter(|s| !s.trim().is_empty()) else {
        issues.push(Issue {
            pointer: pointer.to_owned(),
            rule: "missing_description",
        });
        return;
    };
    if field.is_some_and(|field| {
        text.trim()
            .trim_end_matches('.')
            .replace(['_', '-'], " ")
            .eq_ignore_ascii_case(&field.replace(['_', '-'], " "))
    }) {
        issues.push(Issue {
            pointer: pointer.to_owned(),
            rule: "placeholder_description",
        });
    }
}

pub(super) fn lint_schema(root: &Value) -> Vec<Issue> {
    fn walk(
        node: &Value,
        root: &Value,
        pointer: &str,
        seen: &mut BTreeSet<String>,
        issues: &mut Vec<Issue>,
    ) {
        if !seen.insert(pointer.to_owned()) {
            return;
        }
        if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
            match reference
                .strip_prefix('#')
                .and_then(|p| root.pointer(p).map(|target| (p, target)))
            {
                Some((p, target)) => walk(target, root, p, seen, issues),
                None => issues.push(Issue {
                    pointer: format!("{pointer}/$ref"),
                    rule: "unresolved_ref",
                }),
            }
        }
        if let Some(text) = node.get("description").and_then(Value::as_str) {
            static JARGON: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            let jargon = JARGON.get_or_init(|| {
                regex::Regex::new(r"(?i)evolution\s*(?:§|I\d)|\bADR[ -]?\d+|\(I\d+\)").unwrap()
            });
            let lower = text.to_ascii_lowercase();
            if ["using gemini", "via gemini"]
                .iter()
                .any(|pattern| lower.contains(pattern))
            {
                issues.push(Issue {
                    pointer: format!("{pointer}/description"),
                    rule: "provider_promise",
                });
            }
            if jargon.is_match(text) {
                issues.push(Issue {
                    pointer: format!("{pointer}/description"),
                    rule: "implementation_jargon",
                });
            }
        }
        for keyword in [
            "properties",
            "patternProperties",
            "$defs",
            "definitions",
            "dependentSchemas",
        ] {
            if let Some(map) = node.get(keyword).and_then(Value::as_object) {
                for (name, child) in map {
                    let p = format!("{pointer}/{keyword}/{}", escaped(name));
                    if keyword == "properties" {
                        description_issues(description(child, root), Some(name), &p, issues);
                        if (name.ends_with("_policy")
                            || ["mode", "status", "action"].contains(&name.as_str()))
                            && is_string(child, root)
                            && !has_enum(child, root)
                        {
                            issues.push(Issue {
                                pointer: p.clone(),
                                rule: "string_vocabulary_review",
                            });
                        }
                    }
                    walk(child, root, &p, seen, issues);
                }
            }
        }
        for keyword in [
            "items",
            "additionalProperties",
            "unevaluatedProperties",
            "contains",
            "propertyNames",
            "not",
            "if",
            "then",
            "else",
        ] {
            if let Some(child) = node.get(keyword) {
                walk(child, root, &format!("{pointer}/{keyword}"), seen, issues);
            }
        }
        for keyword in ["anyOf", "oneOf", "allOf", "prefixItems"] {
            if let Some(children) = node.get(keyword).and_then(Value::as_array) {
                for (index, child) in children.iter().enumerate() {
                    walk(
                        child,
                        root,
                        &format!("{pointer}/{keyword}/{index}"),
                        seen,
                        issues,
                    );
                }
            }
        }
    }
    let mut issues = Vec::new();
    walk(root, root, "", &mut BTreeSet::new(), &mut issues);
    issues.sort();
    issues.dedup();
    issues
}

pub(super) fn lint_tool(tool: &Value) -> Vec<Issue> {
    let mut issues = lint_schema(tool);
    description_issues(
        tool.get("description").and_then(Value::as_str),
        tool.get("name").and_then(Value::as_str),
        "/description",
        &mut issues,
    );
    for key in ["inputSchema", "outputSchema"] {
        if let Some(schema) = tool.get(key) {
            for issue in lint_schema(schema) {
                issues.push(Issue {
                    pointer: format!("/{key}{}", issue.pointer),
                    rule: issue.rule,
                });
            }
        }
    }
    issues.sort();
    issues
}

pub(super) fn canonical_tool_issues(tool: &Value) -> Vec<Issue> {
    let mut issues = lint_tool(tool);
    if serde_json::to_vec(tool).unwrap().len() > 4096 {
        issues.push(Issue {
            pointer: String::new(),
            rule: "discovery_size",
        });
    }
    issues
}

pub(super) fn closed_field_issues(schema: &Value, pointers: &[String]) -> Vec<Issue> {
    pointers
        .iter()
        .filter_map(|pointer| {
            let Some(node) = schema.pointer(pointer) else {
                return Some(Issue {
                    pointer: pointer.clone(),
                    rule: "closed_set_missing_field",
                });
            };
            (!has_enum(node, schema)).then(|| Issue {
                pointer: pointer.clone(),
                rule: "closed_set_missing_enum",
            })
        })
        .collect()
}
