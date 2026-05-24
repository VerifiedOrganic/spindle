//! Per-segment entity candidate extractor.
//!
//! Pure heuristic extractor — operates on a single segment of normalized
//! manuscript text and returns mention candidates the service layer then
//! routes through `Repository::create_import_entity_mention`. Ported from
//! the SurrealDB-era `crate::import::extract` with no functional changes;
//! no repository or record-id types are involved here.

use std::collections::{BTreeMap, BTreeSet};

use spindle_core::models::ImportEntityKind;

#[derive(Debug, Clone)]
pub struct ExtractedMentionCandidate {
    pub entity_kind: ImportEntityKind,
    pub surface_form: String,
    pub normalized_name: String,
    pub alias_hint: Option<String>,
    pub surrounding_text: Option<String>,
    pub confidence: f64,
    pub review_reason: Option<String>,
}

pub fn extract_entity_candidates(segment_text: &str) -> Vec<ExtractedMentionCandidate> {
    let mut candidates = Vec::new();
    candidates.extend(extract_character_candidates(segment_text));
    candidates.extend(extract_location_candidates(segment_text));
    candidates.extend(extract_event_candidates(segment_text));
    candidates.sort_by(|left, right| {
        left.normalized_name
            .cmp(&right.normalized_name)
            .then_with(|| left.surface_form.cmp(&right.surface_form))
            .then_with(|| left.entity_kind_string().cmp(right.entity_kind_string()))
    });
    candidates.dedup_by(|left, right| {
        left.entity_kind_string() == right.entity_kind_string()
            && left.normalized_name == right.normalized_name
            && left.surface_form == right.surface_form
    });
    candidates
}

impl ExtractedMentionCandidate {
    fn entity_kind_string(&self) -> &'static str {
        match self.entity_kind {
            ImportEntityKind::Character => "character",
            ImportEntityKind::Location => "location",
            ImportEntityKind::Conflict => "conflict",
            ImportEntityKind::NarrativePromise => "narrative_promise",
            ImportEntityKind::Theme => "theme",
            ImportEntityKind::Motif => "motif",
            ImportEntityKind::CharacterArc => "character_arc",
            ImportEntityKind::Knowledge => "knowledge",
            ImportEntityKind::Faction => "faction",
            ImportEntityKind::Religion => "religion",
            ImportEntityKind::Economy => "economy",
            ImportEntityKind::Term => "term",
            ImportEntityKind::WorldRule => "world_rule",
            ImportEntityKind::PlotLine => "plot_line",
            ImportEntityKind::Other => "other",
        }
    }
}

fn extract_character_candidates(segment_text: &str) -> Vec<ExtractedMentionCandidate> {
    let mut counts = BTreeMap::<String, usize>::new();
    let mut aliases = BTreeMap::<String, String>::new();
    let tokens = segment_text
        .split(|ch: char| !ch.is_ascii_alphabetic() && ch != '\'')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    for window in tokens.windows(2) {
        if let [title, name] = window
            && is_honorific(title)
            && looks_like_name(name)
        {
            aliases.insert(normalize_name(name), format!("{} {}", title, name));
        }
    }

    for token in tokens {
        if !looks_like_name(token) || is_common_capitalized_word(token) {
            continue;
        }
        *counts.entry(token.to_string()).or_insert(0) += 1;
    }

    counts
        .into_iter()
        .map(|(surface_form, count)| {
            let normalized_name = normalize_name(&surface_form);
            let confidence = if count >= 3 { 0.86 } else { 0.63 };
            ExtractedMentionCandidate {
                entity_kind: ImportEntityKind::Character,
                surface_form: surface_form.clone(),
                normalized_name: normalized_name.clone(),
                alias_hint: aliases.get(&normalized_name).cloned(),
                surrounding_text: excerpt_around(segment_text, &surface_form),
                confidence,
                review_reason: (count == 1).then(|| {
                    format!(
                        "character mention '{}' only appeared once in this segment",
                        surface_form
                    )
                }),
            }
        })
        .collect()
}

fn extract_location_candidates(segment_text: &str) -> Vec<ExtractedMentionCandidate> {
    let location_suffixes = [
        "Hall", "Keep", "Tower", "Gate", "Bridge", "Court", "Harbor", "Archive", "Forest",
        "Temple", "Road", "City", "Village", "Inn", "Palace",
    ];
    let mut seen = BTreeSet::new();
    let words = segment_text.split_whitespace().collect::<Vec<_>>();
    let mut candidates = Vec::new();

    for window in words.windows(2) {
        if let [left, right] = window {
            let left = clean_token(left);
            let right = clean_token(right);
            if looks_like_name(left)
                && location_suffixes.contains(&right)
                && seen.insert(format!("{} {}", left, right))
            {
                candidates.push(ExtractedMentionCandidate {
                    entity_kind: ImportEntityKind::Location,
                    surface_form: format!("{} {}", left, right),
                    normalized_name: normalize_name(&format!("{} {}", left, right)),
                    alias_hint: None,
                    surrounding_text: excerpt_around(segment_text, &format!("{} {}", left, right)),
                    confidence: 0.82,
                    review_reason: None,
                });
            }
        }
    }

    candidates
}

fn extract_event_candidates(segment_text: &str) -> Vec<ExtractedMentionCandidate> {
    let event_markers = [
        "battle",
        "attack",
        "warning",
        "betrayal",
        "revelation",
        "promise",
        "oath",
        "secret",
        "argument",
        "fire",
        "murder",
        "escape",
    ];
    let lowered = segment_text.to_ascii_lowercase();
    event_markers
        .iter()
        .filter(|marker| lowered.contains(**marker))
        .map(|marker| ExtractedMentionCandidate {
            entity_kind: ImportEntityKind::Conflict,
            surface_form: marker.to_string(),
            normalized_name: marker.to_string(),
            alias_hint: None,
            surrounding_text: excerpt_around_case_insensitive(segment_text, marker),
            confidence: 0.58,
            review_reason: Some(format!(
                "event-like mention '{}' was inferred from keyword context",
                marker
            )),
        })
        .collect()
}

fn excerpt_around(segment_text: &str, needle: &str) -> Option<String> {
    let start = segment_text.find(needle)?;
    let excerpt_start = start.saturating_sub(36);
    let excerpt_end = (start + needle.len() + 48).min(segment_text.len());
    segment_text
        .get(excerpt_start..excerpt_end)
        .map(str::trim)
        .map(ToString::to_string)
}

fn excerpt_around_case_insensitive(segment_text: &str, needle: &str) -> Option<String> {
    let lowered = segment_text.to_ascii_lowercase();
    let start = lowered.find(needle)?;
    let excerpt_start = start.saturating_sub(36);
    let excerpt_end = (start + needle.len() + 48).min(segment_text.len());
    segment_text
        .get(excerpt_start..excerpt_end)
        .map(str::trim)
        .map(ToString::to_string)
}

fn clean_token(token: &str) -> &str {
    token.trim_matches(|ch: char| !ch.is_ascii_alphabetic() && ch != '\'')
}

fn is_honorific(token: &str) -> bool {
    matches!(
        clean_token(token),
        "Lord" | "Lady" | "Captain" | "Prince" | "Princess"
    )
}

fn looks_like_name(token: &str) -> bool {
    let token = clean_token(token);
    token.len() >= 2
        && token
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
        && token
            .chars()
            .skip(1)
            .all(|ch| ch.is_ascii_lowercase() || ch == '\'')
}

fn is_common_capitalized_word(token: &str) -> bool {
    matches!(
        token,
        "The"
            | "A"
            | "An"
            | "And"
            | "But"
            | "If"
            | "When"
            | "Then"
            | "Chapter"
            | "Prologue"
            | "Epilogue"
    )
}

fn normalize_name(token: &str) -> String {
    clean_token(token).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_character_location_and_event_candidates() {
        let candidates = extract_entity_candidates(
            "Lady Mara crossed the Silver Court while Kade shouted a warning about the fire.",
        );

        assert!(candidates.iter().any(|candidate| {
            matches!(candidate.entity_kind, ImportEntityKind::Character)
                && candidate.surface_form == "Mara"
                && candidate.alias_hint.as_deref() == Some("Lady Mara")
        }));
        assert!(candidates.iter().any(|candidate| {
            matches!(candidate.entity_kind, ImportEntityKind::Location)
                && candidate.surface_form == "Silver Court"
        }));
        assert!(candidates.iter().any(|candidate| {
            matches!(candidate.entity_kind, ImportEntityKind::Conflict)
                && candidate.surface_form == "warning"
        }));
    }
}
