#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedSkill {
    pub name: &'static str,
    pub markdown: &'static str,
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
        assert!(names.contains(&"character-creator"));
        assert!(names.contains(&"continuity-editor"));
        assert!(names.contains(&"manuscript-importer"));
        assert!(names.contains(&"plot-architect"));
        assert!(names.contains(&"revision-manager"));
        assert!(names.contains(&"scene-writer"));
        assert!(names.contains(&"worldbuilder"));
        assert!(names.contains(&"editor"));
        assert_eq!(skills.len(), 9);
    }
}
