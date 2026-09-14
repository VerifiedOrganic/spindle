use super::pack::{RewriteMode, Severity, ShelfPack, ShelfSpec};
use crate::models::TextByteRange;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

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
        };
    }

    let mut hits = Vec::new();
    let mut hard_count = 0_u32;
    let mut soft_count = 0_u32;

    for shelf in &pack.shelves {
        if !pack.is_enabled(shelf, None) {
            continue;
        }
        let severity = pack.effective_severity(shelf, None);
        let raw = collect_matches(shelf, input.prose);
        let emit = select_emitted(shelf, raw);
        for raw in emit {
            let hit = AntiSlopHit {
                shelf_id: shelf.id.clone(),
                severity,
                rewrite: RewriteMode::FromBeats,
                rewrite_hint: rewrite_hint(&shelf.id),
                excerpt: raw.excerpt,
                byte_range: TextByteRange {
                    start: raw.start,
                    end: raw.end,
                },
            };
            match severity {
                Severity::Hard => hard_count += 1,
                Severity::Soft => soft_count += 1,
            }
            hits.push(hit);
        }
    }

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
    }
}

fn select_emitted(shelf: &ShelfSpec, matches: Vec<RawMatch>) -> Vec<RawMatch> {
    if let Some(allowed) = shelf.default_limit.allowed_count() {
        matches.into_iter().skip(allowed as usize).collect()
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

fn collect_matches(shelf: &ShelfSpec, prose: &str) -> Vec<RawMatch> {
    match shelf.id.as_str() {
        "contrast_not_x_but_y" => find_all(contrast_re(), prose),
        "emotion_cocktail" => find_all(emotion_re(), prose),
        "fishing_ending" => find_in_close(fishing_re(), prose),
        "said_bookism" => find_all(said_bookism_re(), prose),
        "body_reactions" => distinct_markers(prose, BODY_MARKERS),
        "eye_department" => distinct_markers(prose, EYE_MARKERS),
        "gesture_rack" => distinct_markers(prose, GESTURE_MARKERS),
        "atmosphere_prefabs" => distinct_markers(prose, ATMOSPHERE_MARKERS),
        "naming_watchlist" => naming_watchlist(prose),
        "rhythm_cadence" => find_all(rhythm_re(), prose),
        "triadic_listing" => find_all(triad_re(), prose),
        "solitary_fade" => find_all(solitary_fade_re(), prose),
        _ => Vec::new(),
    }
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

fn find_all(re: &Regex, prose: &str) -> Vec<RawMatch> {
    re.find_iter(prose)
        .map(|mat| RawMatch {
            start: mat.start(),
            end: mat.end(),
            excerpt: mat.as_str().trim().to_string(),
        })
        .collect()
}

fn find_in_close(re: &Regex, prose: &str) -> Vec<RawMatch> {
    let trimmed = prose.trim_end();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let para_start = trimmed.rfind("\n\n").map(|idx| idx + 2).unwrap_or(0);
    let close_start = if trimmed.len() > 400 {
        floor_char_boundary(trimmed, trimmed.len().saturating_sub(400).max(para_start))
    } else {
        para_start
    };
    let close = &trimmed[close_start..];
    re.find_iter(close)
        .map(|mat| RawMatch {
            start: close_start + mat.start(),
            end: close_start + mat.end(),
            excerpt: mat.as_str().trim().to_string(),
        })
        .collect()
}

fn distinct_markers(prose: &str, markers: &[&str]) -> Vec<RawMatch> {
    let lower = prose.to_lowercase();
    let mut out = Vec::new();
    for marker in markers {
        if let Some(rel) = lower.find(marker) {
            let end = rel + marker.len();
            if end <= prose.len() {
                out.push(RawMatch {
                    start: rel,
                    end,
                    excerpt: prose[rel..end].to_string(),
                });
            }
        }
    }
    out
}

fn naming_watchlist(prose: &str) -> Vec<RawMatch> {
    // `delve` is explicitly not a tech gate. Do not add it here.
    distinct_markers(prose, NAMING_MARKERS)
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

fn contrast_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:\b(?:it|this|that)\s+wasn['’]t\b[^\n.]{0,80}[\.!?]\s*(?:it\s+was|it['’]s)\b|\bnot\s+[^.\n]{1,40}(?:,\s*but\s+|[\u2014—–-]\s+)|\bdidn['’]t\b[^\n]{0,80}\bbecause\b[^\n]{0,80}\bbecause\b)",
        )
        .expect("contrast regex")
    })
}

fn emotion_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:a\s+mix\s+of\s+\w+\s+and\s+\w+|equal\s+parts\b|swirl\s+of\s+(?:conflicting\s+)?(?:emotions?|\w+\s+and\s+\w+)|torn\s+between\b|conflicting\s+emotions\b)",
        )
        .expect("emotion regex")
    })
}

fn fishing_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:only\s+time\s+would\s+tell|tomorrow\s+would\s+bring|real\s+journey\b|somehow,?\s+(?:she|he|they)\s+knew|what\s+happened\s+next\s+would\s+change|nothing\s+would\s+ever\s+be\s+the\s+same)\b",
        )
        .expect("fishing regex")
    })
}

fn said_bookism_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)["“][^"”\n]{0,240}["”]\s+(?:she|he|they|[A-Z][\w'-]*)\s+(?:ejaculated|expostulated|intoned|queried|interjected|hissed|growled|snapped|breathed|gasped)\b"#,
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
            r"(?i)\b(?:(?:hours|days|weeks|months)\s+passed|spent the (?:afternoon|morning|evening|night|day)\s+thinking|Later,\s+at the\b|thought about (?:what|the offer)[^\n]{0,80}until (?:morning|dawn)|later told \w+)\b",
        )
        .expect("solitary_fade regex")
    })
}

const BODY_MARKERS: &[&str] = &[
    "jaw tightened",
    "stomach dropped",
    "heart hammered",
    "blood ran cold",
    "knuckles whitened",
    "shivers down",
    "lump in his throat",
    "lump in her throat",
    "didn't know she was holding",
    "didn't know they were holding",
];

const EYE_MARKERS: &[&str] = &[
    "eyes that held",
    "gaze pierced",
    "eyes darkened",
    "looked into his eyes",
    "looked into her eyes",
    "flickered behind",
    "eyes held",
];

const GESTURE_MARKERS: &[&str] = &[
    "ran a hand through",
    "crossed his arms",
    "crossed her arms",
    "clenched his fists",
    "clenched her fists",
    "shrugged",
    "sighed",
];

const ATMOSPHERE_MARKERS: &[&str] = &[
    "air crackled",
    "silence hung heavy",
    "hold its breath",
    "something in the air shifted",
    "a testament to",
    "a dance of",
];

const NAMING_MARKERS: &[&str] = &[
    "suddenly",
    "somehow",
    "couldn't help but",
    "found themselves",
    "in that moment",
    "something shifted",
    "it was as if",
    "with a sense of",
    "the weight of",
];
