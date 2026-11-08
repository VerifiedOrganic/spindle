use super::*;

#[tokio::test(flavor = "current_thread")]
async fn list_skills_returns_embedded_skills_and_references() {
    let router = router().await;

    let result = router
        .call_tool("list_skills", None)
        .await
        .expect("tool call should return result");

    assert_eq!(result.is_error, Some(false));
    let content = result
        .structured_content
        .expect("list_skills should use structured content");
    let skill_names: Vec<&str> = content["skills"]
        .as_array()
        .expect("skills array")
        .iter()
        .filter_map(|skill| skill["name"].as_str())
        .collect();
    for expected in [
        "spindle",
        "scene-writer",
        "character-creator",
        "worldbuilder",
        "authoring-supervisor",
    ] {
        assert!(
            skill_names.contains(&expected),
            "missing skill {expected} in: {skill_names:?}"
        );
    }
    let reference_names: Vec<&str> = content["references"]
        .as_array()
        .expect("references array")
        .iter()
        .filter_map(|reference| reference["name"].as_str())
        .collect();
    assert!(
        reference_names.contains(&"anti-slop"),
        "missing reference anti-slop in: {reference_names:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn get_skill_returns_markdown_and_unknown_skill_errors() {
    let router = router().await;
    let mut args = serde_json::Map::new();
    args.insert("name".to_string(), "scene-writer".into());

    let result = router
        .call_tool("get_skill", Some(&args))
        .await
        .expect("tool call should return result");

    assert_eq!(result.is_error, Some(false));
    let content = result
        .structured_content
        .expect("get_skill should use structured content");
    assert_eq!(content["name"].as_str(), Some("scene-writer"));
    assert!(
        content["markdown"]
            .as_str()
            .is_some_and(|markdown| !markdown.is_empty()),
        "markdown should be non-empty"
    );

    args.insert("name".to_string(), "not-a-skill".into());
    let result = router
        .call_tool("get_skill", Some(&args))
        .await
        .expect("tool call should return result");
    assert_eq!(result.is_error, Some(true));
    let text = format!("{:?}", result.content.first().expect("error content"));
    assert!(
        text.contains("unknown skill: not-a-skill"),
        "expected error text in: {text}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn get_reference_returns_markdown_and_unknown_reference_errors() {
    let router = router().await;
    let mut args = serde_json::Map::new();
    args.insert("name".to_string(), "anti-slop".into());

    let result = router
        .call_tool("get_reference", Some(&args))
        .await
        .expect("tool call should return result");

    assert_eq!(result.is_error, Some(false));
    let content = result
        .structured_content
        .expect("get_reference should use structured content");
    assert_eq!(content["name"].as_str(), Some("anti-slop"));
    assert!(
        content["markdown"]
            .as_str()
            .is_some_and(|markdown| !markdown.is_empty()),
        "markdown should be non-empty"
    );

    args.insert("name".to_string(), "not-a-reference".into());
    let result = router
        .call_tool("get_reference", Some(&args))
        .await
        .expect("tool call should return result");
    assert_eq!(result.is_error, Some(true));
}
