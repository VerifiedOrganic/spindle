use spindle_skills::{
    EmbeddedSkill, get_skill as embedded_get_skill, list_skills as embedded_list_skills,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedReference {
    pub name: &'static str,
    pub markdown: &'static str,
}

const EMBEDDED_REFERENCES: &[EmbeddedReference] = &[
    EmbeddedReference {
        name: "anti-slop",
        markdown: include_str!("../../../references/anti-slop.md"),
    },
    EmbeddedReference {
        name: "mru-guide",
        markdown: include_str!("../../../references/mru-guide.md"),
    },
    EmbeddedReference {
        name: "swain-scene-sequel",
        markdown: include_str!("../../../references/swain-scene-sequel.md"),
    },
    EmbeddedReference {
        name: "voice-differentiation",
        markdown: include_str!("../../../references/voice-differentiation.md"),
    },
];

pub fn list_skills() -> &'static [EmbeddedSkill] {
    embedded_list_skills()
}

pub fn get_skill(name: &str) -> Option<EmbeddedSkill> {
    embedded_get_skill(name)
}

pub fn standards_text() -> &'static str {
    get_skill("scene-writer")
        .map(|skill| skill.markdown)
        .unwrap_or_default()
}

pub fn list_references() -> &'static [EmbeddedReference] {
    EMBEDDED_REFERENCES
}

pub fn get_reference(name: &str) -> Option<EmbeddedReference> {
    EMBEDDED_REFERENCES
        .iter()
        .copied()
        .find(|reference| reference.name == name)
}
