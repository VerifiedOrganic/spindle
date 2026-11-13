use super::static_tool_names;
use rmcp::model::{CallToolResult, Content};

/// Tools that are operator-only. They remain in the registry and disposition
/// ledger so existing operator clients can be migrated, but they are hidden from
/// every model profile and denied by name on stdio/HTTP model transports. Use
/// the `spindle-mcp` CLI commands instead.
pub(crate) const OPERATOR_ONLY_TOOLS: &[&str] = &[
    "init_grok_skills",
    "configure_agents",
    "test_agent",
    "rebuild_search_index",
    "backfill_scene_source_offsets",
    "operator_delete_scene",
];

/// True when `name` is an operator-only command that model transports must not
/// list or call.
pub(crate) fn is_operator_only(name: &str) -> bool {
    OPERATOR_ONLY_TOOLS.contains(&name)
}

/// Tool names enabled by a generated profile. `None` means the profile is
/// unknown and fails closed. The unset startup profile is resolved to
/// `workflow-write` before the server constructs its router.
pub(crate) fn profile_allowed_tool_names(profile: &str) -> Option<&'static [&'static str]> {
    spindle_adapters::guidance::list_profiles()
        .iter()
        .find(|candidate| candidate.name == profile)
        .map(|profile| profile.tools)
}

/// Startup profile names. R+1 removes the legacy compatibility profiles; only
/// generated workflow/specialist profiles are selectable.
pub(crate) fn profile_names() -> Vec<&'static str> {
    let mut names: Vec<_> = spindle_adapters::guidance::list_profiles()
        .iter()
        .map(|profile| profile.name)
        .collect();
    names.sort_unstable();
    names
}

pub(crate) fn validate_tool_profile(profile: Option<&str>) -> anyhow::Result<()> {
    if let Some(profile) = profile {
        anyhow::ensure!(
            profile_names().contains(&profile),
            "unknown tool profile '{profile}'; choose one of: {}. Unset SPINDLE_TOOL_PROFILE uses the generated workflow-write profile",
            profile_names().join(", ")
        );
    }
    Ok(())
}

/// Whether a model transport may list or call `name` under `profile`. A `None`
/// profile is the internal/test unfiltered router; the startup server resolves
/// unset to `workflow-write` before constructing it.
pub(crate) fn profile_has_tool(profile: Option<&str>, name: &str) -> bool {
    if !static_tool_names().contains(name) || is_operator_only(name) {
        return false;
    }
    match profile {
        None => true,
        Some(profile) => {
            profile_allowed_tool_names(profile).is_some_and(|allowed| allowed.contains(&name))
        }
    }
}

pub(crate) fn profile_help() -> String {
    format!(
        "Select a startup profile with SPINDLE_TOOL_PROFILE, or switch a running session with the use_profile tool. Profiles: {}. Unset starts the generated workflow-write profile; operator-only commands are CLI-only and denied on model transports. Run 'spindle-mcp profiles' for units and capabilities.",
        profile_names().join(", ")
    )
}

/// Structured, model-readable rejection for a profile switch. The MCP tool
/// result stays an ordinary error result, but `structuredContent.error.code`
/// lets clients distinguish an unknown name from an unavailable proxy target.
pub(crate) fn profile_switch_error(code: &str, profile: &str, message: &str) -> CallToolResult {
    let structured = serde_json::json!({
        "error": {
            "code": code,
            "profile": profile,
            "message": message,
            "available_profiles": profile_names(),
        }
    });
    let mut result = CallToolResult::error(vec![Content::text(format!("Error: {message}"))]);
    result.structured_content = Some(structured);
    result
}

pub(crate) fn print_profiles(json: bool) {
    let generated = spindle_adapters::guidance::list_profiles();
    if json {
        let entries: Vec<_> = generated
            .iter()
            .map(|profile| {
                serde_json::json!({
                    "name": profile.name,
                    "description": profile.description,
                    "units": profile.units,
                    "tools": profile.tools,
                    "catalog": "generated"
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).expect("profile catalog")
        );
    } else {
        let default_tools = generated
            .iter()
            .find(|profile| profile.name == "workflow-write")
            .map(|profile| profile.tools.len())
            .unwrap_or(0);
        println!("Unset profile: generated workflow-write ({default_tools} tools).");
        for profile in generated {
            println!(
                "{} ({} tools): {} Units: {}",
                profile.name,
                profile.tools.len(),
                profile.description,
                profile.units.join(", ")
            );
        }
    }
}
