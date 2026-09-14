use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::pack::{GenreOverride, Severity, ShelfLimit, ShelfPack};
use crate::style::StyleProfileGuidance;

/// How an applied style profile changed a shelf relative to pack defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileOverlay {
    Disabled,
    Softened,
    PromotedHard,
}

/// One shelf line in the writing-packet digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompactShelfEntry {
    pub id: String,
    pub severity: Severity,
    pub default_limit: ShelfLimit,
    pub limit_scope: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_overlay: Option<ProfileOverlay>,
}

/// Budget-capped digest of the twelve fiction shelves for the active profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompactShelfDigest {
    pub pack_id: String,
    pub pack_version: String,
    pub catalog_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub rewrite_max_passes: u8,
    pub shelves: Vec<CompactShelfEntry>,
}

/// Short on-voice excerpt used as a rewrite-from-beats target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceSample {
    pub speaker: String,
    pub excerpt: String,
    pub source: String,
}

/// Compact do-not-repeat note, optionally tied to a shelf id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SceneNegative {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shelf_id: Option<String>,
    pub note: String,
    pub source: String,
}

/// Style-profile hooks that ride the writing packet beside the digest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WritingPacketHooks {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voice_samples: Vec<VoiceSample>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_negatives: Vec<SceneNegative>,
}

const MAX_VOICE_SAMPLES: usize = 4;
const MAX_SCENE_NEGATIVES: usize = 6;
const MAX_VOICE_WORDS: usize = 40;
const MAX_NEGATIVE_WORDS: usize = 24;

/// Build the packet digest. Includes disabled shelves so a profile overlay is visible.
pub fn compact_shelf_digest(
    pack: &ShelfPack,
    profile: Option<&GenreOverride>,
) -> CompactShelfDigest {
    let catalog_uri = pack
        .catalog_uri
        .clone()
        .unwrap_or_else(|| "bible://references/anti-slop".to_string());
    let shelves = pack
        .shelves
        .iter()
        .map(|shelf| {
            let enabled = pack.is_enabled(shelf, profile);
            let severity = pack.effective_severity(shelf, profile);
            CompactShelfEntry {
                id: shelf.id.clone(),
                severity,
                default_limit: shelf.default_limit.clone(),
                limit_scope: shelf.limit_scope.clone(),
                enabled,
                profile_overlay: overlay_for(shelf.id.as_str(), profile),
            }
        })
        .collect();
    CompactShelfDigest {
        pack_id: pack.pack_id.clone(),
        pack_version: pack.pack_version.clone(),
        catalog_uri,
        profile: profile.map(|overlay| overlay.profile.clone()),
        rewrite_max_passes: pack.rewrite_budget(),
        shelves,
    }
}

fn overlay_for(shelf_id: &str, profile: Option<&GenreOverride>) -> Option<ProfileOverlay> {
    let profile = profile?;
    if profile.disable.iter().any(|id| id == shelf_id) {
        return Some(ProfileOverlay::Disabled);
    }
    if profile.promote_to_hard.iter().any(|id| id == shelf_id) {
        return Some(ProfileOverlay::PromotedHard);
    }
    if profile.soften.iter().any(|id| id == shelf_id) {
        return Some(ProfileOverlay::Softened);
    }
    None
}

/// Parse disable / soften / promote directives from style notes or guidance lines.
pub fn genre_override_from_style_texts(
    profile: impl Into<String>,
    texts: &[String],
) -> GenreOverride {
    let mut disable = Vec::new();
    let mut soften = Vec::new();
    let mut promote_to_hard = Vec::new();
    for text in texts {
        for token in directive_tokens(text) {
            match token.verb.as_str() {
                "disable" | "disabled" => push_unique(&mut disable, token.shelf_id),
                "soften" | "softened" => push_unique(&mut soften, token.shelf_id),
                "promote" | "promote_to_hard" | "promoted" => {
                    push_unique(&mut promote_to_hard, token.shelf_id)
                }
                _ => {}
            }
        }
    }
    GenreOverride {
        profile: profile.into(),
        disable,
        soften,
        promote_to_hard,
    }
}

struct OverlayToken {
    verb: String,
    shelf_id: String,
}

fn directive_tokens(text: &str) -> Vec<OverlayToken> {
    let mut tokens = Vec::new();
    let lower = text.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
        .filter(|word| !word.is_empty())
        .collect();
    let mut i = 0;
    while i + 1 < words.len() {
        let verb = words[i].trim_end_matches(':');
        let shelf = words[i + 1].trim_start_matches(':');
        if is_overlay_verb(verb) && looks_like_shelf_id(shelf) {
            tokens.push(OverlayToken {
                verb: verb.to_string(),
                shelf_id: shelf.to_string(),
            });
            i += 2;
            continue;
        }
        i += 1;
    }
    tokens
}

fn is_overlay_verb(word: &str) -> bool {
    matches!(
        word,
        "disable" | "disabled" | "soften" | "softened" | "promote" | "promote_to_hard" | "promoted"
    )
}

fn looks_like_shelf_id(word: &str) -> bool {
    word.contains('_') && word.chars().all(|c| c.is_ascii_lowercase() || c == '_')
}

fn push_unique(items: &mut Vec<String>, value: String) {
    if !items.iter().any(|existing| existing == &value) {
        items.push(value);
    }
}

/// Assemble the digest + style-profile hooks used on the writing packet.
pub fn assemble_writing_packet(
    pack: &ShelfPack,
    profile_name: Option<&str>,
    style_texts: &[String],
    guidance: Option<&StyleProfileGuidance>,
) -> (CompactShelfDigest, WritingPacketHooks) {
    let mut texts = style_texts.to_vec();
    if let Some(guidance) = guidance {
        texts.extend(guidance.avoid_rules.iter().cloned());
        texts.extend(guidance.do_rules.iter().cloned());
        if !guidance.prompt_snippet.trim().is_empty() {
            texts.push(guidance.prompt_snippet.clone());
        }
    }
    let overlay =
        genre_override_from_style_texts(profile_name.unwrap_or("reader_contract"), &texts);
    let digest = compact_shelf_digest(
        pack,
        if overlay.is_empty() {
            None
        } else {
            Some(&overlay)
        },
    );
    let hooks = guidance
        .map(|guidance| writing_packet_hooks_from_guidance(guidance, pack))
        .unwrap_or_default();
    (digest, hooks)
}

/// Pull budget-capped voice-sample and scene-negative hooks from style guidance.
pub fn writing_packet_hooks_from_guidance(
    guidance: &StyleProfileGuidance,
    pack: &ShelfPack,
) -> WritingPacketHooks {
    let mut voice_samples = Vec::new();
    push_voice(
        &mut voice_samples,
        "narrator",
        "style_profile.do_rules",
        &guidance.do_rules,
    );
    if !guidance.prompt_snippet.trim().is_empty() {
        push_voice(
            &mut voice_samples,
            "narrator",
            "style_profile.prompt_snippet",
            std::slice::from_ref(&guidance.prompt_snippet),
        );
    }
    push_voice(
        &mut voice_samples,
        "narrator",
        "style_profile.narrator_voice",
        &guidance.narrator_voice.notes,
    );

    let mut scene_negatives = Vec::new();
    for rule in &guidance.avoid_rules {
        let note = cap_words(rule, MAX_NEGATIVE_WORDS);
        if note.is_empty() {
            continue;
        }
        if scene_negatives.len() >= MAX_SCENE_NEGATIVES {
            break;
        }
        scene_negatives.push(SceneNegative {
            shelf_id: mentioned_shelf_id(rule, pack),
            note,
            source: "style_profile.avoid_rules".to_string(),
        });
    }

    WritingPacketHooks {
        voice_samples,
        scene_negatives,
    }
}

fn push_voice(out: &mut Vec<VoiceSample>, speaker: &str, source: &str, lines: &[String]) {
    for line in lines {
        if out.len() >= MAX_VOICE_SAMPLES {
            return;
        }
        let excerpt = cap_words(line, MAX_VOICE_WORDS);
        if excerpt.is_empty() {
            continue;
        }
        out.push(VoiceSample {
            speaker: speaker.to_string(),
            excerpt,
            source: source.to_string(),
        });
    }
}

fn mentioned_shelf_id(text: &str, pack: &ShelfPack) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for shelf in &pack.shelves {
        if lower.contains(&shelf.id) {
            return Some(shelf.id.clone());
        }
    }
    if lower.contains("mix of")
        || lower.contains("equal parts")
        || lower.contains("swirl of")
        || lower.contains("torn between")
    {
        return Some("emotion_cocktail".to_string());
    }
    None
}

fn cap_words(text: &str, max_words: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }
    if words.len() <= max_words {
        return words.join(" ");
    }
    words[..max_words].join(" ")
}

pub fn render_compact_shelf_digest_markdown(digest: &CompactShelfDigest) -> String {
    let mut lines = vec![
        "## compact_shelf_digest".to_string(),
        format!(
            "Fiction shelves for this draft (`{}` @ {}). Full catalog: {}. Rewrite-from-beats, at most {} pass(es) — not a paraphrase-humanizer.",
            digest.pack_id, digest.pack_version, digest.catalog_uri, digest.rewrite_max_passes
        ),
    ];
    if let Some(profile) = digest.profile.as_deref() {
        lines.push(format!("Active profile overlay: `{profile}`."));
    }
    lines.push(String::new());
    for shelf in &digest.shelves {
        let limit = match &shelf.default_limit {
            ShelfLimit::Count(count) => format!("≤{count}"),
            ShelfLimit::Label(label) => label.clone(),
        };
        let overlay = match shelf.profile_overlay {
            Some(ProfileOverlay::Disabled) => "; profile disabled",
            Some(ProfileOverlay::Softened) => "; profile softened",
            Some(ProfileOverlay::PromotedHard) => "; profile promoted to hard",
            None => "",
        };
        let enabled = if shelf.enabled { "on" } else { "off" };
        lines.push(format!(
            "- `{id}` — {severity:?} {limit}/{scope} ({enabled}{overlay})",
            id = shelf.id,
            severity = shelf.severity,
            scope = shelf.limit_scope,
        ));
    }
    lines.join("\n")
}

pub fn render_writing_packet_hooks_markdown(hooks: &WritingPacketHooks) -> String {
    if hooks.voice_samples.is_empty() && hooks.scene_negatives.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    if !hooks.voice_samples.is_empty() {
        lines.push("## voice_samples".to_string());
        lines.push(
            "Project-local on-voice targets for rewrite-from-beats. Do not imitate a copyrighted corpus."
                .to_string(),
        );
        for sample in &hooks.voice_samples {
            lines.push(format!(
                "- [{} / {}] {}",
                sample.speaker, sample.source, sample.excerpt
            ));
        }
    }
    if !hooks.scene_negatives.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("## scene_negatives".to_string());
        lines.push(
            "Do not repeat these moves. Go back to the beat; do not synonym-swap.".to_string(),
        );
        for negative in &hooks.scene_negatives {
            match &negative.shelf_id {
                Some(shelf_id) => lines.push(format!(
                    "- `{shelf_id}` ({src}): {note}",
                    src = negative.source,
                    note = negative.note
                )),
                None => lines.push(format!(
                    "- ({src}): {note}",
                    src = negative.source,
                    note = negative.note
                )),
            }
        }
    }
    lines.join("\n")
}

impl GenreOverride {
    pub fn is_empty(&self) -> bool {
        self.disable.is_empty() && self.soften.is_empty() && self.promote_to_hard.is_empty()
    }
}
