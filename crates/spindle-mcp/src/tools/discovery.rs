use super::*;

// Explicit policy: these return expanded context, analysis, alternatives, or
// style cards. Scene write/commit receipts remain inline, even when large.
const REFERENCED_OUTPUTS: &[&str] = &[
    "authoring_status",
    "create_style_profile_from_markdown",
    "generate_alternatives",
    "get_chapter_briefing",
    "get_character_snapshot",
    "get_entity",
    "get_scene_context",
    "get_style_profile",
    "get_writer_state",
    "import_analyze_character",
    "import_analyze_narrative",
    "import_compute_final_state",
    "import_consolidate_entities",
    "import_extract_entities",
    "import_extract_world",
    "import_status",
    "list_style_profiles",
    "prepare_scene",
    "preview_refresh_style_profile",
    "refresh_style_profile",
];

pub(crate) fn response_contract_names() -> &'static [&'static str] {
    REFERENCED_OUTPUTS
}

pub(crate) fn response_contract(name: &str) -> Option<&'static rmcp::model::JsonObject> {
    if !REFERENCED_OUTPUTS.contains(&name) {
        return None;
    }
    all_tool_schemas()
        .iter()
        .find(|t| t.name == name)?
        .output_schema
        .as_deref()
}

pub(crate) fn response_contract_index() -> ResponseContractIndex {
    ResponseContractIndex {
        version: 1,
        contracts: response_contract_names()
            .iter()
            .map(|name| ResponseContractEntry {
                tool_name: (*name).to_owned(),
                uri: format!("bible://contracts/tools/v1/{name}"),
                reference_name: format!("contracts/tools/v1/{name}"),
            })
            .collect(),
    }
}

pub(super) fn response_contract_reference(
    router: &ToolRouter,
    name: &str,
) -> anyhow::Result<GetReferenceOutput> {
    let markdown = if name == "contracts/tools/v1" {
        let mut text = String::from(
            "# Full response contracts v1\n\nThese schemas document existing result shapes. Reading one does not enable its tool or run a workflow. Use the resource URI or load the reference name with get_reference.\n\n",
        );
        for entry in response_contract_index().contracts {
            text.push_str(&format!(
                "- `{}`: `{}` (resource `{}`)\n",
                entry.tool_name, entry.reference_name, entry.uri
            ));
        }
        text
    } else {
        let tool = name
            .strip_prefix("contracts/tools/v1/")
            .ok_or_else(|| anyhow::anyhow!("unknown response contract; use contracts/tools/v1"))?;
        let schema = response_contract(tool).ok_or_else(|| {
            anyhow::anyhow!("unknown v1 response contract: {tool}; use contracts/tools/v1")
        })?;
        format!(
            "# Full response contract v1: {tool}\n\nJSON Schema for the existing structured result. Discovery omission does not change this result or grant access to the tool.\n\n```json\n{}\n```\n",
            serde_json::to_string_pretty(schema)?
        )
    };
    Ok(GetReferenceOutput {
        name: name.to_owned(),
        markdown,
        covered: true,
        required_profile: Some("minimal".to_owned()),
        available_units: router.covered_units(),
        missing_capabilities: Vec::new(),
    })
}

fn generated_tool(mut tool: Tool) -> Tool {
    if REFERENCED_OUTPUTS.contains(&tool.name.as_ref()) {
        tool.output_schema = None;
        let uri = format!("bible://contracts/tools/v1/{}", tool.name);
        let description = tool.description.as_deref().unwrap_or_default();
        if !description.contains(&uri) {
            tool.description = Some(format!("{description} Full response schema: {uri} (also available through get_reference). ").trim_end().to_owned().into());
        }
        tool.meta.get_or_insert_default().insert("spindle/responseContract".to_owned(), serde_json::json!({"version":1, "uri":uri, "reference":format!("contracts/tools/v1/{}",tool.name)}));
    }
    if tool.name == "research_query" {
        tool.description = Some("Research a factual question through the configured research route, grounded in project context from the Bible".into());
    }
    if tool.name == "get_reference" {
        let mut input = (*tool.input_schema).clone();
        if let Some(field) = input
            .get_mut("properties")
            .and_then(|p| p.get_mut("name"))
            .and_then(Value::as_object_mut)
        {
            field.insert("description".to_owned(), Value::String("Reference name from list_skills, or contracts/tools/v1/<tool> for a full response schema.".to_owned()));
        }
        tool.input_schema = Arc::new(input);
    }
    tool
}

pub(crate) fn project_tool_discovery(profile: Option<&str>, tool: Tool) -> Tool {
    if profile.is_some_and(|name| {
        spindle_adapters::guidance::list_profiles()
            .iter()
            .any(|p| p.name == name)
    }) {
        generated_tool(tool)
    } else {
        tool
    }
}

pub(super) fn generated_tool_schemas() -> &'static [Tool] {
    static TOOLS: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    TOOLS.get_or_init(|| {
        all_tool_schemas()
            .iter()
            .cloned()
            .map(generated_tool)
            .collect()
    })
}
