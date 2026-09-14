//! StoryScope-inspired structural observations (experimental).
//!
//! These are editorial notes, not shelves, not scores, and not fail gates.
//! They must never increment `hard_count` or enter `verify_findings`.
//! The feature is off unless the caller sets `experimental_structural`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Default for `[anti_slop] experimental_structural`. Product lock: off.
pub fn experimental_structural_default() -> bool {
    false
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StructuralObservation {
    /// Stable question id (not a shelf id).
    pub question_id: String,
    pub note: String,
}

/// Lightweight heuristics for the five catalog questions. Genre may "fail"
/// these on purpose; they are revision prompts only.
pub fn experimental_structural_observations(prose: &str) -> Vec<StructuralObservation> {
    let lower = prose.to_lowercase();
    let close = close_window(&lower);
    let mut out = Vec::new();

    if contains_any(
        close,
        &[
            "the moral",
            "the lesson",
            "and that was the point",
            "the truth was that",
            "what it all meant",
        ],
    ) {
        out.push(StructuralObservation {
            question_id: "theme_stated".into(),
            note: "Narrator may state the theme outright in the close. Editorial question only — not a shelf or fail gate.".into(),
        });
    }

    if contains_any(
        &lower,
        &[
            "an old novel",
            "a song she had loved",
            "a song he had loved",
            "that place from before",
            "someone once said",
        ],
    ) {
        out.push(StructuralObservation {
            question_id: "gestured_reference".into(),
            note: "A reference may be gestured at rather than named. Editorial question only."
                .into(),
        });
    }

    if contains_any(
        close,
        &[
            "everything would be all right",
            "and that was that",
            "the end of the matter",
        ],
    ) {
        out.push(StructuralObservation {
            question_id: "tidy_close".into(),
            note: "Close may tidy every residue. Ask whether a cost or lie still stands. Not a fishing_ending shelf.".into(),
        });
    }

    out
}

fn close_window(prose: &str) -> &str {
    let trimmed = prose.trim_end();
    if trimmed.len() > 400 {
        let start = trimmed.len().saturating_sub(400);
        let start = trimmed
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|idx| *idx <= start)
            .last()
            .unwrap_or(0);
        &trimmed[start..]
    } else {
        trimmed
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}
