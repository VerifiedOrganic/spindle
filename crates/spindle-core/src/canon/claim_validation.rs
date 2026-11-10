//! Shared claim-vs-canon validators (P2 PR F).
//!
//! The lexical prefilter stays as an inexpensive retrieval hint. Deterministic
//! reducers then compare normalized claims against ratified facts, knowledge
//! edges, world rules and secret circles. A semantic adjudication port exists
//! for prose rules that cannot be expressed deterministically; when no route is
//! configured the report says `Degraded` explicitly instead of treating the
//! missing lane as a clean scan.

use crate::models::TextByteRange;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CLAIM_VALIDATOR_VERSION: &str = "canon-claim-validators-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClaimValidationLane {
    LexicalPrefilter,
    Deterministic,
    SemanticAdjudication,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ClaimValidationLaneStatus {
    Complete,
    Degraded { reason: String },
    Incomplete { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClaimValidationSeverity {
    Error,
    Warning,
    Info,
}

impl ClaimValidationSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimValidationFindingV1 {
    pub check_type: String,
    pub severity: ClaimValidationSeverity,
    pub message: String,
    pub byte_range: Option<TextByteRange>,
    pub entity_ids: Vec<String>,
    pub lane: ClaimValidationLane,
    pub evidence_quote: Option<String>,
    pub dedup_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneValidationInputV1<'a> {
    pub scene_id: &'a str,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub text: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributeValidationFactV1 {
    pub fact_id: String,
    pub subject_id: Option<String>,
    pub subject_names: Vec<String>,
    /// True when the caller established that this fact's subject is the
    /// scene's single POV character and the scene is first-person narration.
    #[serde(default)]
    pub first_person_subject: bool,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeValidationFactV1 {
    pub fact_id: String,
    pub character_id: String,
    pub character_names: Vec<String>,
    /// Resolver-approved mention aliases (full name, forename/surname, cast
    /// nicknames). Empty means use `character_names` only.
    #[serde(default)]
    pub character_aliases: Vec<String>,
    /// True when the caller established first-person POV for this character.
    #[serde(default)]
    pub first_person_subject: bool,
    pub fact: String,
    pub learned_book_number: i32,
    pub learned_chapter_number: i32,
    pub learned_scene_order: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretValidationCandidateV1 {
    pub fact_id: String,
    pub predicate: String,
    pub value: String,
    pub character_id: String,
    pub character_name: String,
    /// Resolver-approved mention aliases (full name, forename/surname, cast
    /// nicknames). Empty means use `character_name` only.
    #[serde(default)]
    pub character_aliases: Vec<String>,
    /// Other present characters used to reject a sentence where the candidate
    /// is merely mentioned and someone else performs the leak action.
    pub other_present_character_names: Vec<String>,
    /// Resolver-approved aliases for those other characters.
    #[serde(default)]
    pub other_present_character_aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimValidationContextV1 {
    pub attributes: Vec<AttributeValidationFactV1>,
    pub knowledge: Vec<KnowledgeValidationFactV1>,
    pub secrets: Vec<SecretValidationCandidateV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimValidationReportV1 {
    pub findings: Vec<ClaimValidationFindingV1>,
    pub lexical_prefilter_hit_count: usize,
    pub lane_statuses: Vec<ClaimValidationLaneStatusEntryV1>,
    pub semantic_status: ClaimValidationLaneStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimValidationLaneStatusEntryV1 {
    pub lane: ClaimValidationLane,
    pub status: ClaimValidationLaneStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticAdjudicationRequestV1 {
    pub scene_id: String,
    pub text: String,
    pub claims: Vec<SemanticClaimV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticClaimV1 {
    pub check_type: String,
    pub description: String,
    pub canonical_value: String,
    pub evidence_quote: String,
}

/// Port for bounded model adjudication. Implementations must return an error
/// when the route is unavailable so callers can report a degraded lane rather
/// than a clean scan.
pub trait SemanticClaimAdjudicator: Send + Sync {
    fn adjudicate(
        &self,
        request: &SemanticAdjudicationRequestV1,
    ) -> Result<Vec<ClaimValidationFindingV1>, String>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParaphraseMatch {
    pub byte_range: TextByteRange,
    pub quote: String,
    pub matched_tokens: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttributeConflictMatch {
    pub byte_range: TextByteRange,
    pub quote: String,
    pub canonical_terms: Vec<String>,
    pub prose_terms: Vec<String>,
}

// ── lexical prefilter (retained, never contradiction proof) ───────────────

pub fn lexical_prefilter_hit(text: &str, key: &str) -> bool {
    !key.trim().is_empty() && contains_phrase(text, key)
}

// ── deterministic attribute contradiction lane ───────────────────────────

const PROSTHETIC_TERMS: &[&str] = &[
    "prosthetic",
    "prosthesis",
    "gauntlet",
    "artificial",
    "mechanical",
    "cybernetic",
    "augmented",
    "implant",
    "replacement",
];

const ORGANIC_TERMS: &[&str] = &[
    "flesh-and-blood",
    "flesh and blood",
    "bare",
    "unscarred",
    "natural",
    "biological",
    "organic",
    "unaugmented",
];

const COLOR_TERMS: &[&str] = &[
    "amber", "black", "blue", "brown", "cerulean", "crimson", "gold", "golden", "gray", "green",
    "grey", "hazel", "red", "scarlet", "silver", "violet", "white",
];

const IRREALIS_MARKERS: &[&str] = &[
    "if ",
    "would ",
    "might ",
    "could ",
    "dream",
    "dreamed",
    "remembered",
    "flashback",
    "recording",
    "legend",
    "rumor",
    "rumour",
    "story",
    "pretend",
    "imagined",
    "supposed",
    "in the mirror of memory",
];

const NEGATION_MARKERS: &[&str] = &[
    "not ",
    "never ",
    "no ",
    "without ",
    "unlike ",
    "instead of ",
    "rather than ",
    "wasn't",
    "was not",
    "isn't",
    "is not",
];

const STOPWORDS: &[&str] = &[
    "about", "after", "again", "against", "also", "and", "the", "that", "this", "their", "there",
    "these", "those", "with", "from", "into", "over", "under", "before", "beneath", "beside",
    "between", "through", "while", "when", "where", "which", "who", "whom", "whose", "had", "has",
    "have", "was", "were", "been", "being", "for", "but", "his", "her", "its", "our", "your",
    "she", "they", "them", "him", "one", "two", "three", "four", "five", "six", "seven",
];

const GENERIC_TITLES: &[&str] = &[
    "commander",
    "captain",
    "director",
    "doctor",
    "dr",
    "inspector",
    "engineer",
    "archivist",
    "quartermaster",
    "scout",
    "envoy",
    "pilot",
    "lieutenant",
    "sergeant",
    "lord",
    "lady",
    "sir",
    "mr",
    "mrs",
    "ms",
];

const LEAK_VERBS: &[&str] = &[
    "said",
    "says",
    "told",
    "tells",
    "tell",
    "repeat",
    "repeats",
    "repeated",
    "warned",
    "warns",
    "remarked",
    "remarks",
    "wrote",
    "writes",
    "copied",
    "copies",
    "logged",
    "logs",
    "recorded",
    "reported",
    "reports",
    "revealed",
    "reveals",
    "explained",
    "explains",
    "ordered",
    "orders",
    "admitted",
    "admits",
    "confirmed",
    "confirms",
    "mentioned",
    "mentions",
    "whispered",
    "whispers",
    "stated",
    "states",
    "claimed",
    "claims",
    "noted",
    "notes",
    "entered",
    "enters",
    "typed",
    "types",
    "inscribed",
    "inscribes",
];

const KNOWLEDGE_IRREALIS: &[&str] = &[
    "if ",
    "would ",
    "might ",
    "could ",
    "dream",
    "dreamed",
    "imagined",
    "supposed",
    "rumor",
    "rumour",
    "flashback",
    "years ago",
    "before learning",
    "had not learned",
    "did not know",
    "didn't know",
    "never learned",
    "unaware",
    "no knowledge",
    "not yet learned",
];

fn stem(word: &str) -> String {
    let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric());
    let trimmed = trimmed.trim_end_matches("'s");
    let lower = trimmed.to_ascii_lowercase();
    if lower.len() > 4
        && (lower.ends_with("ies") || lower.ends_with("ses") || lower.ends_with("xes"))
    {
        return lower[..lower.len() - 2].to_string();
    }
    if lower.len() > 5 && lower.ends_with("ing") {
        return lower[..lower.len() - 3].to_string();
    }
    if lower.len() > 4 && lower.ends_with("ed") {
        return lower[..lower.len() - 2].to_string();
    }
    if lower.len() > 5 && lower.ends_with("ery") {
        return lower[..lower.len() - 3].to_string();
    }
    if lower.len() > 3 && lower.ends_with('s') && !lower.ends_with("ss") {
        return lower[..lower.len() - 1].to_string();
    }
    lower
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn word_indices(haystack: &str, needle: &str) -> Vec<usize> {
    let needle = needle.to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let lower = haystack.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = lower[from..].find(&needle) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = lower[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = lower[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            out.push(start);
        }
        from = end;
    }
    out
}

const NEGATED_VERB_PREFIXES: &[&str] = &[
    "not ",
    "never ",
    "no ",
    "didn't ",
    "did not ",
    "doesn't ",
    "does not ",
    "without ",
    "failed to ",
    "refused to ",
    "denied ",
];

const NEGATED_VERB_SUFFIXES: &[&str] = &[" no one", " nobody", " nothing", " not "];

/// Reject "never warned" / "did not mention" / "warned no one" before treating
/// a communication verb as evidence that the learner referenced the fact.
fn communication_is_negated(sentence_lower: &str, verb_index: usize, verb: &str) -> bool {
    let prefix_start = sentence_lower[..verb_index]
        .char_indices()
        .rev()
        .nth(40)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let prefix = &sentence_lower[prefix_start..verb_index];
    if NEGATED_VERB_PREFIXES.iter().any(|cue| prefix.contains(cue)) {
        return true;
    }
    let suffix_end = floor_char_boundary(
        sentence_lower,
        (verb_index + verb.len() + 24).min(sentence_lower.len()),
    );
    let suffix = &sentence_lower[verb_index + verb.len()..suffix_end];
    NEGATED_VERB_SUFFIXES.iter().any(|cue| suffix.contains(cue))
}

fn contains_stemmed_word(haystack: &str, stemmed_needle: &str) -> bool {
    haystack
        .split(|ch: char| !ch.is_alphanumeric() && ch != '\'' && ch != '-')
        .filter(|token| !token.is_empty())
        .any(|token| stem(token) == stemmed_needle)
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return false;
    }
    haystack
        .to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
        .any(|token| token == needle || token.trim_end_matches("'s") == needle)
}

fn contains_phrase(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return false;
    }
    if needle.contains(' ') {
        let lower = haystack.to_ascii_lowercase();
        let mut from = 0usize;
        while let Some(rel) = lower[from..].find(&needle) {
            let start = from + rel;
            let end = start + needle.len();
            let before_ok = haystack[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric());
            let after_ok = haystack[end..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric());
            if before_ok && after_ok {
                return true;
            }
            from = end;
        }
        false
    } else {
        contains_word(haystack, &needle)
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> Option<String> {
    needles
        .iter()
        .find(|needle| contains_phrase(haystack, needle))
        .map(|needle| (*needle).to_string())
}

fn contains_any_token(haystack: &str, needles: &[&str]) -> Option<String> {
    needles
        .iter()
        .find(|needle| contains_word(haystack, needle))
        .map(|needle| (*needle).to_string())
}

/// Sentence spans with byte offsets, keeping trailing punctuation. Abbreviation
/// handling is intentionally conservative; a missed split only widens the
/// evidence window, it never hides a contradiction.
pub fn sentence_spans(text: &str) -> Vec<TextByteRange> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        let boundary = matches!(ch, '.' | '!' | '?') || ch == '\n';
        if !boundary {
            continue;
        }
        let end = index + ch.len_utf8();
        let next_is_boundary = chars
            .peek()
            .is_none_or(|(_, next)| next.is_whitespace() || matches!(next, '.' | '!' | '?' | '\n'));
        let sentence = text[start..end].trim();
        if next_is_boundary && !sentence.is_empty() {
            spans.push(TextByteRange { start, end });
            start = end;
        }
    }
    if start < text.len() {
        let tail = text[start..].trim();
        if !tail.is_empty() {
            spans.push(TextByteRange {
                start,
                end: text.len(),
            });
        }
    }
    spans
}

fn is_dialogue(text: &str, range: TextByteRange) -> bool {
    let sentence = &text[range.start..range.end];
    let trimmed = sentence.trim_start();
    if trimmed.starts_with('"') || trimmed.starts_with('“') {
        return true;
    }
    if sentence.chars().filter(|ch| *ch == '"').count() % 2 == 1 {
        return true;
    }
    let mut inside = false;
    for ch in text[..range.start].chars() {
        match ch {
            '"' => inside = !inside,
            '“' => inside = true,
            '”' => inside = false,
            _ => {}
        }
    }
    inside
}

fn context_quote(text: &str, range: TextByteRange) -> String {
    text[range.start..range.end].trim().to_string()
}

fn finding(
    check_type: &str,
    severity: ClaimValidationSeverity,
    message: String,
    range: TextByteRange,
    entity_ids: Vec<String>,
    evidence: String,
    dedup_key: String,
) -> ClaimValidationFindingV1 {
    ClaimValidationFindingV1 {
        check_type: check_type.to_string(),
        severity,
        message,
        byte_range: Some(range),
        entity_ids,
        lane: ClaimValidationLane::Deterministic,
        evidence_quote: Some(evidence),
        dedup_key,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttributeFamily {
    EyeColor,
    HairColor,
    SkinColor,
    Handedness,
    Height,
    Scar,
    Build,
    Sight,
    BodyPartLoss,
    Credential,
    BodySide,
}

fn attribute_families(key: &str) -> Vec<AttributeFamily> {
    let normalized = key
        .to_ascii_lowercase()
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let has = |word: &str| contains_word(&normalized, word);
    let mut families = Vec::new();
    let color_word = has("color") || has("colour") || has("tone") || has("shade");
    if color_word && (has("eye") || has("eyes") || has("gaze")) {
        families.push(AttributeFamily::EyeColor);
    }
    if color_word && (has("hair") || has("locks") || has("mane")) {
        families.push(AttributeFamily::HairColor);
    }
    if color_word && (has("skin") || has("complexion")) {
        families.push(AttributeFamily::SkinColor);
    }
    if has("handed")
        || has("handedness")
        || has("dominant")
        || (has("hand") && (has("writing") || has("writes") || has("dominant")))
    {
        families.push(AttributeFamily::Handedness);
    }
    if has("height") || has("stature") {
        families.push(AttributeFamily::Height);
    }
    if has("scar") || has("scars") || has("scarred") || has("scarring") {
        families.push(AttributeFamily::Scar);
    }
    if has("build") || has("physique") || has("frame") || (has("body") && has("type")) {
        families.push(AttributeFamily::Build);
    }
    if has("sight")
        || has("vision")
        || has("eyesight")
        || has("blind")
        || has("eye")
        || has("eyes")
        || has("gaze")
    {
        families.push(AttributeFamily::Sight);
    }
    if has("finger") || has("fingers") || has("amputat") {
        families.push(AttributeFamily::BodyPartLoss);
    }
    if has("badge")
        || has("clearance")
        || has("rank")
        || has("status")
        || has("tier")
        || has("grade")
    {
        families.push(AttributeFamily::Credential);
    }
    if has("eye")
        || has("eyebrow")
        || has("forearm")
        || has("arm")
        || has("leg")
        || has("hand")
        || has("shoulder")
        || has("face")
    {
        families.push(AttributeFamily::BodySide);
    }
    families
}

fn family_heads(family: AttributeFamily) -> &'static [&'static str] {
    match family {
        AttributeFamily::EyeColor => &["eye", "eyes", "gaze", "irises", "pupils"],
        AttributeFamily::HairColor => &["hair", "locks", "mane"],
        AttributeFamily::SkinColor => &["skin", "complexion"],
        AttributeFamily::Handedness => &["hand", "hands", "handed", "writes", "writing", "grip"],
        AttributeFamily::Height => &[
            "height",
            "stature",
            "tall",
            "short",
            "small",
            "little",
            "diminutive",
            "shoulder",
            "waist",
            "knee",
            "reach",
            "duck",
            "stoop",
            "bend",
            "crouch",
            "feet",
            "foot",
            "cm",
            "centimeter",
            "centimetre",
            "meter",
            "metre",
        ],
        AttributeFamily::Scar => &[
            "scar", "scars", "scarred", "scarring", "skin", "face", "cheek", "jaw",
        ],
        AttributeFamily::Build => &[
            "build", "frame", "physique", "body", "broad", "slender", "wiry", "muscular", "stocky",
            "heavyset", "thin",
        ],
        AttributeFamily::Sight => &[
            "sight", "vision", "eyesight", "eye", "eyes", "blind", "sighted", "see", "sees", "saw",
            "look", "looks", "clear", "bright",
        ],
        AttributeFamily::BodyPartLoss => &["finger", "fingers", "hand", "hands", "lost", "missing"],
        AttributeFamily::Credential => &["badge", "clearance", "rank", "status", "tier", "grade"],
        AttributeFamily::BodySide => &[
            "eye", "eyes", "eyebrow", "forearm", "arm", "leg", "hand", "shoulder", "face",
        ],
    }
}

fn attribute_head_tokens(key: &str, subject_names: &[String]) -> Vec<String> {
    let names: Vec<String> = subject_names.iter().flat_map(|name| tokens(name)).collect();
    tokens(key)
        .into_iter()
        .map(|token| stem(&token))
        .filter(|token| {
            token.len() >= 3
                && !STOPWORDS.contains(&token.as_str())
                && !GENERIC_TITLES.contains(&token.as_str())
                && !names.contains(token)
        })
        .collect()
}

/// Deterministic unkeyed attribute contradiction: subject + attribute head +
/// an explicit value antonym in the same narration sentence.
pub fn detect_attribute_prose_contradiction(
    scene: &SceneValidationInputV1<'_>,
    fact: &AttributeValidationFactV1,
) -> Option<ClaimValidationFindingV1> {
    if fact.key.trim().is_empty() || fact.value.trim().is_empty() {
        return None;
    }
    let canonical = fact.value.to_ascii_lowercase();
    let mut families = attribute_families(&fact.key);
    if !families.contains(&AttributeFamily::Sight)
        && ["blind", "sighted", "partially sighted"]
            .iter()
            .any(|word| contains_phrase(&canonical, word))
    {
        families.push(AttributeFamily::Sight);
    }
    if !families.contains(&AttributeFamily::BodyPartLoss)
        && ["finger", "fingers", "missing", "amputat"]
            .iter()
            .any(|word| contains_inflected_word(&canonical, word))
    {
        families.push(AttributeFamily::BodyPartLoss);
    }
    if !families.contains(&AttributeFamily::BodySide)
        && (contains_word(&canonical, "left") || contains_word(&canonical, "right"))
        && [
            "eye", "eyebrow", "forearm", "arm", "leg", "hand", "shoulder", "face",
        ]
        .iter()
        .any(|word| contains_inflected_word(&canonical, word))
    {
        families.push(AttributeFamily::BodySide);
    }
    let canonical_prosthetic = visible_terms(&canonical, PROSTHETIC_TERMS);
    let canonical_organic = visible_terms(&canonical, ORGANIC_TERMS);
    let canonical_colors = visible_terms(&canonical, COLOR_TERMS);
    if canonical_prosthetic.is_empty()
        && canonical_organic.is_empty()
        && canonical_colors.is_empty()
        && families.is_empty()
    {
        return None;
    }
    let mut heads = attribute_head_tokens(&fact.key, &fact.subject_names);
    for family in &families {
        heads.extend(family_heads(*family).iter().map(|head| (*head).to_string()));
    }
    heads.sort();
    heads.dedup();
    if heads.is_empty() {
        return None;
    }
    let mut subject_mentioned_earlier = false;
    for range in sentence_spans(scene.text) {
        if is_dialogue(scene.text, range) {
            continue;
        }
        let sentence = &scene.text[range.start..range.end];
        let lower = sentence.to_ascii_lowercase();
        let subject_present = fact
            .subject_names
            .iter()
            .any(|name| contains_phrase(sentence, name))
            || (fact.first_person_subject
                && crate::character_mentions::first_person_narration(sentence));
        let pronoun_subject = !subject_present
            && subject_mentioned_earlier
            && families.contains(&AttributeFamily::Height)
            && ["he", "she", "they"]
                .iter()
                .any(|pronoun| contains_word(&lower, pronoun));
        if subject_present {
            subject_mentioned_earlier = true;
        }
        if !subject_present && !pronoun_subject {
            continue;
        }
        if contains_any(&lower, IRREALIS_MARKERS).is_some() {
            continue;
        }
        let negated = NEGATION_MARKERS
            .iter()
            .filter(|marker| **marker != "instead of " && **marker != "rather than ")
            .any(|marker| contains_phrase(&lower, marker));
        if negated
            && !families
                .iter()
                .any(|family| matches!(family, AttributeFamily::Height | AttributeFamily::Scar))
        {
            continue;
        }
        let lower_sentence = sentence.to_ascii_lowercase();
        if !heads
            .iter()
            .any(|head| contains_inflected_word(&lower_sentence, head))
        {
            continue;
        }
        let numeric_height_verified = families.contains(&AttributeFamily::Height)
            && height_measurement_belongs_to_subject(sentence, fact);
        let mut canonical_terms = Vec::new();
        let mut prose_terms = Vec::new();
        if !canonical_prosthetic.is_empty()
            && let Some(term) = contains_any(&lower, ORGANIC_TERMS)
        {
            canonical_terms.extend(canonical_prosthetic.clone());
            prose_terms.push(term);
        }
        if !canonical_organic.is_empty()
            && let Some(term) = contains_any(&lower, PROSTHETIC_TERMS)
        {
            canonical_terms.extend(canonical_organic.clone());
            prose_terms.push(term);
        }
        if families.is_empty()
            && canonical_terms.is_empty()
            && !canonical_colors.is_empty()
            && let Some(term) = contains_any(&lower, COLOR_TERMS)
            && !canonical_colors.contains(&term)
        {
            canonical_terms.extend(canonical_colors.clone());
            prose_terms.push(term);
        }
        for family in &families {
            if !canonical_terms.is_empty() {
                break;
            }
            if !family_heads(*family)
                .iter()
                .any(|head| contains_inflected_word(&lower_sentence, head))
            {
                continue;
            }
            if let Some((expected, found)) = family_contradiction(
                *family,
                &fact.key,
                &canonical,
                &lower,
                numeric_height_verified,
            ) {
                canonical_terms.push(expected);
                prose_terms.push(found);
            }
        }
        if canonical_terms.is_empty() {
            continue;
        }
        let quote = context_quote(scene.text, range);
        let message = format!(
            "book {} chapter {} scene {}: '{}' conflicts with canonical value '{}' (prose says '{}': {})",
            scene.book_number,
            scene.chapter_number,
            scene.scene_order,
            fact.key.trim(),
            fact.value.trim(),
            prose_terms.join(", "),
            quote
        );
        return Some(finding(
            "canonical_fact_prose_drift",
            ClaimValidationSeverity::Warning,
            message,
            range,
            std::iter::once(fact.fact_id.clone())
                .chain(fact.subject_id.clone())
                .chain(std::iter::once(scene.scene_id.to_string()))
                .collect(),
            quote,
            format!(
                "attribute:{}:{}:{}",
                fact.fact_id,
                scene.scene_id,
                prose_terms.join(",")
            ),
        ));
    }
    None
}

fn earliest_phrase_index(haystack: &str, phrases: &[String]) -> Option<(usize, usize)> {
    let lower = haystack.to_ascii_lowercase();
    phrases
        .iter()
        .filter_map(|phrase| {
            let phrase = phrase.trim().to_ascii_lowercase();
            if phrase.is_empty() {
                return None;
            }
            lower.find(&phrase).map(|index| (index, phrase.len()))
        })
        .min_by_key(|(index, _)| *index)
}

fn contains_inflected_word(haystack: &str, head: &str) -> bool {
    let head = head.to_ascii_lowercase();
    if head.is_empty() {
        return false;
    }
    haystack
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .any(|token| {
            let token = token.to_ascii_lowercase();
            token == head
                || stem(&token) == stem(&head)
                || (token.len() > head.len()
                    && token.starts_with(&head)
                    && matches!(
                        &token[head.len()..],
                        "s" | "es" | "ed" | "ing" | "er" | "est" | "red" | "ness"
                    ))
        })
}

fn family_contradiction(
    family: AttributeFamily,
    key: &str,
    canonical: &str,
    prose: &str,
    numeric_height_verified: bool,
) -> Option<(String, String)> {
    match family {
        AttributeFamily::EyeColor | AttributeFamily::HairColor | AttributeFamily::SkinColor => {
            let canonical_colors = visible_terms(canonical, COLOR_TERMS);
            if canonical_colors.is_empty() {
                return None;
            }
            let term = contains_any(prose, COLOR_TERMS)?;
            if canonical_colors.contains(&term) {
                return None;
            }
            Some((canonical_colors.join(","), term))
        }
        AttributeFamily::Handedness => {
            let canonical_side = handedness_side(canonical)?;
            let prose_side = handedness_side(prose)?;
            (canonical_side != prose_side)
                .then(|| (canonical_side.to_string(), prose_side.to_string()))
        }
        AttributeFamily::Height => {
            if numeric_height_verified
                && let (Some(canonical_inches), Some(prose_inches)) =
                    (parse_height_inches(canonical), parse_height_inches(prose))
            {
                if (canonical_inches - prose_inches).abs() > 1.0 {
                    return Some((
                        format!("{canonical_inches:.0} in"),
                        format!("{prose_inches:.0} in"),
                    ));
                }
                return None;
            }
            let canonical_height = height_side(canonical)?;
            let prose_height = height_side(prose)?;
            (canonical_height != prose_height)
                .then(|| (canonical_height.to_string(), prose_height.to_string()))
        }
        AttributeFamily::Scar => {
            let canonical_side = body_side(canonical);
            let prose_side = body_side(prose);
            if let (Some(canonical_side), Some(prose_side)) = (canonical_side, prose_side)
                && canonical_side != prose_side
            {
                return Some((canonical_side.to_string(), prose_side.to_string()));
            }
            let canonical_scarred = scar_state(canonical).or_else(|| {
                let trimmed = canonical.trim();
                (!trimmed.is_empty()
                    && !contains_phrase(trimmed, "no scar")
                    && !contains_phrase(trimmed, "no scars")
                    && !["none", "clear", "unscarred", "unblemished"]
                        .iter()
                        .any(|word| contains_word(trimmed, word)))
                .then_some("scarred")
            })?;
            let prose_scarred = scar_state(prose)?;
            (canonical_scarred != prose_scarred)
                .then(|| (canonical_scarred.to_string(), prose_scarred.to_string()))
        }
        AttributeFamily::Build => {
            let canonical_build = build_side(canonical)?;
            let prose_build = build_side(prose)?;
            (canonical_build != prose_build)
                .then(|| (canonical_build.to_string(), prose_build.to_string()))
        }
        AttributeFamily::Sight => {
            let canonical_sight = sight_state(canonical)?;
            let prose_sight = sight_state(prose)?;
            (canonical_sight != prose_sight)
                .then(|| (canonical_sight.to_string(), prose_sight.to_string()))
        }
        AttributeFamily::BodyPartLoss => {
            let canonical_count = lost_finger_count(canonical)?;
            let canonical_side = body_side(canonical).or_else(|| body_side(key));
            let prose_side = body_side(prose);
            if canonical_side.is_some() && prose_side.is_some() && canonical_side != prose_side {
                return None;
            }
            let prose_lost = ["lost", "missing", "amputated"]
                .iter()
                .any(|word| contains_inflected_word(prose, word));
            let prose_total = finger_total(prose);
            if prose_lost || prose_total.is_some_and(|total| total > 5 - canonical_count) {
                Some((canonical_count.to_string(), "lost".to_string()))
            } else {
                None
            }
        }
        AttributeFamily::Credential => {
            let canonical_tokens = hyphenated_tokens(canonical);
            let prose_tokens = hyphenated_tokens(prose);
            if canonical_tokens.is_empty()
                || prose_tokens.is_empty()
                || prose_tokens
                    .iter()
                    .any(|token| canonical_tokens.contains(token))
            {
                return None;
            }
            Some((canonical_tokens.join(","), prose_tokens.join(",")))
        }
        AttributeFamily::BodySide => {
            let canonical_side = body_side(canonical)?;
            let prose_first_eye_side = first_eye_side(prose);
            if let Some(prose_side) = prose_first_eye_side {
                return (canonical_side != prose_side)
                    .then(|| (canonical_side.to_string(), prose_side.to_string()));
            }
            let prose_side = body_side(prose)?;
            (canonical_side != prose_side)
                .then(|| (canonical_side.to_string(), prose_side.to_string()))
        }
    }
}

fn handedness_side(text: &str) -> Option<&'static str> {
    if contains_word(text, "ambidextrous") {
        return Some("ambidextrous");
    }
    // Prefer an explicit handedness phrase; then a side directly attached to
    // "hand". This avoids treating an unrelated "right" elsewhere in the same
    // sentence (e.g. "left-handed ... right") as ambiguity.
    for (side, phrases) in [
        ("left", ["left-handed", "left handed"]),
        ("right", ["right-handed", "right handed"]),
    ] {
        if phrases.iter().any(|phrase| contains_phrase(text, phrase)) {
            return Some(side);
        }
    }
    let mentions_hand = ["hand", "hands", "writes", "writing", "grip", "dominant"]
        .iter()
        .any(|word| contains_inflected_word(text, word));
    if mentions_hand {
        let left_hand = ["left hand", "left-hand", "left hands"]
            .iter()
            .any(|phrase| contains_phrase(text, phrase));
        let right_hand = ["right hand", "right-hand", "right hands"]
            .iter()
            .any(|phrase| contains_phrase(text, phrase));
        match (left_hand, right_hand) {
            (true, false) => return Some("left"),
            (false, true) => return Some("right"),
            _ => {}
        }
    }
    let left = contains_inflected_word(text, "left");
    let right = contains_inflected_word(text, "right");
    if contains_word(text, "left") && !mentions_hand {
        return Some("left");
    }
    if contains_word(text, "right") && !mentions_hand {
        return Some("right");
    }
    if !mentions_hand {
        return None;
    }
    match (left, right) {
        (true, false) => Some("left"),
        (false, true) => Some("right"),
        _ => None,
    }
}

fn height_side(text: &str) -> Option<&'static str> {
    let not_tall = [
        "not tall",
        "isn't tall",
        "is not tall",
        "wasn't tall",
        "was not tall",
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase));
    let not_short = [
        "not short",
        "isn't short",
        "is not short",
        "wasn't short",
        "was not short",
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase));
    if not_tall {
        return Some("short");
    }
    if not_short {
        return Some("tall");
    }
    let relational_short = [
        "waist-high",
        "waist high",
        "knee-high",
        "knee high",
        "shoulder-high",
        "shoulder high",
        "head shorter",
        "foot shorter",
        "came up to",
        "barely reached",
        "reached only",
        "reached barely",
        "could not reach",
        "couldn't reach",
        "needed a stool",
        "needed a step",
        "stood on tiptoe",
        "on her toes",
        "on his toes",
        "had to climb",
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase));
    if relational_short {
        return Some("short");
    }
    let relational_tall = [
        "towered over",
        "head and shoulders above",
        "ducked under",
        "stooped under",
        "had to duck",
        "had to stoop",
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase));
    if relational_tall {
        return Some("tall");
    }
    let tall = contains_inflected_word(text, "tall") || contains_word(text, "towering");
    let short = contains_inflected_word(text, "short")
        || contains_word(text, "petite")
        || contains_word(text, "small")
        || contains_word(text, "little")
        || contains_word(text, "diminutive")
        || contains_word(text, "stunted")
        || contains_word(text, "undersized");
    match (tall, short) {
        (true, false) => Some("tall"),
        (false, true) => Some("short"),
        _ => None,
    }
}

fn body_side(text: &str) -> Option<&'static str> {
    let left = contains_inflected_word(text, "left");
    let right = contains_inflected_word(text, "right");
    match (left, right) {
        (true, false) => Some("left"),
        (false, true) => Some("right"),
        _ => None,
    }
}

fn scar_state(text: &str) -> Option<&'static str> {
    if [
        "never existed",
        "was gone",
        "is gone",
        "had gone",
        "vanished",
        "healed",
        "faded",
        "no longer",
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase))
    {
        return Some("unscarred");
    }
    let scarred = [
        "scar", "scars", "scarred", "scarring", "mark", "seam", "blemish", "pucker", "notch",
    ]
    .iter()
    .any(|word| contains_inflected_word(text, word));
    let clear = [
        "unscarred",
        "unmarred",
        "unblemished",
        "unmarked",
        "smooth",
        "flawless",
        "clear",
    ]
    .iter()
    .any(|word| contains_inflected_word(text, word))
        || contains_phrase(text, "no scar")
        || contains_phrase(text, "no scars")
        || contains_phrase(text, "without a scar")
        || contains_phrase(text, "not scarred")
        || contains_phrase(text, "never scarred");
    match (scarred, clear) {
        (true, false) => Some("scarred"),
        (false, true) => Some("unscarred"),
        _ => None,
    }
}

fn build_side(text: &str) -> Option<&'static str> {
    let broad = [
        "broad", "heavyset", "stocky", "muscular", "bulky", "thickset",
    ];
    let slender = ["slender", "thin", "wiry", "lithe", "slight", "lean"];
    let broad_hit = broad.iter().any(|word| contains_inflected_word(text, word));
    let slender_hit = slender
        .iter()
        .any(|word| contains_inflected_word(text, word));
    match (broad_hit, slender_hit) {
        (true, false) => Some("broad"),
        (false, true) => Some("slender"),
        _ => None,
    }
}

fn number_word(token: &str) -> Option<f64> {
    Some(match token {
        "zero" => 0.0,
        "one" => 1.0,
        "two" => 2.0,
        "three" => 3.0,
        "four" => 4.0,
        "five" => 5.0,
        "six" => 6.0,
        "seven" => 7.0,
        "eight" => 8.0,
        "nine" => 9.0,
        "ten" => 10.0,
        "eleven" => 11.0,
        "twelve" => 12.0,
        "thirteen" => 13.0,
        "fourteen" => 14.0,
        "fifteen" => 15.0,
        "sixteen" => 16.0,
        "seventeen" => 17.0,
        "eighteen" => 18.0,
        "nineteen" => 19.0,
        "twenty" => 20.0,
        "thirty" => 30.0,
        "forty" => 40.0,
        "fifty" => 50.0,
        "sixty" => 60.0,
        "seventy" => 70.0,
        "eighty" => 80.0,
        "ninety" => 90.0,
        "hundred" => 100.0,
        _ => return None,
    })
}

fn normalize_height_number_words(text: &str) -> String {
    text.split_whitespace()
        .map(|token| {
            let cleaned = token.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '.');
            number_word(&cleaned.to_ascii_lowercase())
                .map(|number| number.to_string())
                .unwrap_or_else(|| token.to_string())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_height_measurement(text: &str) -> Option<(f64, usize, usize)> {
    let normalized = normalize_height_number_words(&text.to_ascii_lowercase());
    let cm = regex::Regex::new(r"(\d+(?:\.\d+)?)\s*(?:cm|centimet(?:er|re)s?)")
        .ok()?
        .captures(&normalized);
    if let Some(captures) = cm
        && let Some(value) = captures.get(1).and_then(|m| m.as_str().parse::<f64>().ok())
        && let Some(full) = captures.get(0)
    {
        return Some((value / 2.54, full.start(), full.end()));
    }
    let meters = regex::Regex::new(r"(\d+(?:\.\d+)?)\s*(?:m|meters?|metres?)\b")
        .ok()?
        .captures(&normalized);
    if let Some(captures) = meters
        && let Some(value) = captures.get(1).and_then(|m| m.as_str().parse::<f64>().ok())
        && let Some(full) = captures.get(0)
    {
        return Some((value * 39.370_078_740_157_48, full.start(), full.end()));
    }
    let feet = regex::Regex::new(
        r#"(\d+(?:\.\d+)?)\s*(?:'|ft|feet|foot)\s*(?:(\d+(?:\.\d+)?)\s*(?:"|inch|inches)?)?"#,
    )
    .ok()?
    .captures(&normalized)?;
    let feet_value = feet.get(1)?.as_str().parse::<f64>().ok()?;
    let inches_value = feet
        .get(2)
        .and_then(|m| m.as_str().parse::<f64>().ok())
        .unwrap_or(0.0);
    let full = feet.get(0)?;
    Some((feet_value * 12.0 + inches_value, full.start(), full.end()))
}

fn parse_height_inches(text: &str) -> Option<f64> {
    parse_height_measurement(text).map(|(value, _, _)| value)
}

fn height_measurement_belongs_to_subject(sentence: &str, fact: &AttributeValidationFactV1) -> bool {
    let normalized_case = normalize_height_number_words(sentence);
    let normalized = normalized_case.to_ascii_lowercase();
    let Some((_, measurement_start, measurement_end)) = parse_height_measurement(&normalized)
    else {
        return false;
    };
    let Some((subject_start, subject_end)) = subject_position(&normalized, fact) else {
        return false;
    };
    let (between_case, gap) = if measurement_start >= subject_end {
        (
            &normalized_case[subject_end..measurement_start],
            measurement_start - subject_end,
        )
    } else if subject_start >= measurement_end {
        (
            &normalized_case[measurement_end..subject_start],
            subject_start - measurement_end,
        )
    } else {
        return false;
    };
    if gap > 80 {
        return false;
    }
    // The word immediately before the number must express stature, not merely
    // appear somewhere before it ("was carrying a five foot ladder").
    let before_measurement = normalized[..measurement_start]
        .trim_end_matches(|ch: char| !ch.is_alphanumeric() && !ch.is_whitespace());
    let relation_word = before_measurement
        .split_whitespace()
        .next_back()
        .unwrap_or_default()
        .trim_matches(|ch: char| !ch.is_alphanumeric())
        .to_ascii_lowercase();
    if !matches!(
        relation_word.as_str(),
        "stood"
            | "stands"
            | "stand"
            | "was"
            | "were"
            | "is"
            | "are"
            | "measured"
            | "measures"
            | "height"
            | "stature"
    ) {
        return false;
    }
    // A measured object noun after the number ("five foot ladder", "five feet
    // of rope", "five feet from the door") is not a height assertion. Accept a
    // sentence boundary or an explicit height qualifier/unit follow-up.
    let after_measurement = normalized[measurement_end..]
        .split(|ch: char| ch.is_ascii_punctuation() || ch == '\n')
        .next()
        .unwrap_or_default()
        .trim();
    if let Some(first) = after_measurement.split_whitespace().next() {
        let first = first
            .trim_matches(|ch: char| !ch.is_alphanumeric())
            .to_ascii_lowercase();
        if !matches!(
            first.as_str(),
            "tall" | "short" | "in" | "inch" | "inches" | "and" | "with"
        ) {
            return false;
        }
    }
    // A different capitalized name between the subject and the measurement
    // means the measurement likely belongs to that person, not the subject.
    let another_name = between_case.split_whitespace().any(|token| {
        let token = token.trim_matches(|ch: char| !ch.is_alphanumeric());
        token.chars().next().is_some_and(char::is_uppercase)
            && !fact
                .subject_names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(token))
    });
    !another_name
}

fn subject_position(text: &str, fact: &AttributeValidationFactV1) -> Option<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    for name in &fact.subject_names {
        let name = name.to_ascii_lowercase();
        if let Some(start) = lower.find(&name) {
            return Some((start, start + name.len()));
        }
    }
    if fact.first_person_subject {
        let mut offset = 0usize;
        for token in lower.split_inclusive(|ch: char| !ch.is_alphanumeric()) {
            let cleaned = token.trim_matches(|ch: char| !ch.is_alphanumeric());
            if matches!(cleaned, "i" | "me" | "my" | "mine" | "myself") {
                let start = offset + token.find(cleaned).unwrap_or(0);
                return Some((start, start + cleaned.len()));
            }
            offset += token.len();
        }
    }
    None
}

fn sight_state(text: &str) -> Option<&'static str> {
    if ["partially sighted", "partial sight", "low vision"]
        .iter()
        .any(|phrase| contains_phrase(text, phrase))
    {
        return Some("partially sighted");
    }
    // Clear/bright/seeing descriptors assert usable sight even when the prose
    // also repeats the word "blind" ("his blind eye was clear and brown").
    if [
        "clear",
        "bright",
        "seeing",
        "open",
        "sighted",
        "can see",
        "could see",
        "saw",
        "sees",
        "see",
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase))
    {
        return Some("sighted");
    }
    if [
        "blind",
        "no sight",
        "cannot see",
        "can't see",
        "could not see",
        "couldn't see",
    ]
    .iter()
    .any(|phrase| contains_phrase(text, phrase))
    {
        return Some("blind");
    }
    None
}

fn lost_finger_count(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    if !["lost", "missing", "amputat"]
        .iter()
        .any(|word| contains_inflected_word(&lower, word))
        || !contains_inflected_word(&lower, "finger")
    {
        return None;
    }
    for (word, count) in [
        ("one", 1),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("all", 5),
        ("ten", 10),
    ] {
        if contains_word(&lower, word) {
            return Some(count);
        }
    }
    Some(5)
}

fn finger_total(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    if !contains_inflected_word(&lower, "finger") {
        return None;
    }
    for (word, count) in [
        ("one", 1),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("ten", 10),
        ("all", 5),
    ] {
        if contains_word(&lower, word) {
            return Some(count);
        }
    }
    None
}

fn hyphenated_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|token| token.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '-'))
        .filter(|token| token.contains('-') && token.len() >= 4)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn first_eye_side(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    let left = lower.find("left eye");
    let right = lower.find("right eye");
    match (left, right) {
        (Some(l), Some(r)) if l < r => Some("left"),
        (Some(_), Some(_)) => Some("right"),
        (Some(_), None) => Some("left"),
        (None, Some(_)) => Some("right"),
        _ => None,
    }
}

fn visible_terms(haystack: &str, terms: &[&str]) -> Vec<String> {
    terms
        .iter()
        .filter(|term| contains_phrase(haystack, term))
        .map(|term| (*term).to_string())
        .collect()
}

// ── deterministic knowledge paraphrase lane ──────────────────────────────

fn distinctive_tokens(text: &str, excluded_names: &[String]) -> Vec<String> {
    let excluded: Vec<String> = excluded_names
        .iter()
        .flat_map(|name| tokens(name))
        .map(|token| stem(&token))
        .collect();
    let mut out = Vec::new();
    for token in tokens(text) {
        let stemmed = stem(&token);
        if stemmed.len() < 4
            || STOPWORDS.contains(&stemmed.as_str())
            || GENERIC_TITLES.contains(&stemmed.as_str())
            || excluded.contains(&stemmed)
            || out.contains(&stemmed)
        {
            continue;
        }
        out.push(stemmed);
    }
    out
}

/// Deterministic paraphrase leak: the learner is named near at least three
/// distinctive fact tokens plus a communication/recording verb, before the
/// recorded learned position (the caller filters that position).
pub fn detect_knowledge_paraphrase_leak(
    scene: &SceneValidationInputV1<'_>,
    fact: &KnowledgeValidationFactV1,
) -> Option<ClaimValidationFindingV1> {
    let mut excluded_names = fact.character_names.clone();
    excluded_names.extend(fact.character_aliases.iter().cloned());
    let fact_tokens = distinctive_tokens(&fact.fact, &excluded_names);
    if fact_tokens.len() < 3 {
        return None;
    }
    for range in sentence_spans(scene.text) {
        let sentence = &scene.text[range.start..range.end];
        let lower = sentence.to_ascii_lowercase();
        if contains_any(&lower, KNOWLEDGE_IRREALIS).is_some() {
            continue;
        }
        let mention_aliases = if fact.character_aliases.is_empty() {
            &fact.character_names
        } else {
            &fact.character_aliases
        };
        let mentioned = mention_aliases
            .iter()
            .any(|name| contains_phrase(sentence, name))
            || (fact.first_person_subject
                && crate::character_mentions::first_person_narration(sentence));
        if !mentioned {
            continue;
        }
        let leak_verb_present = LEAK_VERBS.iter().any(|verb| {
            word_indices(&lower, verb)
                .into_iter()
                .any(|index| !communication_is_negated(&lower, index, verb))
        });
        if !leak_verb_present {
            continue;
        }
        let matched: Vec<String> = fact_tokens
            .iter()
            .filter(|token| contains_stemmed_word(&lower, token))
            .cloned()
            .collect();
        // Two strong content tokens are enough; a single generic overlap is
        // not. At least one token must carry substantive length/specificity.
        if matched.len() < 2 || !matched.iter().any(|token| token.len() >= 5) {
            continue;
        }
        let quote = context_quote(scene.text, range);
        let message = format!(
            "book {} chapter {} scene {}: '{}' references '{}' before learning it in book {} chapter {} (paraphrased: '{}')",
            scene.book_number,
            scene.chapter_number,
            scene.scene_order,
            fact.character_names.join(" / "),
            fact.fact,
            fact.learned_book_number,
            fact.learned_chapter_number,
            quote
        );
        return Some(finding(
            "knowledge_timing",
            ClaimValidationSeverity::Warning,
            message,
            range,
            vec![
                fact.fact_id.clone(),
                fact.character_id.clone(),
                scene.scene_id.to_string(),
            ],
            quote,
            format!("knowledge:{}:{}", fact.fact_id, scene.scene_id),
        ));
    }
    None
}

// ── deterministic secret narration lane ─────────────────────────────────

/// Secret value in a narration sentence that names an out-of-circle character.
/// Dialogue attribution is handled by the existing attributed-dialogue lane;
/// this complements it for unquoted narration.
pub fn detect_secret_narration_leak(
    scene: &SceneValidationInputV1<'_>,
    candidate: &SecretValidationCandidateV1,
) -> Option<ClaimValidationFindingV1> {
    if candidate.value.trim().is_empty() || candidate.value == "<unset>" {
        return None;
    }
    for range in sentence_spans(scene.text) {
        if is_dialogue(scene.text, range) {
            continue;
        }
        let sentence = &scene.text[range.start..range.end];
        let lower = sentence.to_ascii_lowercase();
        if contains_any(&lower, NEGATION_MARKERS).is_some() {
            continue;
        }
        let lower_sentence = sentence.to_ascii_lowercase();
        let mention_aliases = if candidate.character_aliases.is_empty() {
            std::slice::from_ref(&candidate.character_name)
        } else {
            candidate.character_aliases.as_slice()
        };
        let Some((name_index, name_len)) = earliest_phrase_index(&lower_sentence, mention_aliases)
        else {
            continue;
        };
        let value_index = lower_sentence.find(&candidate.value.to_ascii_lowercase());
        let paraphrased = value_index.is_none();
        if !paraphrased {
            let value_index = value_index.unwrap_or_default();
            if name_index >= value_index {
                continue;
            }
        }
        if paraphrased
            && !secret_content_overlap(sentence, &candidate.value, &candidate.character_name)
        {
            continue;
        }
        let window_end = value_index.unwrap_or(sentence.len());
        let actor_window = &sentence[name_index + name_len..window_end];
        let mut other_names = candidate.other_present_character_names.clone();
        other_names.extend(candidate.other_present_character_aliases.iter().cloned());
        if other_names
            .iter()
            .any(|name| contains_phrase(actor_window, name))
        {
            continue;
        }
        if contains_any_token(&actor_window.to_ascii_lowercase(), LEAK_VERBS).is_none() {
            continue;
        }
        let quote = context_quote(scene.text, range);
        let message = format!(
            "book {} chapter {} scene {}: {} narrates secret fact '{}' ('{}') outside dialogue — no recorded reveal to {} before this scene",
            scene.book_number,
            scene.chapter_number,
            scene.scene_order,
            candidate.character_name,
            candidate.predicate,
            candidate.value,
            candidate.character_name
        );
        return Some(finding(
            "secret_leak",
            ClaimValidationSeverity::Warning,
            message,
            range,
            vec![
                candidate.fact_id.clone(),
                candidate.character_id.clone(),
                scene.scene_id.to_string(),
            ],
            quote,
            format!(
                "secret-narration:{}:{}:{}",
                candidate.fact_id, candidate.character_id, scene.scene_id
            ),
        ));
    }
    None
}

pub fn secret_content_overlap(text: &str, value: &str, excluded_names: &str) -> bool {
    let excluded = if excluded_names.trim().is_empty() {
        Vec::new()
    } else {
        vec![excluded_names.to_string()]
    };
    let tokens = distinctive_tokens(value, &excluded);
    let lower = text.to_ascii_lowercase();
    let matched: Vec<&String> = tokens
        .iter()
        .filter(|token| contains_stemmed_word(&lower, token))
        .collect();
    matched.len() >= 2 && matched.iter().any(|token| token.len() >= 5)
}

const SECRET_DISCLOSURE_VERBS: &[&str] = &[
    "said",
    "says",
    "shouted",
    "shouts",
    "announced",
    "announces",
    "printed",
    "prints",
    "wrote",
    "writes",
    "written",
    "reported",
    "reports",
    "claimed",
    "claims",
    "spread",
    "spreads",
    "told",
    "tells",
    "whispered",
    "whispers",
    "murmured",
    "murmurs",
    "repeated",
    "repeats",
    "knew",
    "knows",
    "know",
    "learned",
    "learns",
    "disclosed",
    "discloses",
    "revealed",
    "reveals",
    "published",
    "publishes",
    "posted",
    "posts",
    "carved",
    "carves",
    "sang",
    "sings",
];

const SECRET_DISCLOSURE_PHRASES: &[&str] = &["read aloud", "read out"];

const SECRET_GENERIC_ACTORS: &[&str] = &[
    "everyone",
    "everybody",
    "someone",
    "somebody",
    "people",
    "crowd",
    "town",
    "village",
    "dockworker",
    "dockworkers",
    "worker",
    "workers",
    "clerk",
    "clerks",
    "carrier",
    "carriers",
    "picker",
    "pickers",
    "sailor",
    "sailors",
    "guard",
    "guards",
    "servant",
    "servants",
    "merchant",
    "merchants",
    "courier",
    "couriers",
    "reader",
    "readers",
];

const SECRET_WRITTEN_CHANNELS: &[&str] = &[
    "broadsheet",
    "broadsheets",
    "notice",
    "noticeboard",
    "board",
    "letter",
    "letters",
    "headline",
    "headlines",
    "gazette",
    "placard",
    "placards",
    "handbill",
    "handbills",
    "printed",
    "posted",
    "published",
    "read aloud",
];

fn contains_any_word(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| contains_inflected_word(text, word))
}

/// Paraphrased secret disclosure through a generic/collective actor, reported
/// speech or a written notice. Named-character dialogue/narration remains in
/// the caller's circle-aware lane.
pub fn detect_secret_generic_disclosure(
    scene: &SceneValidationInputV1<'_>,
    fact_id: &str,
    value: &str,
    holder_names: &[String],
    in_circle_names: &[String],
) -> Option<ClaimValidationFindingV1> {
    if value.trim().is_empty() || value == "<unset>" {
        return None;
    }
    let tokens = distinctive_tokens(value, &[]);
    for range in sentence_spans(scene.text) {
        let sentence = &scene.text[range.start..range.end];
        let lower = sentence.to_ascii_lowercase();
        if contains_any(&lower, IRREALIS_MARKERS).is_some()
            || contains_any(&lower, NEGATION_MARKERS).is_some()
        {
            continue;
        }
        if [
            "in a dream",
            "dreamed",
            "dream",
            "remembered",
            "memory",
            "flashback",
            "years ago",
            "before the death",
            "before her death",
            "before his death",
            "rumor",
            "rumour",
            "legend",
        ]
        .iter()
        .any(|marker| contains_phrase(&lower, marker))
        {
            continue;
        }
        let matched = tokens
            .iter()
            .filter(|token| contains_stemmed_word(&lower, token))
            .count();
        if matched < 2
            || !tokens
                .iter()
                .any(|token| token.len() >= 5 && contains_stemmed_word(&lower, token))
        {
            continue;
        }
        let read_aloud = (contains_word(&lower, "read") || contains_word(&lower, "reads"))
            && (contains_word(&lower, "aloud") || contains_word(&lower, "out"));
        let verb = contains_any_word(&lower, SECRET_DISCLOSURE_VERBS)
            || read_aloud
            || SECRET_DISCLOSURE_PHRASES
                .iter()
                .any(|phrase| contains_phrase(&lower, phrase));
        let written = contains_any_word(&lower, SECRET_WRITTEN_CHANNELS);
        let generic_actor = contains_any_word(&lower, SECRET_GENERIC_ACTORS);
        if !(verb && (written || generic_actor)) {
            continue;
        }
        // If an in-circle actor/author appears before the first disclosure verb,
        // treat the sentence as the holder's own knowledge/private writing.
        let mut verb_positions: Vec<usize> = SECRET_DISCLOSURE_VERBS
            .iter()
            .filter_map(|verb| lower.match_indices(verb).next().map(|(pos, _)| pos))
            .collect();
        if read_aloud && let Some(pos) = lower.find("read").or_else(|| lower.find("reads")) {
            verb_positions.push(pos);
        }
        let first_verb_pos = verb_positions.into_iter().min().unwrap_or(lower.len());
        let in_circle_before = in_circle_names.iter().any(|name| {
            let name = name.to_ascii_lowercase();
            lower
                .match_indices(&name)
                .next()
                .is_some_and(|(pos, _)| pos < first_verb_pos)
        });
        let holder_before = holder_names.iter().any(|name| {
            let name = name.to_ascii_lowercase();
            lower
                .match_indices(&name)
                .next()
                .is_some_and(|(pos, _)| pos < first_verb_pos)
        });
        if in_circle_before || holder_before {
            continue;
        }
        let quote = context_quote(scene.text, range);
        let message = format!(
            "book {} chapter {} scene {}: paraphrased secret fact '{}' is disclosed through a generic/reported/written channel — no recorded reveal before this scene ({})",
            scene.book_number, scene.chapter_number, scene.scene_order, fact_id, quote
        );
        return Some(finding(
            "secret_leak",
            ClaimValidationSeverity::Warning,
            message,
            range,
            vec![fact_id.to_string(), scene.scene_id.to_string()],
            quote,
            format!("secret-generic:{}:{}", fact_id, scene.scene_id),
        ));
    }
    None
}

// ── report assembly ────────────────────────────────────────────────────

pub fn validate_scene_claims(
    scene: &SceneValidationInputV1<'_>,
    context: &ClaimValidationContextV1,
    adjudicator: Option<&dyn SemanticClaimAdjudicator>,
) -> ClaimValidationReportV1 {
    let mut findings = Vec::new();
    let mut lexical_prefilter_hit_count = 0usize;
    for fact in &context.attributes {
        if lexical_prefilter_hit(scene.text, &fact.key) {
            lexical_prefilter_hit_count += 1;
        }
        if let Some(finding) = detect_attribute_prose_contradiction(scene, fact) {
            findings.push(finding);
        }
    }
    for fact in &context.knowledge {
        if let Some(finding) = detect_knowledge_paraphrase_leak(scene, fact) {
            findings.push(finding);
        }
    }
    for candidate in &context.secrets {
        if let Some(finding) = detect_secret_narration_leak(scene, candidate) {
            findings.push(finding);
        }
    }
    let semantic_status = match adjudicator {
        None => ClaimValidationLaneStatus::Degraded {
            reason: "no semantic adjudication route configured; deterministic lanes only".into(),
        },
        Some(port) => {
            let request = SemanticAdjudicationRequestV1 {
                scene_id: scene.scene_id.to_string(),
                text: scene.text.to_string(),
                claims: context
                    .attributes
                    .iter()
                    .map(|fact| SemanticClaimV1 {
                        check_type: "canonical_fact_prose_drift".into(),
                        description: format!("{} = {}", fact.key, fact.value),
                        canonical_value: fact.value.clone(),
                        evidence_quote: scene.text.to_string(),
                    })
                    .chain(context.knowledge.iter().map(|fact| SemanticClaimV1 {
                        check_type: "knowledge_timing".into(),
                        description: fact.fact.clone(),
                        canonical_value: format!(
                            "learned in chapter {}",
                            fact.learned_chapter_number
                        ),
                        evidence_quote: scene.text.to_string(),
                    }))
                    .collect(),
            };
            match port.adjudicate(&request) {
                Ok(extra) => {
                    findings.extend(extra);
                    ClaimValidationLaneStatus::Complete
                }
                Err(reason) => ClaimValidationLaneStatus::Degraded {
                    reason: format!("semantic adjudication unavailable: {reason}"),
                },
            }
        }
    };
    ClaimValidationReportV1 {
        findings,
        lexical_prefilter_hit_count,
        lane_statuses: vec![
            ClaimValidationLaneStatusEntryV1 {
                lane: ClaimValidationLane::LexicalPrefilter,
                status: ClaimValidationLaneStatus::Complete,
            },
            ClaimValidationLaneStatusEntryV1 {
                lane: ClaimValidationLane::Deterministic,
                status: ClaimValidationLaneStatus::Complete,
            },
            ClaimValidationLaneStatusEntryV1 {
                lane: ClaimValidationLane::SemanticAdjudication,
                status: semantic_status.clone(),
            },
        ],
        semantic_status,
    }
}

#[cfg(test)]
#[path = "claim_validation_tests.rs"]
mod tests;
