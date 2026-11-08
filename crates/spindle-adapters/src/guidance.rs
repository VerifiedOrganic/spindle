use spindle_skills::{
    EmbeddedSkill, get_skill as embedded_get_skill, list_skills as embedded_list_skills,
};
pub use spindle_skills::{GeneratedProfile, list_profiles};

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
    static REFERENCES: std::sync::OnceLock<Vec<EmbeddedReference>> = std::sync::OnceLock::new();
    REFERENCES.get_or_init(|| {
        let mut references = EMBEDDED_REFERENCES.to_vec();
        references.extend(
            embedded_list_skills()
                .iter()
                .flat_map(|s| s.manifest.references)
                .map(|r| EmbeddedReference {
                    name: r.name,
                    markdown: r.markdown,
                }),
        );
        references
    })
}

pub fn get_reference(name: &str) -> Option<EmbeddedReference> {
    list_references()
        .iter()
        .copied()
        .find(|reference| reference.name == name)
}
