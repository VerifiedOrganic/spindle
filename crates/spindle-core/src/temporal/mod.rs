//! Intra-scene temporal-coherence analysis.
//!
//! A pure, deterministic, infrastructure-free prose scan that detects
//! time-of-day jumps *inside a single scene* that the prose never signals — the
//! within-scene, forward-looking complement to the between-scene `chronology`
//! check (which only catches a *later* scene rewinding the clock).
//!
//! It answers the two failure modes an author hits when the manuscript
//! teleports through the day with no bridge:
//! - **teleporting time** — a large forward time-of-day skip (e.g. morning →
//!   night) with no transition beat or scene break, and
//! - **drifting time** — the prose contradicting its own established
//!   time-of-day (e.g. establishing night, then referring to the same scene's
//!   morning).
//!
//! It also uses the declared scene span: a scene that declares
//! `duration_days >= 1` is asserting it spans real in-world time, so the scan
//! expects the prose to *render* that span with at least one transition marker
//! or scene break; a declared multi-day scene written as one unbroken block is
//! an **unrendered declared span**.
//!
//! Design posture (matching the existing `style`/`world_rules` scanners): a
//! **high-precision, low-recall advisory tripwire**, never a hard gate. It
//! favours silence over noise — word-boundary matching, conservative band-jump
//! thresholds, and explicit suppression for every legitimate construct
//! (signalled ellipsis, scene break, declared flashback, in-scene recollection,
//! coarse clock precision).

use crate::models::TextByteRange;

/// Per-scene temporal metadata the coherence scan consumes.
///
/// The raw `prose` is the primary input; the remaining fields are the scene's
/// declared clock metadata, used only to *suppress* findings (they never create
/// one). Built by the adapter from a `StoredSceneClock`; the core stays
/// infrastructure-free and never sees the storage type.
#[derive(Debug, Clone, Default)]
pub struct TemporalSceneInput<'a> {
    /// The full drafted prose of the scene.
    pub prose: &'a str,
    /// Declared in-world span of the scene in days (`StoryClock::duration_days`).
    pub duration_days: Option<f64>,
    /// `linear|flashback|flashforward|concurrent` — a non-linear mode suppresses
    /// every finding (the whole scene is deliberately out of sequence).
    pub temporal_mode: Option<&'a str>,
    /// `minute|hour|day|week|month|year` — a coarse precision (week or larger)
    /// suppresses every finding (the author has declared time-of-day is not
    /// meaningful at this granularity).
    pub precision: Option<&'a str>,
}

/// Severity of a temporal-coherence finding. Advisory only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalCoherenceSeverity {
    Warning,
    Info,
}

/// A single intra-scene temporal-coherence finding.
#[derive(Debug, Clone, PartialEq)]
pub struct TemporalCoherenceHit {
    /// Stable machine kind: `temporal_teleport` | `temporal_drift` |
    /// `unrendered_declared_span`.
    pub kind: &'static str,
    pub severity: TemporalCoherenceSeverity,
    pub message: String,
    /// Byte span of the offending marker(s) in the prose, when localisable.
    pub byte_range: Option<TextByteRange>,
}

/// A minimum forward band-jump (e.g. morning → evening = 3) that, when
/// unbridged, reads as a teleport. Deliberately conservative: a two-band step
/// (morning → afternoon) is plausibly one continuous scene and never fires.
const FORWARD_TELEPORT_THRESHOLD: i16 = 3;

/// A minimum backward band-jump magnitude (e.g. night → morning = 4) that, when
/// unbridged, reads as an internal time-of-day contradiction.
const BACKWARD_DRIFT_THRESHOLD: i16 = 2;

/// A declared scene span (`duration_days`) at or above which the prose is
/// expected to render the passage with at least one transition or scene break.
const DECLARED_SPAN_DAYS: f64 = 1.0;

/// Ordered time-of-day vocabulary. Each phrase maps to a band ordinal so a
/// jump's size is the band delta. Word-boundary matched, so `afternoon` never
/// matches the `noon` band and `midnight` never matches the bare `night` band.
const TIME_OF_DAY_BANDS: &[(&str, i16)] = &[
    // 0 — first light (incl. the canonical night/dawn offices)
    ("dawn", 0),
    ("daybreak", 0),
    ("sunrise", 0),
    ("first light", 0),
    ("sunup", 0),
    ("cockcrow", 0),
    ("matins", 0),
    ("lauds", 0),
    // 1 — morning (incl. breakfast + the office of terce)
    ("morning", 1),
    ("forenoon", 1),
    ("breakfast", 1),
    ("terce", 1),
    // 2 — midday (incl. the midday meal)
    ("midday", 2),
    ("noonday", 2),
    ("noon", 2),
    ("luncheon", 2),
    ("lunch", 2),
    // 3 — afternoon
    ("afternoon", 3),
    // 4 — evening / dusk (incl. the evening meal + the office of vespers)
    ("evening", 4),
    ("dusk", 4),
    ("sunset", 4),
    ("sundown", 4),
    ("twilight", 4),
    ("nightfall", 4),
    ("gloaming", 4),
    ("dinner", 4),
    ("supper", 4),
    ("vespers", 4),
    // 5 — night (incl. the night office of compline)
    ("midnight", 5),
    ("night", 5),
    ("compline", 5),
];

/// Phrases that explicitly signal deliberate time passage. Their presence
/// between two band markers turns an otherwise-unmarked jump into an intended,
/// signalled ellipsis. Word-boundary matched, case-insensitive.
const TRANSITION_MARKERS: &[&str] = &[
    "later",
    "afterward",
    "afterwards",
    "thereafter",
    "eventually",
    "presently",
    "meanwhile",
    "soon after",
    "by the time",
    "by morning",
    "by breakfast",
    "by noon",
    "by midday",
    "by lunch",
    "by luncheon",
    "by afternoon",
    "by evening",
    "by supper",
    "by dinner",
    "by midnight",
    "by nightfall",
    "by night",
    "by dawn",
    "by dusk",
    "by sunrise",
    "by sunset",
    "by daybreak",
    "next morning",
    "next day",
    "next evening",
    "next night",
    "following morning",
    "following day",
    "hours later",
    "hours passed",
    "an hour later",
    "a few hours",
    "minutes later",
    "moments later",
    "a while later",
    "some time later",
    "time passed",
    "days later",
    "weeks later",
    "the sun rose",
    "the sun set",
    "the sun climbed",
    "the sun sank",
    "the sun dipped",
    "the sun crept",
    "darkness fell",
    "night fell",
    "night came",
    "dawn broke",
    "dusk fell",
    "dusk gathered",
    "shadows lengthened",
    "wore on",
];

/// Phrases that mark an in-scene recollection (a mini-flashback). Their presence
/// in the span between two band markers legitimately explains a reference to
/// another time, so it suppresses a finding the same way a transition does.
const RECALL_MARKERS: &[&str] = &[
    "remember",
    "remembered",
    "remembering",
    "recall",
    "recalled",
    "recalling",
    "memory of",
    "memories of",
    "thought back",
    "flashed back",
    "years ago",
    "years earlier",
    "years before",
    "long ago",
    "back when",
];

#[derive(Debug, Clone, Copy)]
struct BandAnchor {
    start: usize,
    end: usize,
    band: i16,
}

/// Scan one scene's prose for intra-scene temporal-coherence problems.
///
/// Pure and deterministic: the same `input` always yields the same hits, in
/// document order. Returns an empty vec for any scene with no detectable
/// problem, for a non-linear `temporal_mode`, or for coarse `precision`.
pub fn scan_temporal_coherence(input: &TemporalSceneInput) -> Vec<TemporalCoherenceHit> {
    // A scene deliberately out of linear sequence may carry any jump.
    if let Some(mode) = input.temporal_mode
        && matches!(mode, "flashback" | "flashforward" | "concurrent")
    {
        return Vec::new();
    }
    // At week-or-coarser precision the author has declared time-of-day is not
    // meaningful for this scene.
    if let Some(precision) = input.precision
        && matches!(precision, "week" | "month" | "year")
    {
        return Vec::new();
    }

    let prose = input.prose;
    // ASCII-lowercasing leaves byte length and char boundaries unchanged, so
    // offsets computed against `lower` index `prose` identically.
    let lower = prose.to_ascii_lowercase();

    let bands = extract_band_anchors(&lower);
    let bridges = extract_bridge_positions(prose, &lower);

    let mut hits = Vec::new();

    // Teleport / drift across consecutive established times-of-day.
    if let Some((first, rest)) = bands.split_first() {
        let mut current = first;
        for next in rest {
            let delta = next.band - current.band;
            let bridged = bridges
                .iter()
                .any(|&pos| pos > current.start && pos < next.start);
            if !bridged {
                if delta >= FORWARD_TELEPORT_THRESHOLD {
                    hits.push(TemporalCoherenceHit {
                        kind: "temporal_teleport",
                        severity: TemporalCoherenceSeverity::Warning,
                        message: format!(
                            "the scene jumps from {} to {} with no transition beat or scene break (unmarked time skip)",
                            band_label(current.band),
                            band_label(next.band),
                        ),
                        byte_range: Some(TextByteRange {
                            start: next.start,
                            end: next.end,
                        }),
                    });
                } else if delta <= -BACKWARD_DRIFT_THRESHOLD {
                    hits.push(TemporalCoherenceHit {
                        kind: "temporal_drift",
                        severity: TemporalCoherenceSeverity::Warning,
                        message: format!(
                            "the scene's time-of-day contradicts itself ({} after {}) within one continuous passage",
                            band_label(next.band),
                            band_label(current.band),
                        ),
                        byte_range: Some(TextByteRange {
                            start: next.start,
                            end: next.end,
                        }),
                    });
                }
            }
            current = next;
        }
    }

    // A declared multi-day span rendered as one unbroken block: the author
    // asserted time passes but never signalled it (an accidental ellipsis).
    if let Some(duration) = input.duration_days
        && duration >= DECLARED_SPAN_DAYS
        && bridges.is_empty()
    {
        hits.push(TemporalCoherenceHit {
            kind: "unrendered_declared_span",
            severity: TemporalCoherenceSeverity::Warning,
            message: format!(
                "the scene declares duration_days = {duration} but renders the span as one unbroken passage with no transition beat or scene break"
            ),
            byte_range: None,
        });
    }

    hits
}

fn band_label(band: i16) -> &'static str {
    match band {
        0 => "first light",
        1 => "morning",
        2 => "midday",
        3 => "afternoon",
        4 => "evening",
        _ => "night",
    }
}

/// Collect time-of-day anchors in document order, word-boundary matched, with
/// "good <band>" greetings, deadline idioms, gnomic statements, and
/// retrospective clauses filtered, and overlapping matches deduped.
fn extract_band_anchors(lower: &str) -> Vec<BandAnchor> {
    let mut anchors: Vec<BandAnchor> = Vec::new();
    for &(phrase, band) in TIME_OF_DAY_BANDS {
        let mut from = 0;
        while let Some(rel) = lower[from..].find(phrase) {
            let start = from + rel;
            let end = start + phrase.len();
            from = end;
            if !is_word_boundary(lower, start, end) {
                continue;
            }
            // A "good morning" / "good night" greeting is dialogue ritual, not
            // a time-of-day anchor.
            if lower[..start].ends_with("good ") {
                continue;
            }
            // "before lunch" / "before dawn" is a deadline idiom (relative
            // time), not the scene's clock (defect item 6).
            if lower[..start].ends_with("before ") {
                continue;
            }
            // A present-tense copula right after the token ("second breakfast
            // is a governing institution") marks a gnomic/habitual statement,
            // not scene time (defect item 6).
            if lower[end..].starts_with(" is ") || lower[end..].starts_with(" are ") {
                continue;
            }
            // A contracted past perfect anywhere in the sentence ("I'd faced
            // it down … at midnight") marks the clause as retrospective — its
            // time tokens describe the past, not scene-now (defect item 6).
            if sentence_is_retrospective(lower, start, end) {
                continue;
            }
            anchors.push(BandAnchor { start, end, band });
        }
    }
    anchors.extend(extract_clock_anchors(lower));
    // Sort by position (longer match first on ties) and drop overlaps so a
    // phrase that contains a shorter band word yields a single anchor.
    anchors.sort_by_key(|a| (a.start, std::cmp::Reverse(a.end)));
    let mut deduped: Vec<BandAnchor> = Vec::with_capacity(anchors.len());
    for anchor in anchors {
        if deduped.last().is_some_and(|last| anchor.start < last.end) {
            continue;
        }
        deduped.push(anchor);
    }
    deduped
}

/// Lazily-compiled regex for explicit meridian clock times. Meridian-only
/// (`8 a.m.`, `11 p.m.`, `8:30 pm`) on purpose: a bare `HH:MM` collides with
/// scores, ratios, and chapter/verse references, so it is deliberately excluded
/// to hold the detector's high-precision posture. `None` if compilation fails
/// (the scan then simply skips clock anchors — never panics).
fn clock_time_regex() -> Option<&'static regex::Regex> {
    static RE: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::RegexBuilder::new(r"\b(\d{1,2})(?::[0-5]\d)?\s*([ap])\.?\s?m\b")
            .case_insensitive(true)
            .build()
            .ok()
    })
    .as_ref()
}

/// Lazily-compiled regex for spelled-out "N in the morning/afternoon/evening"
/// clock phrases ("two in the morning"). Parsed as a clock hour so the small
/// hours read as night, not the bare "morning" band inside the phrase (defect
/// item 6). `None` if compilation fails (the scan skips these — never panics).
fn phrase_clock_regex() -> Option<&'static regex::Regex> {
    static RE: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::RegexBuilder::new(
            r"\b(one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|\d{1,2})\s+in\s+the\s+(morning|afternoon|evening)\b",
        )
        .case_insensitive(true)
        .build()
        .ok()
    })
    .as_ref()
}

fn parse_hour_word(word: &str) -> Option<i32> {
    match word {
        "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        "eleven" => Some(11),
        "twelve" => Some(12),
        other => other.parse::<i32>().ok().filter(|h| (1..=12).contains(h)),
    }
}

/// Extract explicit meridian clock times as time-of-day anchors, mapping the
/// hour to the same band scale as the word lexicon.
fn extract_clock_anchors(lower: &str) -> Vec<BandAnchor> {
    let mut anchors = Vec::new();
    if let Some(re) = phrase_clock_regex() {
        for caps in re.captures_iter(lower) {
            let (Some(whole), Some(hour_match), Some(part)) =
                (caps.get(0), caps.get(1), caps.get(2))
            else {
                continue;
            };
            let Some(hour) = parse_hour_word(hour_match.as_str()) else {
                continue;
            };
            // "in the morning" is what a speaker says for a.m. hours — small
            // hours (1–4) land in the night band via the hour scale, later
            // hours in first-light/morning. Afternoon/evening read as p.m.
            let hour24 = match part.as_str() {
                "morning" => {
                    if hour == 12 {
                        0
                    } else {
                        hour
                    }
                }
                _ => {
                    if hour == 12 {
                        12
                    } else {
                        hour + 12
                    }
                }
            };
            anchors.push(BandAnchor {
                start: whole.start(),
                end: whole.end(),
                band: hour_to_band(hour24),
            });
        }
    }
    let Some(re) = clock_time_regex() else {
        return anchors;
    };
    for caps in re.captures_iter(lower) {
        let (Some(whole), Some(hour_match), Some(meridian)) =
            (caps.get(0), caps.get(1), caps.get(2))
        else {
            continue;
        };
        let Ok(hour) = hour_match.as_str().parse::<i32>() else {
            continue;
        };
        if !(1..=12).contains(&hour) {
            continue;
        }
        let hour24 = if meridian.as_str().eq_ignore_ascii_case("p") {
            if hour == 12 { 12 } else { hour + 12 }
        } else if hour == 12 {
            0
        } else {
            hour
        };
        anchors.push(BandAnchor {
            start: whole.start(),
            end: whole.end(),
            band: hour_to_band(hour24),
        });
    }
    anchors
}

/// Map a 24-hour clock hour onto the time-of-day band scale (0 first light .. 5 night).
fn hour_to_band(hour24: i32) -> i16 {
    match hour24 {
        5 | 6 => 0,
        7..=10 => 1,
        11 | 12 => 2,
        13..=16 => 3,
        17..=20 => 4,
        _ => 5,
    }
}

/// Byte positions of every transition / recollection marker and scene break —
/// the signals that legitimately bridge a time jump.
fn extract_bridge_positions(prose: &str, lower: &str) -> Vec<usize> {
    let mut positions: Vec<usize> = Vec::new();
    for phrase in TRANSITION_MARKERS
        .iter()
        .chain(RECALL_MARKERS.iter())
        .copied()
    {
        let mut from = 0;
        while let Some(rel) = lower[from..].find(phrase) {
            let start = from + rel;
            let end = start + phrase.len();
            from = end;
            if is_word_boundary(lower, start, end) {
                positions.push(start);
            }
        }
    }
    positions.extend(scene_break_positions(prose));
    positions.sort_unstable();
    positions
}

/// Byte offsets of separator-only lines (`***`, `* * *`, `---`, `___`, `###`).
fn scene_break_positions(prose: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut offset = 0;
    for line in prose.split_inclusive('\n') {
        if is_scene_break_line(line.trim()) {
            positions.push(offset);
        }
        offset += line.len();
    }
    positions
}

fn is_scene_break_line(trimmed: &str) -> bool {
    if trimmed.chars().count() < 3 {
        return false;
    }
    let mut has_separator = false;
    for c in trimmed.chars() {
        match c {
            '*' | '-' | '_' | '#' => has_separator = true,
            ' ' | '\t' | '·' | '•' | '—' | '–' => {}
            _ => return false,
        }
    }
    has_separator
}

/// True when the sentence containing `[start, end)` carries a contracted past
/// perfect ("i'd", "she'd", …): the clause is retrospective, so its time tokens
/// describe the past rather than scene-now (defect item 6). Straight and curly
/// apostrophes both match. Deliberately narrower than bare "had" — "Night had
/// fallen" is standard scene-time narration and must keep anchoring.
fn sentence_is_retrospective(lower: &str, start: usize, end: usize) -> bool {
    // '.', '!', '?', '\n' are single-byte, so +1 stays on a char boundary.
    let sentence_start = lower[..start]
        .rfind(['.', '!', '?', '\n'])
        .map(|pos| pos + 1)
        .unwrap_or(0);
    let sentence_end = lower[end..]
        .find(['.', '!', '?', '\n'])
        .map(|pos| end + pos)
        .unwrap_or(lower.len());
    let sentence = &lower[sentence_start..sentence_end];
    sentence.contains("'d ") || sentence.contains("’d ")
}

/// True when the `[start, end)` slice of `s` is bounded by non-word characters
/// on both sides (or by the string ends).
fn is_word_boundary(s: &str, start: usize, end: usize) -> bool {
    let prev_ok = s[..start]
        .chars()
        .next_back()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
    let next_ok = s[end..]
        .chars()
        .next()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
    prev_ok && next_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(prose: &str) -> Vec<TemporalCoherenceHit> {
        scan_temporal_coherence(&TemporalSceneInput {
            prose,
            ..Default::default()
        })
    }

    fn kinds(hits: &[TemporalCoherenceHit]) -> Vec<&'static str> {
        hits.iter().map(|h| h.kind).collect()
    }

    #[test]
    fn plain_continuous_scene_is_silent() {
        // A scene that stays within one part of the day must never fire.
        let hits = scan(
            "The morning market was loud. She haggled over fish and bread, \
             elbowing through the morning crowd until her basket was full.",
        );
        assert!(
            hits.is_empty(),
            "single-band scene must be silent: {hits:?}"
        );
    }

    #[test]
    fn small_forward_step_is_below_the_teleport_threshold() {
        // morning -> afternoon is a two-band step: plausibly one continuous
        // scene, so the conservative threshold must not fire.
        let hits = scan(
            "Morning found him on the road north. The afternoon sun beat down \
             on the same rutted road and the same tired horse.",
        );
        assert!(
            !hits.iter().any(|h| h.kind == "temporal_teleport"),
            "a two-band step must stay below the teleport threshold: {hits:?}"
        );
    }

    #[test]
    fn internal_contradiction_backward_is_drift() {
        // Establishes night, then refers to the same scene's morning with no
        // bridge: the prose contradicts its own time-of-day.
        let hits = scan(
            "Night had fallen over the city and the lamps were lit along the \
             quay. The morning sun struck the rooftops as he spoke.",
        );
        assert!(
            hits.iter().any(|h| h.kind == "temporal_drift"),
            "a backward time-of-day contradiction must be flagged as drift: {hits:?}"
        );
    }

    #[test]
    fn mourning_does_not_match_morning() {
        // Word-boundary guard: "mourning" must not be read as "morning" and
        // create a phantom night->morning drift.
        let hits = scan(
            "Night had fallen over the city. She wore mourning black at the \
             graveside and did not weep.",
        );
        assert!(
            hits.is_empty(),
            "'mourning' must not match the morning band: {hits:?}"
        );
    }

    #[test]
    fn greeting_good_morning_is_not_a_time_anchor() {
        // A "good morning"/"good night" greeting is dialogue ritual, not a
        // time-of-day anchor, and must not create a contradiction.
        let hits = scan(
            "It was the dead of night in the camp. \"Good morning,\" the sentry \
             said with a smirk, and waved him through.",
        );
        assert!(
            hits.is_empty(),
            "a 'good morning' greeting must not anchor time: {hits:?}"
        );
    }

    #[test]
    fn meridian_clock_times_map_to_correct_bands() {
        // 8 a.m. is morning (band 1), 8 p.m. is evening (band 4): a 3-band jump
        // that fires only if p.m. is parsed as evening, not re-read as morning.
        let hits = scan(
            "He reached the office at 8 a.m. and bent over the same ledger. He \
             signed the last page at 8 p.m. and did not look up once.",
        );
        assert!(
            hits.iter().any(|h| h.kind == "temporal_teleport"),
            "8 a.m. -> 8 p.m. must be a forward jump (p.m. = evening): {hits:?}"
        );
    }

    #[test]
    fn bare_numbers_are_not_clock_times() {
        // Scores, ratios, and chapter refs must not register as times.
        let hits = scan(
            "Chapter 8 ran long. The vote was 11 to 3, and the ledger showed a \
             ratio of 8 to 5 against him.",
        );
        assert!(
            hits.is_empty(),
            "bare numbers without a meridian must not anchor time: {hits:?}"
        );
    }

    #[test]
    fn coarse_precision_suppresses_findings() {
        // At week/month/year precision the author has declared time-of-day is
        // not meaningful; the scan stays silent even on a hard jump.
        let hits = scan_temporal_coherence(&TemporalSceneInput {
            prose: "He rose in the morning. By the deep of midnight he was still \
                    riding hard for the pass.",
            precision: Some("month"),
            ..Default::default()
        });
        assert!(
            hits.is_empty(),
            "coarse precision must suppress findings: {kinds:?}",
            kinds = kinds(&hits)
        );
    }

    // ── Idiom / retrospective false positives (defect item 6) ──

    #[test]
    fn deadline_idiom_before_lunch_is_not_an_anchor() {
        // "before lunch" in "ended a career with a shrug before lunch" is a
        // deadline idiom describing capability, not the scene's clock. It must
        // not anchor midday and then read the scene's real night as a skip.
        let hits = scan(
            "The man could end a career with a shrug before lunch. The night \
             pressed close around the tavern as he said it.",
        );
        assert!(
            hits.is_empty(),
            "'before <meal>' deadline idiom must not anchor time: {hits:?}"
        );
    }

    #[test]
    fn small_hours_clock_phrase_reads_as_night_not_morning() {
        // "two in the morning" is the small hours of the night; the bare
        // "morning" token inside the phrase must not fire a night->morning
        // drift inside a night section.
        let hits = scan(
            "The night watch dragged on and the fire burned low. By two in the \
             morning the coals had gone grey.",
        );
        assert!(
            hits.is_empty(),
            "'N in the morning' must map by hour, not the word 'morning': {hits:?}"
        );
    }

    #[test]
    fn gnomic_present_statement_is_not_an_anchor() {
        // "second breakfast is a governing institution" is a gnomic (habitual
        // present) statement, not the scene's clock; it must not read as a
        // backward jump inside an afternoon passage.
        let hits = scan(
            "The afternoon light slanted across the table. In this household, \
             second breakfast is a governing institution.",
        );
        assert!(
            hits.is_empty(),
            "a present-copula gnomic statement must not anchor time: {hits:?}"
        );
    }

    #[test]
    fn retrospective_contracted_clause_is_not_an_anchor() {
        // A past-perfect contraction ("I'd", "she'd") marks the sentence as
        // retrospective; its time tokens describe the past, not scene-now, and
        // must not flag in either direction inside a morning section.
        let hits = scan(
            "The morning light filled the kitchen. I'd faced it down in the \
             top drawer at midnight. By breakfast she'd demoted it to a coaster.",
        );
        assert!(
            hits.is_empty(),
            "retrospective contracted clauses must not anchor time: {hits:?}"
        );
    }

    #[test]
    fn linear_mode_does_not_suppress() {
        // Only non-linear modes suppress; an explicit "linear" mode is the
        // normal case and must still flag.
        let hits = scan_temporal_coherence(&TemporalSceneInput {
            prose: "He woke in the grey morning and ate. The midnight harbour \
                    lay black and still around him.",
            temporal_mode: Some("linear"),
            ..Default::default()
        });
        assert!(
            hits.iter().any(|h| h.kind == "temporal_teleport"),
            "linear mode must not suppress a real teleport: {hits:?}"
        );
    }
}
