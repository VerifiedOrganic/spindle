//! Deterministic style-drift heuristics.
//!
//! These checks are intentionally *coarse and high-precision*: they flag the
//! clearest genre-voice mismatches (a grief/quiet tone string on a comedy
//! project; a contemplative chapter ending where the genre wants a hook) and
//! stay quiet otherwise. The reliable, nuanced judgement ("is this scene
//! actually funny?") is semantic and lives in the dual-persona review's Target
//! Reader persona — not here. False positives erode trust in the gate, so when
//! in doubt this scanner emits `Info`, or nothing.

use super::StyleDirective;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleDriftSeverity {
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleDriftHit {
    /// Stable identifier for the kind of drift, for grouping/telemetry.
    pub kind: &'static str,
    pub severity: StyleDriftSeverity,
    pub message: String,
}

/// Inputs for a single draft scan.
#[derive(Debug, Clone, Default)]
pub struct StyleScanInput<'a> {
    pub prose: &'a str,
    /// The author-declared tone string passed to `save_scene_draft`, if any.
    pub declared_tone: Option<&'a str>,
    /// Whether this scene closes its chapter (enables ending-beat checks).
    pub is_chapter_end: bool,
}

/// Tone descriptors that signal quiet/contemplative/literary register. Used
/// both for declared-tone checks and prose-marker density.
const CONTEMPLATIVE_MARKERS: &[&str] = &[
    "grief",
    "grieving",
    "mournful",
    "mourning",
    "elegiac",
    "elegy",
    "sorrow",
    "melancholy",
    "melancholic",
    "wistful",
    "contemplat",
    "meditation on",
    "meditative",
    "quiet ache",
    "hollow ache",
    "the weight of",
    "lament",
    "keening",
    "subdued",
    "muted",
    "somber",
    "sombre",
];

/// Tone descriptors used specifically for the *declared tone string* check —
/// short single words that, when a comedy/fast genre is declared, indicate the
/// author themselves framed the scene as quiet/literary.
const DECLARED_TONE_RED_FLAGS: &[&str] = &[
    "grief",
    "quiet",
    "contained",
    "contemplat",
    "elegiac",
    "mournful",
    "somber",
    "sombre",
    "melancholy",
    "wistful",
    "literary",
    "muted",
    "subdued",
    "reflective",
    "introspective",
    "meditative",
    "lyrical",
];

pub(super) fn scan(directive: &StyleDirective, input: &StyleScanInput) -> Vec<StyleDriftHit> {
    let mut hits = Vec::new();
    if directive.is_empty() {
        return hits;
    }
    let intent = directive.intent();

    // Literary projects legitimately use contemplative prose — skip the
    // literary-marker heuristics for them entirely.
    let check_against_literary =
        (intent.wants_comedy || intent.wants_fast_pacing) && !intent.is_literary;

    // 1. Declared tone string conflicts with the genre. Highest-confidence
    //    check: the author themselves labeled the scene quiet/literary on a
    //    comedy/fast project. This is the exact failure from the brief.
    if check_against_literary && let Some(tone) = input.declared_tone {
        let tone_lc = tone.to_lowercase();
        let flagged: Vec<&str> = DECLARED_TONE_RED_FLAGS
            .iter()
            .copied()
            .filter(|flag| tone_lc.contains(flag))
            .collect();
        if !flagged.is_empty() {
            hits.push(StyleDriftHit {
                kind: "declared_tone_conflict",
                severity: StyleDriftSeverity::Warning,
                message: format!(
                    "Declared tone '{}' conflicts with the project's {} style contract \
                     (matched: {}). A comedy/fast-paced project should not lean on a \
                     quiet/contemplative/grief register.",
                    tone.trim(),
                    genre_word(&intent),
                    flagged.join(", "),
                ),
            });
        }
    }

    // 2. Contemplative chapter ending where the genre wants a hook.
    if intent.wants_hook_endings
        && input.is_chapter_end
        && let Some(message) = check_contemplative_ending(input.prose)
    {
        hits.push(StyleDriftHit {
            kind: "contemplative_ending",
            severity: StyleDriftSeverity::Warning,
            message,
        });
    }

    // 3. High density of literary/grief markers in the prose body on a
    //    comedy/fast project. Conservative threshold; emitted as Info because
    //    these words can legitimately appear in passing.
    if check_against_literary {
        let distinct = count_distinct_markers(input.prose, CONTEMPLATIVE_MARKERS);
        if distinct >= 4 {
            hits.push(StyleDriftHit {
                kind: "literary_marker_density",
                severity: StyleDriftSeverity::Info,
                message: format!(
                    "Prose carries {distinct} distinct literary/grief tone markers but the style \
                     contract declares a {} register; the scene may be reading \
                     contemplative-literary rather than on-genre.",
                    genre_word(&intent),
                ),
            });
        }
    }

    hits
}

fn genre_word(intent: &super::StyleIntent) -> &'static str {
    if intent.wants_comedy && intent.wants_fast_pacing {
        "comedy / fast-paced"
    } else if intent.wants_comedy {
        "comedy"
    } else if intent.wants_fast_pacing {
        "fast-paced"
    } else {
        "declared"
    }
}

/// Inspect the final paragraph of a scene. Flag when it both reads
/// contemplative AND lacks any hook signal (a trailing question, exclamation,
/// dangling dash, or a line of dialogue).
fn check_contemplative_ending(prose: &str) -> Option<String> {
    let trimmed = prose.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    // Last non-empty paragraph.
    let last_paragraph = trimmed
        .rsplit("\n\n")
        .map(str::trim)
        .find(|para| !para.is_empty())
        .unwrap_or(trimmed);
    let last_paragraph_lc = last_paragraph.to_lowercase();

    let has_contemplative = CONTEMPLATIVE_MARKERS
        .iter()
        .any(|marker| last_paragraph_lc.contains(marker));
    if !has_contemplative {
        return None;
    }

    let has_hook_signal = {
        let last_char = trimmed.chars().next_back();
        matches!(last_char, Some('?') | Some('!') | Some('—') | Some('-'))
            // A closing line of dialogue is a reasonable hook.
            || trimmed.ends_with('"')
            || trimmed.ends_with('\u{201d}')
    };
    if has_hook_signal {
        return None;
    }

    Some(
        "Chapter-ending scene closes on a quiet/contemplative beat with no hook; the style \
         contract asks chapters to end on a hook or cliffhanger."
            .to_string(),
    )
}

/// Count how many *distinct* markers from `markers` appear anywhere in `prose`.
fn count_distinct_markers(prose: &str, markers: &[&str]) -> usize {
    let prose_lc = prose.to_lowercase();
    markers
        .iter()
        .filter(|marker| prose_lc.contains(*marker))
        .count()
}

#[cfg(test)]
mod tests {
    use super::super::{NarratorVoice, StyleDirective, StyleRule};
    use super::*;

    fn comedy_directive() -> StyleDirective {
        StyleDirective::assemble(
            "Comedy",
            "NSFW Comedy Webnovel",
            "A raunchy funny gacha romp",
            vec!["Raunchy modern comedy tone".to_string()],
            Vec::new(),
            vec![StyleRule {
                rule_name: "Prose Style Bible".to_string(),
                description: "No grief beats; no contemplative literary pacing.".to_string(),
            }],
            Some(NarratorVoice {
                chapter_ending_style: Some("hook".to_string()),
                ..Default::default()
            }),
        )
    }

    #[test]
    fn flags_grief_tone_on_comedy_project() {
        let directive = comedy_directive();
        let hits = directive.scan(&StyleScanInput {
            prose: "Some prose.",
            declared_tone: Some("Quiet, contained, grief beat"),
            is_chapter_end: false,
        });
        assert!(hits.iter().any(|hit| hit.kind == "declared_tone_conflict"
            && hit.severity == StyleDriftSeverity::Warning));
    }

    #[test]
    fn does_not_flag_comedic_tone() {
        let directive = comedy_directive();
        let hits = directive.scan(&StyleScanInput {
            prose: "He cracked a joke and everyone laughed.",
            declared_tone: Some("manic, raunchy, fast"),
            is_chapter_end: true,
        });
        assert!(hits.is_empty(), "unexpected hits: {hits:?}");
    }

    #[test]
    fn flags_contemplative_no_hook_ending() {
        let directive = comedy_directive();
        let prose = "They joked all day.\n\nThat night he sat alone with the hollow ache of \
                     everything he had lost, and let the quiet settle over him like snow.";
        let hits = directive.scan(&StyleScanInput {
            prose,
            declared_tone: None,
            is_chapter_end: true,
        });
        assert!(hits.iter().any(|hit| hit.kind == "contemplative_ending"));
    }

    #[test]
    fn contemplative_ending_with_hook_punctuation_is_allowed() {
        let directive = comedy_directive();
        let prose = "That night he sat with the hollow ache of his losses. Then the System \
                     pinged: \"New quest available — want to get laid or what?\"";
        let hits = directive.scan(&StyleScanInput {
            prose,
            declared_tone: None,
            is_chapter_end: true,
        });
        assert!(
            !hits.iter().any(|hit| hit.kind == "contemplative_ending"),
            "hook-ending should not be flagged: {hits:?}"
        );
    }

    #[test]
    fn literary_project_prose_is_not_flagged() {
        let directive = StyleDirective::assemble(
            "Literary Fiction",
            "Novel",
            "A meditation on grief",
            vec!["Lyrical contemplative literary prose".to_string()],
            Vec::new(),
            Vec::new(),
            None,
        );
        let prose = "She sat with her grief, the hollow ache, the quiet sorrow, the melancholy \
                     of mourning settling like dusk.";
        let hits = directive.scan(&StyleScanInput {
            prose,
            declared_tone: Some("quiet, elegiac, contemplative"),
            is_chapter_end: true,
        });
        assert!(
            hits.is_empty(),
            "literary project should be exempt: {hits:?}"
        );
    }

    #[test]
    fn flags_dense_literary_markers_as_info() {
        let directive = comedy_directive();
        let prose = "The grief was a hollow ache. A wistful sorrow, a quiet melancholy, the \
                     weight of mourning pressing down.";
        let hits = directive.scan(&StyleScanInput {
            prose,
            declared_tone: None,
            is_chapter_end: false,
        });
        assert!(
            hits.iter().any(|hit| hit.kind == "literary_marker_density"
                && hit.severity == StyleDriftSeverity::Info)
        );
    }
}
