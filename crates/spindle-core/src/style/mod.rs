//! Project-level **style contract**: the single source of truth for genre-voice
//! enforcement.
//!
//! Spindle previously presented the reader contract as context but never
//! enforced it. The writing pipeline was built around literary-fiction craft
//! techniques, so a comedy webnovel could silently come out as a contemplative
//! grief memoir. This module centralizes "what should the prose FEEL like" so
//! that scene-context assembly, the `save_scene_draft` gate, the
//! `style_compliance` validator, and the dual-persona review's Target Reader
//! persona all read the *same* contract instead of re-deriving it (or ignoring
//! it) independently.
//!
//! A [`StyleDirective`] is assembled from three sources:
//! 1. the project [`reader_contract`](crate::models::ReaderContract)
//!    (`promise`, `style_notes`, `boundaries`),
//! 2. world rules whose `rule_type` is `"style"`, and
//! 3. the project [`NarratorVoice`] (prose-level narration directive).
//!
//! The deterministic [`StyleDirective::scan`] only catches *coarse* signals
//! (a grief-beat tone string on a comedy project, a contemplative no-hook
//! chapter ending). Reliable genre judgement ("is this actually funny?") is
//! semantic and belongs to the LLM-backed review persona — this scanner is the
//! cheap, always-on first line, not the whole defense.

pub mod scanner;

pub use scanner::{StyleDriftHit, StyleDriftSeverity, StyleScanInput};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Prose-level narration directive. For first-person or close-third narration
/// the narrator's voice *is* the prose style of the whole book, which a
/// per-character dialogue voice profile cannot capture. All fields are optional
/// so a project can specify only what it cares about.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct NarratorVoice {
    /// How often the reader should laugh / how dense the comedy is
    /// (e.g. "high — a laugh a page", "light wry undertone", "none").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comedy_density: Option<String>,
    /// Punchy/snappy vs contemplative/flowing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pacing_feel: Option<String>,
    /// Balance of interior monologue vs dialogue/action
    /// (e.g. "dialogue-forward", "heavy interiority").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interiority_ratio: Option<String>,
    /// Default emotional register
    /// (e.g. "funny-and-sarcastic", "brooding-and-reflective").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotional_register: Option<String>,
    /// Preferred chapter-ending beat
    /// (e.g. "hook", "cliffhanger", "laugh", "grief beat", "resolution").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_ending_style: Option<String>,
    /// Freeform extra narration directives.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl NarratorVoice {
    /// True when no narration directive has been set.
    pub fn is_empty(&self) -> bool {
        self.comedy_density.is_none()
            && self.pacing_feel.is_none()
            && self.interiority_ratio.is_none()
            && self.emotional_register.is_none()
            && self.chapter_ending_style.is_none()
            && self.notes.is_empty()
    }

    fn render_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(value) = field_value(&self.emotional_register) {
            lines.push(format!("  - Emotional register: {value}"));
        }
        if let Some(value) = field_value(&self.comedy_density) {
            lines.push(format!("  - Comedy density: {value}"));
        }
        if let Some(value) = field_value(&self.pacing_feel) {
            lines.push(format!("  - Pacing feel: {value}"));
        }
        if let Some(value) = field_value(&self.interiority_ratio) {
            lines.push(format!("  - Interiority vs dialogue: {value}"));
        }
        if let Some(value) = field_value(&self.chapter_ending_style) {
            lines.push(format!("  - Chapter-ending style: {value}"));
        }
        for note in &self.notes {
            let note = note.trim();
            if !note.is_empty() {
                lines.push(format!("  - {note}"));
            }
        }
        lines
    }
}

/// A world rule of type `style`, surfaced as a mandatory prose-level directive
/// rather than background lore.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StyleRule {
    pub rule_name: String,
    pub description: String,
}

/// The consolidated style contract for a project. This is what the scene
/// pipeline reads to enforce genre voice. Built via
/// [`StyleDirective::assemble`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct StyleDirective {
    pub genre: String,
    pub project_type: String,
    pub promise: String,
    #[serde(default)]
    pub style_notes: Vec<String>,
    #[serde(default)]
    pub boundaries: Vec<String>,
    #[serde(default)]
    pub style_rules: Vec<StyleRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrator_voice: Option<NarratorVoice>,
}

/// Genre intent flags derived from the directive's free-text fields. Heuristic,
/// case-insensitive keyword matching — deliberately conservative so that the
/// deterministic scanner does not cry wolf.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StyleIntent {
    /// The genre demands humor (comedy, rom-com, satire, "raunchy", ...).
    pub wants_comedy: bool,
    /// The genre demands fast, serialized pacing (webnovel, litrpg,
    /// progression, "punchy", ...).
    pub wants_fast_pacing: bool,
    /// Chapters should end on hooks/cliffhangers rather than resolution beats.
    pub wants_hook_endings: bool,
    /// The project explicitly embraces literary/contemplative prose — when set,
    /// literary-marker heuristics are suppressed (they would be false positives).
    pub is_literary: bool,
}

impl StyleDirective {
    /// Assemble a directive from its three sources. `narrator_voice` is dropped
    /// if empty so callers can treat `None`/empty uniformly.
    pub fn assemble(
        genre: impl Into<String>,
        project_type: impl Into<String>,
        promise: impl Into<String>,
        style_notes: Vec<String>,
        boundaries: Vec<String>,
        style_rules: Vec<StyleRule>,
        narrator_voice: Option<NarratorVoice>,
    ) -> Self {
        let narrator_voice = narrator_voice.filter(|voice| !voice.is_empty());
        Self {
            genre: genre.into(),
            project_type: project_type.into(),
            promise: promise.into(),
            style_notes,
            boundaries,
            style_rules,
            narrator_voice,
        }
    }

    /// True when there is no style signal at all worth enforcing or rendering.
    pub fn is_empty(&self) -> bool {
        self.genre.trim().is_empty()
            && self.project_type.trim().is_empty()
            && self.promise.trim().is_empty()
            && self.style_notes.iter().all(|note| note.trim().is_empty())
            && self.boundaries.iter().all(|note| note.trim().is_empty())
            && self.style_rules.is_empty()
            && self.narrator_voice.is_none()
    }

    /// Lowercased haystack of every free-text field, used for intent detection.
    fn intent_haystack(&self) -> String {
        let mut parts: Vec<String> = vec![
            self.genre.clone(),
            self.project_type.clone(),
            self.promise.clone(),
        ];
        parts.extend(self.style_notes.iter().cloned());
        parts.extend(self.boundaries.iter().cloned());
        for rule in &self.style_rules {
            parts.push(rule.rule_name.clone());
            parts.push(rule.description.clone());
        }
        if let Some(voice) = &self.narrator_voice {
            parts.extend(
                [
                    &voice.comedy_density,
                    &voice.pacing_feel,
                    &voice.interiority_ratio,
                    &voice.emotional_register,
                    &voice.chapter_ending_style,
                ]
                .into_iter()
                .flatten()
                .cloned(),
            );
            parts.extend(voice.notes.iter().cloned());
        }
        parts.join("\n").to_lowercase()
    }

    /// Derive the [`StyleIntent`] flags from the directive's text.
    pub fn intent(&self) -> StyleIntent {
        let haystack = self.intent_haystack();
        let has_any = |needles: &[&str]| needles.iter().any(|needle| haystack.contains(needle));

        let wants_comedy = has_any(&[
            "comedy",
            "comedic",
            "funny",
            "humor",
            "humour",
            "raunchy",
            "laugh",
            "satire",
            "farce",
            "rom-com",
            "romcom",
            "slapstick",
            "hilar",
            "absurd",
        ]);
        let wants_fast_pacing = has_any(&[
            "webnovel",
            "web novel",
            "litrpg",
            "gamelit",
            "progression",
            "serial",
            "punchy",
            "snappy",
            "fast pac",
            "page-turner",
            "pageturner",
            "fast-pac",
            "gacha",
        ]);
        let ending = self
            .narrator_voice
            .as_ref()
            .and_then(|voice| voice.chapter_ending_style.as_deref())
            .unwrap_or("")
            .to_lowercase();
        let wants_hook_endings = wants_fast_pacing
            || ending.contains("hook")
            || ending.contains("cliffhang")
            || has_any(&[
                "hook ending",
                "cliffhanger",
                "chapter hook",
                "end on a hook",
            ]);

        // Only treat the project as deliberately literary when it says so AND
        // it is not also asking for comedy/fast pacing (a literary comedy still
        // wants the comedy heuristics to run).
        let is_literary = !wants_comedy
            && !wants_fast_pacing
            && has_any(&[
                "literary fiction",
                "literary",
                "prestige",
                "prose poem",
                "contemplative",
                "meditative",
                "elegiac",
                "lyrical",
            ]);

        StyleIntent {
            wants_comedy,
            wants_fast_pacing,
            wants_hook_endings,
            is_literary,
        }
    }

    /// Render the forceful "Project Style Requirements" block injected at the
    /// top of scene context and into the standards section. Returns `None` when
    /// the directive is empty.
    pub fn render_markdown(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let intent = self.intent();
        let mut lines = vec!["\n## Project Style Requirements".to_string()];

        let descriptor = self.genre_descriptor();
        lines.push(format!(
            "These requirements are MANDATORY and override general craft guidance wherever they \
             conflict. This project is {descriptor}."
        ));

        if !self.promise.trim().is_empty() {
            lines.push(format!(
                "\n- **Promise to the reader**: {}",
                self.promise.trim()
            ));
        }

        let style_notes: Vec<&str> = self
            .style_notes
            .iter()
            .map(|note| note.trim())
            .filter(|note| !note.is_empty())
            .collect();
        if !style_notes.is_empty() {
            lines.push("\n**Style notes (the prose MUST deliver these):**".to_string());
            for note in style_notes {
                lines.push(format!("- {note}"));
            }
        }

        let boundaries: Vec<&str> = self
            .boundaries
            .iter()
            .map(|note| note.trim())
            .filter(|note| !note.is_empty())
            .collect();
        if !boundaries.is_empty() {
            lines.push("\n**Boundaries (keep these OUT):**".to_string());
            for note in boundaries {
                lines.push(format!("- {note}"));
            }
        }

        if !self.style_rules.is_empty() {
            lines.push("\n**Style directives (mandatory prose-level world rules):**".to_string());
            for rule in &self.style_rules {
                lines.push(format!(
                    "- [STYLE DIRECTIVE] **{}**: {}",
                    rule.rule_name.trim(),
                    rule.description.trim()
                ));
            }
        }

        if let Some(voice) = &self.narrator_voice {
            let voice_lines = voice.render_lines();
            if !voice_lines.is_empty() {
                lines.push(
                    "\n**Narrator voice (this IS the prose style — write the narration to it):**"
                        .to_string(),
                );
                lines.extend(voice_lines);
            }
        }

        // The closing imperative — phrased so a craft-optimizing model cannot
        // rationalize a beautiful but off-genre scene.
        lines.push("\n**Enforcement:** Your prose MUST deliver on the above.".to_string());
        if intent.wants_comedy {
            lines.push(
                "If the scene is not funny, it has failed regardless of how well-crafted it is — \
                 a wry aside is not comedy."
                    .to_string(),
            );
        }
        if intent.wants_hook_endings {
            lines.push(
                "If a chapter-ending scene closes on a quiet/reflective/grief beat instead of a \
                 hook, it has failed regardless of how \"emotionally resonant\" the ending is."
                    .to_string(),
            );
        }
        if intent.wants_fast_pacing {
            lines.push(
                "Keep the pacing punchy and serialized; do not drift into slow literary rhythm."
                    .to_string(),
            );
        }
        lines.push(
            "\"Beautifully written\" is never a defense against \"wrong for this book.\""
                .to_string(),
        );

        Some(lines.join("\n"))
    }

    /// A short human descriptor like `an NSFW Comedy Webnovel` for the directive
    /// header, derived from project_type + genre.
    fn genre_descriptor(&self) -> String {
        let project_type = self.project_type.trim();
        let genre = self.genre.trim();
        match (project_type.is_empty(), genre.is_empty()) {
            (false, false) => format!("a {project_type} in the {genre} genre"),
            (false, true) => format!("a {project_type} project"),
            (true, false) => format!("a {genre} project"),
            (true, true) => "a project with a declared style contract".to_string(),
        }
    }

    /// Run the deterministic style-drift heuristics against a draft.
    pub fn scan(&self, input: &StyleScanInput) -> Vec<StyleDriftHit> {
        scanner::scan(self, input)
    }
}

fn field_value(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comedy_directive() -> StyleDirective {
        StyleDirective::assemble(
            "Comedy",
            "NSFW Comedy Webnovel",
            "A raunchy, funny gacha power-fantasy romp",
            vec![
                "Raunchy modern comedy tone".to_string(),
                "Webnovel pacing with clear progression".to_string(),
            ],
            vec!["Focus on raunchy comedy and fun over dark themes".to_string()],
            vec![StyleRule {
                rule_name: "Prose Style Bible — Webnovel-First, Comedy-First".to_string(),
                description: "No grief-beat endings; no contemplative literary pacing.".to_string(),
            }],
            Some(NarratorVoice {
                emotional_register: Some("funny-and-sarcastic".to_string()),
                chapter_ending_style: Some("hook".to_string()),
                ..Default::default()
            }),
        )
    }

    #[test]
    fn empty_directive_renders_nothing() {
        let directive = StyleDirective::default();
        assert!(directive.is_empty());
        assert!(directive.render_markdown().is_none());
        assert!(directive.scan(&StyleScanInput::default()).is_empty());
    }

    #[test]
    fn comedy_intent_is_detected() {
        let intent = comedy_directive().intent();
        assert!(intent.wants_comedy);
        assert!(intent.wants_fast_pacing);
        assert!(intent.wants_hook_endings);
        assert!(!intent.is_literary);
    }

    #[test]
    fn literary_project_is_not_flagged_as_comedy() {
        let directive = StyleDirective::assemble(
            "Literary Fiction",
            "Novel",
            "A contemplative meditation on grief and memory",
            vec!["Lyrical, flowing, contemplative prose".to_string()],
            Vec::new(),
            Vec::new(),
            None,
        );
        let intent = directive.intent();
        assert!(!intent.wants_comedy);
        assert!(intent.is_literary);
    }

    #[test]
    fn render_includes_forceful_enforcement_for_comedy() {
        let rendered = comedy_directive().render_markdown().expect("non-empty");
        assert!(rendered.contains("Project Style Requirements"));
        assert!(rendered.contains("MANDATORY"));
        assert!(rendered.contains("[STYLE DIRECTIVE]"));
        assert!(rendered.contains("not funny, it has failed"));
        assert!(rendered.contains("Narrator voice"));
    }
}
