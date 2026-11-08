#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedSkill {
    pub name: &'static str,
    pub markdown: &'static str,
    pub manifest: SkillManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillManifest {
    pub version: u32,
    pub name: &'static str,
    pub tools: &'static [&'static str],
    pub profiles: &'static [&'static str],
    pub requires_skills: &'static [&'static str],
    pub references: &'static [EmbeddedReference],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedReference {
    pub name: &'static str,
    pub markdown: &'static str,
    pub tools: &'static [&'static str],
    pub profiles: &'static [&'static str],
    pub trigger: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratedProfile {
    pub name: &'static str,
    pub description: &'static str,
    pub units: &'static [&'static str],
    pub tools: &'static [&'static str],
}

pub fn list_profiles() -> &'static [GeneratedProfile] {
    GENERATED_PROFILES
}

pub fn get_reference(name: &str) -> Option<EmbeddedReference> {
    EMBEDDED_SKILLS
        .iter()
        .flat_map(|skill| skill.manifest.references)
        .copied()
        .find(|reference| reference.name == name)
}

include!(concat!(env!("OUT_DIR"), "/embedded_skills.rs"));

pub fn list_skills() -> &'static [EmbeddedSkill] {
    EMBEDDED_SKILLS
}

pub fn get_skill(name: &str) -> Option<EmbeddedSkill> {
    EMBEDDED_SKILLS
        .iter()
        .copied()
        .find(|skill| skill.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_all_repo_skill_files() {
        let skills = list_skills();
        let names: Vec<_> = skills.iter().map(|skill| skill.name).collect();

        assert!(names.contains(&"bible-librarian"));
        assert!(names.contains(&"canon-steward"));
        assert!(names.contains(&"character-creator"));
        assert!(names.contains(&"continuity-editor"));
        assert!(names.contains(&"manuscript-importer"));
        assert!(names.contains(&"plot-architect"));
        assert!(names.contains(&"revision-manager"));
        assert!(names.contains(&"scene-writer"));
        assert!(names.contains(&"worldbuilder"));
        assert!(names.contains(&"editor"));
        assert!(names.contains(&"authoring-supervisor"));
        assert!(names.contains(&"researcher"));
        assert!(names.contains(&"spindle"));
        assert_eq!(skills.len(), 13);
    }
}

#[cfg(test)]
mod manifest_contract_tests {
    use super::*;

    #[test]
    fn every_skill_has_a_versioned_manifest() {
        for skill in list_skills() {
            assert_eq!(skill.manifest.version, 1, "{}", skill.name);
            assert_eq!(skill.manifest.name, skill.name);
            assert!(!skill.manifest.profiles.is_empty());
        }
    }

    #[test]
    fn nested_units_are_embedded_and_exactly_addressed() {
        let reference = get_reference("spindle/setup").expect("setup reference");
        assert!(reference.markdown.contains("create_project"));
        assert!(get_reference("../spindle/setup").is_none());
        assert!(get_reference("spindle/../setup").is_none());
        assert!(get_reference("setup").is_none());
    }
}

#[cfg(test)]
#[allow(dead_code)]
#[path = "../build.rs"]
mod build_contract;

#[cfg(test)]
mod profile_contract_tests {
    use super::*;

    #[test]
    fn generated_profiles_are_sorted_and_resolve_dependency_closure() {
        let profiles = list_profiles();
        assert_eq!(profiles.len(), 10);
        assert!(profiles.windows(2).all(|p| p[0].name < p[1].name));
        for profile in profiles {
            assert!(profile.tools.windows(2).all(|p| p[0] < p[1]));
            assert!(profile.units.contains(&"spindle"));
            for name in [
                "list_projects",
                "set_active_project",
                "get_writer_state",
                "get_skill",
                "get_reference",
                "list_skills",
            ] {
                assert!(profile.tools.contains(&name), "{}: {name}", profile.name);
            }
        }
        let import = profiles
            .iter()
            .find(|p| p.name == "workflow-import")
            .unwrap();
        assert!(!import.tools.contains(&"apply_style_profile"));
        let revision = profiles.iter().find(|p| p.name == "revision").unwrap();
        assert!(revision.tools.contains(&"create_branch"));
        assert!(revision.tools.contains(&"revise_scene"));
    }
}
