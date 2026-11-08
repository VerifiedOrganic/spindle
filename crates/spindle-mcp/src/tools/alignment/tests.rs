use super::*;
use spindle_skills::{EmbeddedSkill, list_skills};
use std::collections::{BTreeMap, BTreeSet};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    version: u32,
    tools: Vec<Disposition>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Disposition {
    name: String,
    classification: String,
    owner: String,
    review_by: String,
    reason: String,
}

fn closure<'a>(
    skill: &'a EmbeddedSkill,
    skills: &'a [EmbeddedSkill],
    seen: &mut BTreeSet<&'a str>,
) -> BTreeSet<&'a str> {
    assert!(seen.insert(skill.name), "dependency cycle: {}", skill.name);
    let mut tools: BTreeSet<_> = skill.manifest.tools.iter().copied().collect();
    for dependency in skill.manifest.requires_skills {
        let parent = skills
            .iter()
            .find(|s| s.name == *dependency)
            .expect("known dependency");
        tools.extend(closure(parent, skills, seen));
    }
    seen.remove(skill.name);
    tools
}

fn check_unit(
    markdown: &str,
    declared: &BTreeSet<&str>,
    registry: &BTreeSet<&str>,
    operators: &BTreeSet<&str>,
    available: &BTreeSet<&str>,
) -> Result<(), String> {
    for name in declared {
        if !registry.contains(name) {
            return Err(format!("unknown tool: {name}"));
        }
        if operators.contains(name) {
            return Err(format!("operator dependency: {name}"));
        }
        if !available.contains(name) {
            return Err(format!("unavailable tool: {name}"));
        }
    }
    let markers = regex::Regex::new(r"<!--\s*tool:\s*([^>]*?)\s*-->").unwrap();
    let marked: BTreeSet<_> = markers
        .captures_iter(markdown)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();
    for name in &marked {
        if !registry.contains(name) {
            return Err(format!("unknown marker: {name}"));
        }
        if !declared.contains(name) {
            return Err(format!("undeclared marker: {name}"));
        }
    }
    let words = regex::Regex::new(r"\b[a-z][a-z0-9_]+\b").unwrap();
    for word in words
        .find_iter(markdown)
        .map(|m| m.as_str())
        .filter(|name| registry.contains(name))
    {
        if !declared.contains(word) {
            return Err(format!("undeclared mention: {word}"));
        }
        if !marked.contains(word) {
            return Err(format!("missing tool marker: {word}"));
        }
    }
    Ok(())
}

fn check_owned(
    registry: &BTreeSet<&str>,
    owned: &BTreeSet<&str>,
    classified: &BTreeSet<&str>,
) -> Result<(), String> {
    for tool in registry {
        if !owned.contains(tool) && !classified.contains(tool) {
            return Err(format!("unowned tool: {tool}"));
        }
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn skill_profile_alignment() {
    let skills = list_skills();
    let ledger: Ledger = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/tool-dispositions.toml"
    )))
    .unwrap();
    assert_eq!(ledger.version, 1);
    let registry: BTreeSet<_> = static_tool_names().iter().copied().collect();
    let mut classified = BTreeSet::new();
    for entry in &ledger.tools {
        assert!(
            registry.contains(entry.name.as_str()),
            "stale disposition: {}",
            entry.name
        );
        assert!(
            classified.insert(entry.name.as_str()),
            "duplicate disposition"
        );
        assert!(["retained", "operator", "compatibility"].contains(&entry.classification.as_str()));
        assert!(!entry.owner.is_empty() && !entry.review_by.is_empty() && !entry.reason.is_empty());
    }
    let operators: BTreeSet<_> = ledger
        .tools
        .iter()
        .filter(|e| e.classification == "operator")
        .map(|e| e.name.as_str())
        .collect();
    for reference in spindle_adapters::list_references()
        .iter()
        .filter(|r| !r.name.contains('/'))
    {
        check_unit(
            reference.markdown,
            &BTreeSet::new(),
            &registry,
            &operators,
            &registry,
        )
        .unwrap_or_else(|e| {
            panic!(
                "global craft reference {} must remain tool-free: {e}",
                reference.name
            )
        });
    }
    let mut owned = BTreeSet::new();
    let mut router = router().await;
    for skill in skills {
        let tools = closure(skill, skills, &mut BTreeSet::new());
        for (name, markdown, required, profiles) in std::iter::once((
            skill.name,
            skill.markdown,
            tools.clone(),
            skill.manifest.profiles,
        ))
        .chain(skill.manifest.references.iter().map(|r| {
            let mut required = tools.clone();
            required.extend(r.tools.iter().copied());
            (r.name, r.markdown, required, r.profiles)
        })) {
            owned.extend(required.iter().copied());
            for profile in profiles {
                assert!(
                    *profile == "default"
                        || crate::tools::validate_tool_profile(Some(profile)).is_ok(),
                    "unknown declared profile {profile}"
                );
                router.tool_profile = (*profile != "default").then(|| (*profile).to_owned());
                let listed = router.list_tools();
                let available: BTreeSet<_> = listed.iter().map(|t| t.name.as_ref()).collect();
                check_unit(markdown, &required, &registry, &operators, &available)
                    .unwrap_or_else(|e| panic!("{name} in {profile}: {e}"));
                for tool in &required {
                    assert!(
                        router.has_tool(tool),
                        "{name}/{profile}: dispatch denied {tool}"
                    );
                }
            }
        }
    }
    assert_eq!(
        classified, registry,
        "every registry tool needs one disposition"
    );
    for entry in &ledger.tools {
        if entry.classification == "retained" {
            assert!(
                owned.contains(entry.name.as_str()),
                "retained tool has no skill owner: {}",
                entry.name
            );
        }
    }
    let exemptions = ledger
        .tools
        .iter()
        .filter(|e| e.classification != "retained")
        .map(|e| e.name.as_str())
        .collect();
    check_owned(&registry, &owned, &exemptions).unwrap();
    assert!(operators.is_disjoint(&owned));
}

#[test]
fn alignment_negative_fixtures() {
    let registry = BTreeSet::from([
        "set_active_project",
        "continue_generation",
        "configure_agents",
    ]);
    let operators = BTreeSet::from(["configure_agents"]);
    let all = registry.clone();
    for (text, declared, available, expected) in [
        (
            "",
            BTreeSet::from(["set_active_project"]),
            BTreeSet::new(),
            "unavailable tool: set_active_project",
        ),
        (
            "`continue_generation`",
            BTreeSet::new(),
            all.clone(),
            "undeclared mention: continue_generation",
        ),
        (
            "<!-- tool: continnu_generation -->",
            BTreeSet::new(),
            all.clone(),
            "unknown marker: continnu_generation",
        ),
        (
            "",
            BTreeSet::from(["configure_agents"]),
            all.clone(),
            "operator dependency: configure_agents",
        ),
        (
            "`set_active_project`",
            BTreeSet::from(["set_active_project"]),
            all.clone(),
            "missing tool marker: set_active_project",
        ),
    ] {
        assert_eq!(
            check_unit(text, &declared, &registry, &operators, &available),
            Err(expected.to_owned())
        );
    }
    assert_eq!(
        check_owned(
            &BTreeSet::from(["new_tool"]),
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Err("unowned tool: new_tool".to_owned())
    );
}

#[test]
fn skill_manifest_metadata_matches_snapshot() {
    let metadata: BTreeMap<_, _> = list_skills().iter().map(|s| (s.name, serde_json::json!({
        "version": s.manifest.version, "tools": s.manifest.tools, "profiles": s.manifest.profiles,
        "requires_skills": s.manifest.requires_skills,
        "references": s.manifest.references.iter().map(|r| serde_json::json!({"name": r.name, "tools": r.tools, "profiles": r.profiles, "trigger": r.trigger})).collect::<Vec<_>>()
    }))).collect();
    let actual = serde_json::to_string_pretty(&metadata).unwrap() + "\n";
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools/alignment/metadata.json");
    if std::env::var("SPINDLE_UPDATE_SKILL_SNAPSHOT").as_deref() == Ok("1") {
        std::fs::write(&path, &actual).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        actual,
        "regenerate metadata snapshot intentionally; never hand edit"
    );
}

#[test]
fn generated_profile_metadata_matches_snapshot() {
    let metadata: Vec<_> = spindle_adapters::guidance::list_profiles().iter().map(|p| serde_json::json!({"name": p.name, "description": p.description, "units": p.units, "tools": p.tools})).collect();
    let actual = serde_json::to_string_pretty(&metadata).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/generated-profiles.json");
    if std::env::var("SPINDLE_UPDATE_SKILL_SNAPSHOT").as_deref() == Ok("1") {
        std::fs::write(&path, &actual).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        actual,
        "regenerate from composition and manifests; never hand edit"
    );
}
