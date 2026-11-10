use super::*;

#[tokio::test]
async fn test_style_profile_mcp_tools() {
    let router = router().await;
    let tools = router.list_tools();
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(tool_names.contains(&"create_style_profile_from_markdown"));
    assert!(tool_names.contains(&"list_style_profiles"));
    assert!(tool_names.contains(&"get_style_profile"));
    assert!(tool_names.contains(&"apply_style_profile"));
    assert!(tool_names.contains(&"list_style_profile_applications"));
    assert!(tool_names.contains(&"rollback_style_profile_application"));
    assert!(tool_names.contains(&"check_style_profile_sources"));
    assert!(tool_names.contains(&"refresh_style_profile"));
    assert!(tool_names.contains(&"check_style_against_profile"));
    assert!(tool_names.contains(&"compare_style_profiles"));
    assert!(tool_names.contains(&"archive_style_profile"));
    assert!(tool_names.contains(&"plan_style_revision"));
    assert!(tool_names.contains(&"create_style_revision_patch"));
    assert!(tool_names.contains(&"evaluate_style_revision_patch"));
    assert!(tool_names.contains(&"apply_style_revision_patch"));
    assert!(tool_names.contains(&"list_style_revision_patch_audits"));
    assert!(tool_names.contains(&"rollback_style_revision_patch"));
}
