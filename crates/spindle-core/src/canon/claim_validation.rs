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
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeValidationFactV1 {
    pub fact_id: String,
    pub character_id: String,
    pub character_names: Vec<String>,
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
    /// Other present characters used to reject a sentence where the candidate
    /// is merely mentioned and someone else performs the leak action.
    pub other_present_character_names: Vec<String>,
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
    "records",
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
    let canonical_prosthetic = visible_terms(&canonical, PROSTHETIC_TERMS);
    let canonical_organic = visible_terms(&canonical, ORGANIC_TERMS);
    let canonical_colors = visible_terms(&canonical, COLOR_TERMS);
    if canonical_prosthetic.is_empty()
        && canonical_organic.is_empty()
        && canonical_colors.is_empty()
    {
        return None;
    }
    let heads = attribute_head_tokens(&fact.key, &fact.subject_names);
    if heads.is_empty() {
        return None;
    }
    for range in sentence_spans(scene.text) {
        if is_dialogue(scene.text, range) {
            continue;
        }
        let sentence = &scene.text[range.start..range.end];
        let lower = sentence.to_ascii_lowercase();
        if contains_any(&lower, IRREALIS_MARKERS).is_some()
            || contains_any(&lower, NEGATION_MARKERS).is_some()
        {
            continue;
        }
        if !fact
            .subject_names
            .iter()
            .any(|name| contains_phrase(sentence, name))
        {
            continue;
        }
        if !heads
            .iter()
            .any(|head| contains_word(&sentence.to_ascii_lowercase(), head))
        {
            continue;
        }
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
        if canonical_terms.is_empty()
            && !canonical_colors.is_empty()
            && let Some(term) = contains_any(&lower, COLOR_TERMS)
            && !canonical_colors.contains(&term)
        {
            canonical_terms.extend(canonical_colors.clone());
            prose_terms.push(term);
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
    let fact_tokens = distinctive_tokens(&fact.fact, &fact.character_names);
    if fact_tokens.len() < 3 {
        return None;
    }
    for range in sentence_spans(scene.text) {
        let sentence = &scene.text[range.start..range.end];
        let lower = sentence.to_ascii_lowercase();
        if contains_any(&lower, KNOWLEDGE_IRREALIS).is_some() {
            continue;
        }
        if !fact
            .character_names
            .iter()
            .any(|name| contains_phrase(sentence, name))
        {
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
            .filter(|token| contains_word(&lower, token))
            .cloned()
            .collect();
        if matched.len() < 3 {
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
        let Some(name_index) = sentence
            .to_ascii_lowercase()
            .find(&candidate.character_name.to_ascii_lowercase())
        else {
            continue;
        };
        let Some(value_index) = sentence
            .to_ascii_lowercase()
            .find(&candidate.value.to_ascii_lowercase())
        else {
            continue;
        };
        if name_index >= value_index {
            continue;
        }
        let actor_window = &sentence[name_index + candidate.character_name.len()..value_index];
        if candidate
            .other_present_character_names
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
