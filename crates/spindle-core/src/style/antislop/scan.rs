use super::chapter::{ChapterCounters, is_chapter_scoped};
use super::pack::{GenreOverride, RewriteMode, Severity, ShelfPack, ShelfSpec};
use super::structural::{StructuralObservation, experimental_structural_observations};
use super::suppress::{SuppressionStore, recount_after_suppress};
use crate::models::TextByteRange;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// Options for [`scan_with_options`]. Defaults match Phases 0–4: no chapter
/// prior, no suppressions, experimental structural off.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScanOptions<'a> {
    pub overlay: Option<&'a GenreOverride>,
    pub chapter_prior: Option<&'a ChapterCounters>,
    pub suppressions: Option<&'a SuppressionStore>,
    pub experimental_structural: bool,
}

/// Shelf IDs the scanner actually handles. CI drift fails if this set
/// diverges from the catalog markdown or the v0 pack.
pub const SCANNER_SHELF_IDS: &[&str] = &[
    "contrast_not_x_but_y",
    "emotion_cocktail",
    "fishing_ending",
    "said_bookism",
    "body_reactions",
    "eye_department",
    "gesture_rack",
    "atmosphere_prefabs",
    "naming_watchlist",
    "rhythm_cadence",
    "triadic_listing",
    "solitary_fade",
];

/// Surfaces the scanner is allowed to consider. Non-fiction is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScanSurface {
    Fiction,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanInput<'a> {
    pub prose: &'a str,
    pub surface: ScanSurface,
}

impl<'a> ScanInput<'a> {
    pub fn fiction(prose: &'a str) -> Self {
        Self {
            prose,
            surface: ScanSurface::Fiction,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AntiSlopHit {
    pub shelf_id: String,
    pub severity: Severity,
    pub rewrite: RewriteMode,
    /// Short rewrite-from-beats direction (not a synonym swap).
    pub rewrite_hint: String,
    pub excerpt: String,
    pub byte_range: TextByteRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AntiSlopReport {
    pub pack_id: String,
    pub pack_version: String,
    pub domain: String,
    pub surface: ScanSurface,
    pub skipped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    /// Hard shelves over their pack limit. Soft never increments this.
    pub hard_count: u32,
    /// Emitted soft / advisory findings.
    pub soft_count: u32,
    pub hits: Vec<AntiSlopHit>,
    pub rewrite_max_passes: u8,
    /// StoryScope-inspired notes. Empty unless experimental structural is on.
    /// Never increment `hard_count`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structural_observations: Vec<StructuralObservation>,
}

/// Journal / console summary: ids and counts only (ADR 0002 D3.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AntiSlopJournalSummary {
    pub hard_count: u32,
    pub soft_count: u32,
    pub hard_ids: Vec<String>,
    pub soft_ids: Vec<String>,
}

/// Compact `anti_slop` payload for the run journal and operator console.
/// Never includes excerpts or rewrite hints.
pub fn journal_summary(report: &AntiSlopReport) -> AntiSlopJournalSummary {
    let mut hard_ids = BTreeSet::new();
    let mut soft_ids = BTreeSet::new();
    for hit in &report.hits {
        match hit.severity {
            Severity::Hard => {
                hard_ids.insert(hit.shelf_id.clone());
            }
            Severity::Soft => {
                soft_ids.insert(hit.shelf_id.clone());
            }
        }
    }
    AntiSlopJournalSummary {
        hard_count: report.hard_count,
        soft_count: report.soft_count,
        hard_ids: hard_ids.into_iter().collect(),
        soft_ids: soft_ids.into_iter().collect(),
    }
}

/// Later persist (not wired in Phase 1): JSON beside the scene receipt.
pub fn persist_path_sketch(project_id: &str, branch_id: &str, scene_id: &str) -> String {
    format!("projects/{project_id}/branches/{branch_id}/scenes/{scene_id}/anti_slop_report.json")
}

#[derive(Clone)]
struct RawMatch {
    start: usize,
    end: usize,
    excerpt: String,
}

/// Scan fiction prose against the loaded pack. Soft hits never raise `hard_count`.
pub fn scan(pack: &ShelfPack, input: &ScanInput<'_>) -> AntiSlopReport {
    scan_with_options(pack, input, ScanOptions::default())
}

/// Scan with an optional project / profile overlay (`disable` / `soften` /
/// `promote_to_hard`). Product locks still apply (`said_bookism` stays soft
/// unless the overlay explicitly promotes it).
pub fn scan_with_overlay(
    pack: &ShelfPack,
    input: &ScanInput<'_>,
    overlay: Option<&GenreOverride>,
) -> AntiSlopReport {
    scan_with_options(
        pack,
        input,
        ScanOptions {
            overlay,
            ..ScanOptions::default()
        },
    )
}

/// Full scan: overlay, rolling chapter counters, suppressions, and the
/// experimental structural flag (off unless set).
pub fn scan_with_options(
    pack: &ShelfPack,
    input: &ScanInput<'_>,
    options: ScanOptions<'_>,
) -> AntiSlopReport {
    let rewrite_max_passes = pack.rewrite_budget();
    if input.surface != ScanSurface::Fiction {
        return AntiSlopReport {
            pack_id: pack.pack_id.clone(),
            pack_version: pack.pack_version.clone(),
            domain: pack.domain.clone(),
            surface: input.surface,
            skipped: true,
            skip_reason: Some("fiction-only: scanner does not run on non-prose surfaces".into()),
            hard_count: 0,
            soft_count: 0,
            hits: Vec::new(),
            rewrite_max_passes,
            structural_observations: Vec::new(),
        };
    }

    let overlay = options.overlay;
    let mut hits = Vec::new();

    for shelf in &pack.shelves {
        if !pack.is_enabled(shelf, overlay) {
            continue;
        }
        let severity = pack.effective_severity(shelf, overlay);
        let raw = collect_matches(shelf, input.prose);
        let prior = if is_chapter_scoped(&shelf.limit_scope) {
            options
                .chapter_prior
                .map(|counters| counters.used(&shelf.id))
                .unwrap_or(0)
        } else {
            0
        };
        let emit = select_emitted(shelf, raw, prior);
        for raw in emit {
            hits.push(AntiSlopHit {
                shelf_id: shelf.id.clone(),
                severity,
                rewrite: RewriteMode::FromBeats,
                rewrite_hint: rewrite_hint(&shelf.id),
                excerpt: raw.excerpt,
                byte_range: TextByteRange {
                    start: raw.start,
                    end: raw.end,
                },
            });
        }
    }

    if let Some(store) = options.suppressions {
        hits.retain(|hit| !store.suppresses_in_prose(hit, input.prose));
    }
    let (hard_count, soft_count) = recount_after_suppress(&hits);

    let structural_observations = if options.experimental_structural {
        experimental_structural_observations(input.prose)
    } else {
        Vec::new()
    };

    AntiSlopReport {
        pack_id: pack.pack_id.clone(),
        pack_version: pack.pack_version.clone(),
        domain: pack.domain.clone(),
        surface: input.surface,
        skipped: false,
        skip_reason: None,
        hard_count,
        soft_count,
        hits,
        rewrite_max_passes,
        structural_observations,
    }
}

fn select_emitted(shelf: &ShelfSpec, matches: Vec<RawMatch>, prior_used: u32) -> Vec<RawMatch> {
    if let Some(allowed) = shelf.default_limit.allowed_count() {
        let remaining = allowed.saturating_sub(prior_used);
        matches.into_iter().skip(remaining as usize).collect()
    } else if shelf.default_limit.is_advisory_cluster() {
        let threshold = if shelf.id == "solitary_fade" { 1 } else { 2 };
        if matches.len() >= threshold {
            matches
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    }
}

/// Raw match count for one shelf (used by rolling chapter counters).
pub(crate) fn raw_match_count(shelf: &ShelfSpec, prose: &str) -> u32 {
    collect_matches(shelf, prose).len() as u32
}

fn collect_matches(shelf: &ShelfSpec, prose: &str) -> Vec<RawMatch> {
    if shelf.id == "said_bookism" {
        return find_all(said_bookism_re(), prose, prose);
    }
    let narration = mask_dialogue_preserving_bytes(prose);
    match shelf.id.as_str() {
        "contrast_not_x_but_y" => find_all(contrast_re(), &narration, prose),
        "emotion_cocktail" => find_all(emotion_re(), &narration, prose),
        "fishing_ending" => find_in_close(fishing_re(), &narration, prose),
        "body_reactions" => find_all(body_re(), &narration, prose),
        "eye_department" => find_all(eye_re(), &narration, prose),
        "gesture_rack" => gesture_cluster_matches(&narration, prose),
        "atmosphere_prefabs" => find_all(atmosphere_re(), &narration, prose),
        "naming_watchlist" => naming_watchlist(&narration, prose),
        "rhythm_cadence" => find_all(rhythm_re(), &narration, prose),
        "triadic_listing" => find_all(triad_re(), &narration, prose),
        "solitary_fade" => find_all(solitary_fade_re(), &narration, prose),
        _ => Vec::new(),
    }
}

/// Replace balanced double-quoted dialogue spans (`"..."` and `“...”`) within
/// each line with ASCII spaces of the exact same byte length.
///
/// Preserves UTF-8 validity and exact byte offsets for all narration outside
/// quotes so regex matches on the returned string slice `prose` without drift.
fn mask_dialogue_preserving_bytes(prose: &str) -> String {
    let mut bytes = prose.as_bytes().to_vec();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        // ASCII double quote '"'
        if bytes[i] == b'"' {
            let mut j = i + 1;
            let mut found = None;
            while j < len && bytes[j] != b'\n' {
                if bytes[j] == b'"' {
                    found = Some(j + 1);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                for b in &mut bytes[i..end] {
                    if *b != b'\n' {
                        *b = b' ';
                    }
                }
                i = end;
                continue;
            }
        }
        // Curly open double quote '“' (0xE2 0x80 0x9C) -> close '”' (0xE2 0x80 0x9D)
        if i + 3 <= len && bytes[i] == 0xE2 && bytes[i + 1] == 0x80 && bytes[i + 2] == 0x9C {
            let mut j = i + 3;
            let mut found = None;
            while j + 3 <= len && bytes[j] != b'\n' {
                if bytes[j] == 0xE2 && bytes[j + 1] == 0x80 && bytes[j + 2] == 0x9D {
                    found = Some(j + 3);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                for b in &mut bytes[i..end] {
                    if *b != b'\n' {
                        *b = b' ';
                    }
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| prose.to_string())
}

/// Snap a byte index down to the nearest UTF-8 char boundary so a close-window
/// slice cannot panic inside a multibyte glyph (em dash, curly quotes).
fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn find_all(re: &Regex, search_text: &str, original_prose: &str) -> Vec<RawMatch> {
    re.find_iter(search_text)
        .filter_map(|mat| {
            let start = mat.start();
            let end = mat.end();
            let slice = original_prose.get(start..end)?;
            let trimmed = slice.trim();
            if trimmed.is_empty() {
                return None;
            }
            let lead = slice.len() - slice.trim_start().len();
            let actual_start = start + lead;
            let actual_end = actual_start + trimmed.len();
            Some(RawMatch {
                start: actual_start,
                end: actual_end,
                excerpt: trimmed.to_string(),
            })
        })
        .collect()
}

fn find_in_close(re: &Regex, search_text: &str, original_prose: &str) -> Vec<RawMatch> {
    let trimmed_len = original_prose.trim_end().len();
    if trimmed_len == 0 {
        return Vec::new();
    }
    let trimmed_orig = &original_prose[..trimmed_len];
    let trimmed_search = &search_text[..trimmed_len.min(search_text.len())];
    let para_start = trimmed_orig.rfind("\n\n").map(|idx| idx + 2).unwrap_or(0);
    let close_start = if trimmed_orig.len() > 400 {
        floor_char_boundary(
            trimmed_orig,
            trimmed_orig.len().saturating_sub(400).max(para_start),
        )
    } else {
        para_start
    };
    let close = &trimmed_search[close_start..];
    re.find_iter(close)
        .filter_map(|mat| {
            let start = close_start + mat.start();
            let end = close_start + mat.end();
            let slice = original_prose.get(start..end)?;
            let trimmed = slice.trim();
            if trimmed.is_empty() {
                return None;
            }
            let lead = slice.len() - slice.trim_start().len();
            let actual_start = start + lead;
            let actual_end = actual_start + trimmed.len();
            Some(RawMatch {
                start: actual_start,
                end: actual_end,
                excerpt: trimmed.to_string(),
            })
        })
        .collect()
}

fn filter_clustered_matches(matches: Vec<RawMatch>, max_gap_bytes: usize) -> Vec<RawMatch> {
    if matches.len() < 2 {
        return Vec::new();
    }
    let mut clustered = Vec::new();
    for (idx, item) in matches.iter().enumerate() {
        let prev_close =
            idx > 0 && item.start.saturating_sub(matches[idx - 1].end) <= max_gap_bytes;
        let next_close = idx + 1 < matches.len()
            && matches[idx + 1].start.saturating_sub(item.end) <= max_gap_bytes;
        if prev_close || next_close {
            clustered.push(item.clone());
        }
    }
    clustered
}

fn gesture_cluster_matches(narration: &str, original_prose: &str) -> Vec<RawMatch> {
    let all = find_all(gesture_re(), narration, original_prose);
    if all.len() >= 3 {
        return all;
    }
    filter_clustered_matches(all, 220)
}

fn naming_watchlist(narration: &str, original_prose: &str) -> Vec<RawMatch> {
    // `delve` is explicitly not a tech gate. Do not add it here.
    let all = find_all(naming_re(), narration, original_prose);
    let clustered = filter_clustered_matches(all, 220);
    // Require at least one multi-word hedge phrase (or >= 3 clustered hedges) so two
    // ordinary adverbs (`suddenly` / `somehow`) do not trigger on their own.
    let has_phrase = clustered
        .iter()
        .any(|m| m.excerpt.split_whitespace().count() >= 2);
    if has_phrase || clustered.len() >= 3 {
        clustered
    } else {
        Vec::new()
    }
}

fn rewrite_hint(shelf_id: &str) -> String {
    match shelf_id {
        "contrast_not_x_but_y" => {
            "Rewrite from the beat: pick the true state and dramatize the action that proves it."
                .into()
        }
        "emotion_cocktail" => {
            "Rewrite from the beat: choose the dominant pressure; give any second feeling an action, not a second noun."
                .into()
        }
        "fishing_ending" => {
            "Rewrite from the beat: end on a choice, cost, new desire, or a fact that was not true at the scene open."
                .into()
        }
        "said_bookism" => {
            "Rewrite from the beat: put the force in the line or a physical beat, then use said (or nothing)."
                .into()
        }
        "solitary_fade" => {
            "Rewrite from the beat: when the POV is alone, root place + body + mundane action (transit, waiting, chores)."
                .into()
        }
        "body_reactions" => {
            "Rewrite from the beat: use this body in this room, or a task that fails because the feeling is in the way."
                .into()
        }
        "eye_department" => {
            "Rewrite from the beat: write what the look does to the next action, not a noun stored in the eyes."
                .into()
        }
        "gesture_rack" => {
            "Rewrite from the beat: give the character a task that can go wrong; skip rotating stock shrugs."
                .into()
        }
        "atmosphere_prefabs" => {
            "Rewrite from the beat: one sensory fact the POV notices because it changes a choice."
                .into()
        }
        "naming_watchlist" => {
            "Rewrite from the beat: cut the hedge and keep the verb that already has consequence."
                .into()
        }
        "rhythm_cadence" => {
            "Rewrite from the beat: vary for pressure, not decoration. Do not insert em-dashes as a fix."
                .into()
        }
        "triadic_listing" => {
            "Rewrite from the beat: keep the one detail that changes the next action."
                .into()
        }
        _ => "Rewrite from the scene beat (at most 1–2 passes). Do not paraphrase-humanize.".into(),
    }
}

const EMOTION_WORDS: &str = "relief|dread|hope|terror|fear|love|resentment|pride|shame|anger|disappointment|grief|guilt|joy|sorrow|panic|calm|rage|pity|resolve|awe|disgust|envy|jealousy|gratitude|bitterness|longing|despair|excitement|anxiety|affection|contempt|fury|remorse|sadness|happiness|elation|melancholy|wonder|horror|revulsion|sympathy|irritation|amusement|confusion|curiosity|unease|apprehension|exhilaration|humiliation|indignation|nostalgia|regret|satisfaction|tenderness|triumph|vulnerability|yearning|frustration|admiration|devotion|hatred|loathing|lust|passion|doubt|helplessness|defiance|resignation|shock|disbelief|astonishment|bewilderment|dismay|foreboding|glee|pleasure|pain|agony|ecstasy|bliss|misery|worry|compassion|empathy|spite|embarrassment|chagrin|vexation|annoyance|delight|euphoria|rapture|serenity|turmoil|anguish|torment|heartbreak|loneliness|suspicion|hopeful|terrified|relieved|afraid|angry|sad|happy|proud|ashamed|guilty|grateful|bitter|anxious|furious|jealous|envious|curious|uneasy|excited|nervous|scared|thrilled|horrified|disgusted|amused|irritated|confused";

const CONTRAST_HINGE_NOUNS: &str = "homecoming|reckoning|request|order|truce|trap|mercy|calculation|threat|promise|warning|apology|confession|verdict|sentence|invitation|ultimatum|command|plea|bargain|surrender|victory|defeat|betrayal|sacrifice|punishment|reward|accident|coincidence|mistake|choice|decision|declaration|judgment|trial|test|lesson|gift|curse|blessing|omen|prophecy|miracle|tragedy|farce|game|war|battle|duel|ambush|execution|inquisition|interrogation|negotiation|conversation|question|answer|lie|truth|secret|revelation|epiphany|awakening|rebirth|ending|beginning|farewell|goodbye|welcome|reunion|escape|rescue|exile|sanctuary|prison|cage|weapon|shield|mask|armor|wound|scar|ghost|haunting|memory|dream|nightmare|illusion|vow|oath|covenant|sin|atonement|redemption|absolution|damnation|salvation|revenge|vengeance|justice|cruelty|kindness|weakness|strength|courage|cowardice|bravery|folly|madness|sanity|survival|destiny|fate";

fn contrast_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let hinge_terms = format!("{EMOTION_WORDS}|{CONTRAST_HINGE_NOUNS}");
        let pat = format!(
            r"(?i)(?:\b(?:it|this|that)\s+(?:wasn['’]t|was\s+not)\s+(?:(?:just|merely|simply)\s+(?:a\s+|an\s+|the\s+)?[a-z'’-]+(?:\s+[a-z'’-]+){{0,3}}\s*(?:[\.!?]\s*(?:it\s+was|this\s+was|that\s+was|it['’]s)\s+(?:a\s+|an\s+|the\s+)?[a-z'’-]+|[\u2014—]\s*(?:(?:it\s+was|this\s+was|that\s+was)\s+)?(?:a\s+|an\s+|the\s+)?[a-z'’-]+)|(?:a\s+|an\s+|the\s+)?(?:(?:{h})\b[a-z'’\s-]{{0,24}}(?:[\.!?]\s*(?:it\s+was|this\s+was|that\s+was|it['’]s)\s+(?:a\s+|an\s+|the\s+)?[a-z'’-]+|[\u2014—]\s*(?:(?:it\s+was|this\s+was|that\s+was)\s+)?(?:a\s+|an\s+|the\s+)?[a-z'’-]+)|[a-z'’-]+(?:\s+[a-z'’-]+){{0,3}}\s*(?:[\.!?]\s*(?:it\s+was|this\s+was|that\s+was|it['’]s)\s+(?:a\s+|an\s+|the\s+)?(?:[a-z'’-]+\s+){{0,2}}(?:{h})|[\u2014—]\s*(?:(?:it\s+was|this\s+was|that\s+was)\s+)?(?:a\s+|an\s+|the\s+)?(?:[a-z'’-]+\s+){{0,2}}(?:{h}))))\b|(?:^|[\.!?\n]\s*)Not\s+(?:just\s+|merely\s+|simply\s+)?(?:a\s+|an\s+|the\s+)?(?:{h})\s*(?:[\u2014—–-]\s*(?:something\b|a\b|an\b|the\b|{h})|,\s*but\s+(?:something\b|rather\b|a\b|an\b|the\b|{h}))\b|\b(?:he|she|they|it)\s+didn['’]t\s+\w+[^\n]{{0,60}}\bbecause\b[^\n]{{1,60}}\bbecause\b)",
            h = hinge_terms
        );
        Regex::new(&pat).expect("contrast regex")
    })
}

fn emotion_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let pat = format!(
            r"(?i)\b(?:(?:a\s+)?(?:mix|mixture|blend|concoction|tangle|wash|rush|flood|wave|surge)\s+of\s+(?:both\s+)?(?:{e})\s+and\s+(?:{e})|equal\s+parts\s+(?:{e})\s+and\s+(?:{e})|(?:a\s+)?(?:confusing\s+|strange\s+|heady\s+|sudden\s+|dizzying\s+)?swirl\s+of\s+(?:conflicting\s+emotions?|(?:{e})\s+and\s+(?:{e}))|torn\s+between\s+(?:{e})\s+and\s+(?:{e})|conflicting\s+emotions|(?:{e})\s+and\s+(?:{e})\s+warred\s+(?:within|inside)\s+(?:him|her|them|me))\b",
            e = EMOTION_WORDS
        );
        Regex::new(&pat).expect("emotion regex")
    })
}

fn fishing_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:only\s+time\s+(?:would|will)\s+tell|what\s+tomorrow\s+would\s+bring|the\s+real\s+journey\s+(?:was\s+just\s+beginning|had\s+only\s+just\s+begun|lay\s+ahead)|somehow,?\s+(?:she|he|they)\s+knew\s+(?:that\s+)?(?:everything\s+had\s+changed|nothing\s+would\s+ever\s+be\s+the\s+same|this\s+was\s+(?:only|just)\s+the\s+beginning|there\s+was\s+no\s+going\s+back)|what\s+happened\s+next\s+would\s+change|nothing\s+would\s+ever\s+be\s+the\s+same)\b",
        )
        .expect("fishing regex")
    })
}

fn said_bookism_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)["“][^"”\n]{1,240}[,?!—…]["”]\s+(?:she|he|they|(?:the|a|an)\s+[a-z][\w'-]*|(?-i:[A-Z][\w'-]*))\s+(?:ejaculated|expostulated|intoned|queried|interjected|hissed|growled|snapped|breathed|gasped)\b"#,
        )
        .expect("said_bookism regex")
    })
}

fn rhythm_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)(?:^(?:He|She|They) \w+\.\s*){3,}").expect("rhythm regex"))
}

fn triad_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:\bthe \w+ of \w+,\s+the \w+ of \w+,\s+(?:and\s+)?the \w+ of \w+\b|\bblood,\s+ash,\s+and\s+\w+\b)",
        )
        .expect("triad regex")
    })
}

fn solitary_fade_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?im)(?:\b(?:(?:a\s+few|several|many|two|three|four|five|six|seven|eight|nine|ten)\s+)?(?:hours|days|weeks|months)\s+passed(?:\s*[\.!?,;—\u2014…]|\s*$)|\bspent the (?:afternoon|morning|evening|night|day)\s+thinking(?:\s+about)?\b|\bLater,\s+at the\s+(?:meeting|office|table|hall),\s+(?:she|he|they)\s+told\s+(?:him|her|them)\s+everything\b|\bthought about (?:what|the offer|his words|her words)[^\n]{0,80}\buntil (?:morning|dawn)\b|\bleft the \w+ and later told \w+\b)",
        )
        .expect("solitary_fade regex")
    })
}

fn body_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:jaw\s+tightened|stomach\s+dropped|heart\s+hammered|blood\s+ran\s+cold|knuckles\s+whitened|shivers\s+down(?:\s+(?:his|her|their|my)\s+spine)?|lump\s+in\s+(?:his|her|their)\s+throat|didn['’]t\s+know\s+(?:she|he|they)\s+(?:was|were)\s+holding)\b",
        )
        .expect("body_reactions regex")
    })
}

fn eye_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let pat = format!(
            r"(?i)\b(?:eyes\s+that\s+held|gaze\s+pierced|eyes\s+darkened|looked\s+into\s+(?:his|her|their)\s+eyes(?:\s+and\s+saw)?|flickered\s+behind\s+(?:his|her|their)\s+eyes|eyes\s+held\s+(?:a\b|an\b|only\b|nothing\b|{e}))\b",
            e = EMOTION_WORDS
        );
        Regex::new(&pat).expect("eye_department regex")
    })
}

fn gesture_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:ran\s+a\s+hand\s+through(?:\s+(?:his|her|their)\s+hair)?|crossed\s+(?:his|her|their)\s+arms|clenched\s+(?:his|her|their)\s+fists|shrugged|sighed)\b",
        )
        .expect("gesture_rack regex")
    })
}

fn atmosphere_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:air\s+crackled|silence\s+hung\s+heavy|(?:room|air|world|night|house|hall|crowd)\s+(?:seemed\s+to\s+)?(?:hold|held)\s+its\s+breath|hold\s+its\s+breath|something\s+in\s+the\s+air\s+shifted|a\s+testament\s+to|a\s+dance\s+of\s+(?:light|shadow|shadows|death|fire|flame|steel|blades))\b",
        )
        .expect("atmosphere_prefabs regex")
    })
}

fn naming_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let pat = format!(
            r"(?i)\b(?:couldn['’]t\s+help\s+but|found\s+(?:himself|herself|themselves)\b|in\s+that\s+moment|something\s+shifted|it\s+was\s+as\s+if|with\s+a\s+sense\s+of\s+(?:{e}|finality|purpose|inevitability)|the\s+weight\s+of\s+(?:it\s+all|everything|nothing|the\s+moment|the\s+silence|the\s+years|the\s+past|the\s+truth|his\s+words|her\s+words|their\s+words|his\s+gaze|her\s+gaze|{e})|suddenly|somehow)\b",
            e = EMOTION_WORDS
        );
        Regex::new(&pat).expect("naming_watchlist regex")
    })
}
