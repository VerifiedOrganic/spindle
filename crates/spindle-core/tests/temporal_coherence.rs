//! Golden-fixture tests for the intra-scene temporal-coherence scanner.
//!
//! Each fixture is a committed manuscript fragment (`tests/temporal_coherence/
//! fixtures/*.md`) so the corpus the detector is tuned against is reviewable in
//! diffs and cannot silently drift. The mandate cases live here; the
//! micro-behaviour guards (word boundaries, greetings, precision) live inline in
//! the module.

use spindle_core::temporal::{
    TemporalCoherenceSeverity, TemporalSceneInput, scan_temporal_coherence,
};

const UNMARKED_JUMP: &str =
    include_str!("temporal_coherence/fixtures/morning_to_night_unmarked.md");
const SIGNALED_ELLIPSIS: &str = include_str!("temporal_coherence/fixtures/signaled_ellipsis.md");
const SCENE_BREAK_SIGNAL: &str = include_str!("temporal_coherence/fixtures/scene_break_signal.md");
const MULTI_DAY_UNMARKED: &str = include_str!("temporal_coherence/fixtures/multi_day_unmarked.md");
const MULTI_DAY_SIGNALED: &str = include_str!("temporal_coherence/fixtures/multi_day_signaled.md");
const MEALS_UNMARKED: &str =
    include_str!("temporal_coherence/fixtures/meals_breakfast_to_supper_unmarked.md");
const BELLS_UNMARKED: &str =
    include_str!("temporal_coherence/fixtures/bells_matins_to_vespers_unmarked.md");
const CLOCK_UNMARKED: &str =
    include_str!("temporal_coherence/fixtures/clock_morning_to_night_unmarked.md");
const SIGNALED_MEAL: &str = include_str!("temporal_coherence/fixtures/signaled_meal_transition.md");

fn prose(text: &str) -> TemporalSceneInput<'_> {
    TemporalSceneInput {
        prose: text,
        ..Default::default()
    }
}

/// THE mandated must-flag case: a single scene that wakes in the morning, eats
/// breakfast, and a sentence later stands under the midnight sky — with no
/// transition language and no scene break.
#[test]
fn unmarked_morning_to_night_jump_is_flagged() {
    let hits = scan_temporal_coherence(&prose(UNMARKED_JUMP));
    assert!(
        hits.iter().any(|h| h.kind == "temporal_teleport"),
        "an unmarked morning->night jump must be flagged as a teleport: {hits:?}"
    );
    let teleport = hits
        .iter()
        .find(|h| h.kind == "temporal_teleport")
        .expect("teleport hit present");
    assert_eq!(teleport.severity, TemporalCoherenceSeverity::Warning);
    assert!(
        teleport.byte_range.is_some(),
        "a teleport hit should localise the offending marker"
    );
}

/// THE mandated must-pass case: the same large jump, but bridged with an
/// explicit "Hours later," transition. An intended, signalled ellipsis.
#[test]
fn signaled_ellipsis_passes() {
    let hits = scan_temporal_coherence(&prose(SIGNALED_ELLIPSIS));
    assert!(
        !hits.iter().any(|h| h.kind == "temporal_teleport"),
        "a transition-signalled ellipsis must not be flagged: {hits:?}"
    );
}

/// Edge path: a scene break (`* * *`) is itself a valid signal for a jump.
#[test]
fn scene_break_is_a_valid_signal() {
    let hits = scan_temporal_coherence(&prose(SCENE_BREAK_SIGNAL));
    assert!(
        !hits.iter().any(|h| h.kind == "temporal_teleport"),
        "a jump across a scene break must not be flagged: {hits:?}"
    );
}

/// Edge path: a scene that *declares* it spans real time via `duration_days`
/// but renders that span as one unbroken block with no transition is an
/// accidental (unrendered) ellipsis.
#[test]
fn declared_multi_day_span_without_transition_is_flagged() {
    let input = TemporalSceneInput {
        prose: MULTI_DAY_UNMARKED,
        duration_days: Some(3.0),
        ..Default::default()
    };
    let hits = scan_temporal_coherence(&input);
    assert!(
        hits.iter().any(|h| h.kind == "unrendered_declared_span"),
        "a declared multi-day scene with no transition markers must be flagged: {hits:?}"
    );
}

/// The intended-vs-accidental distinction: the same declared multi-day span,
/// rendered with transitions ("The next day", "By the third day"), passes.
#[test]
fn declared_multi_day_span_with_transitions_passes() {
    let input = TemporalSceneInput {
        prose: MULTI_DAY_SIGNALED,
        duration_days: Some(3.0),
        ..Default::default()
    };
    let hits = scan_temporal_coherence(&input);
    assert!(
        !hits.iter().any(|h| h.kind == "unrendered_declared_span"),
        "a declared multi-day scene rendered with transitions must not be flagged: {hits:?}"
    );
    assert!(
        !hits.iter().any(|h| h.kind == "temporal_teleport"),
        "the bridged morning->evening progression must not be flagged: {hits:?}"
    );
}

/// Lexicon hardening: meal-name time cues (breakfast → supper) are recognized
/// as a time-of-day progression.
#[test]
fn unmarked_meal_jump_is_flagged() {
    let hits = scan_temporal_coherence(&prose(MEALS_UNMARKED));
    assert!(
        hits.iter().any(|h| h.kind == "temporal_teleport"),
        "an unmarked breakfast->supper jump must be flagged: {hits:?}"
    );
}

/// Lexicon hardening: canonical-hour / bell vocabulary (matins → vespers).
#[test]
fn unmarked_canonical_hour_jump_is_flagged() {
    let hits = scan_temporal_coherence(&prose(BELLS_UNMARKED));
    assert!(
        hits.iter().any(|h| h.kind == "temporal_teleport"),
        "an unmarked matins->vespers jump must be flagged: {hits:?}"
    );
}

/// Lexicon hardening: explicit meridian clock times (8 a.m. → 11 p.m.).
#[test]
fn unmarked_clock_time_jump_is_flagged() {
    let hits = scan_temporal_coherence(&prose(CLOCK_UNMARKED));
    assert!(
        hits.iter().any(|h| h.kind == "temporal_teleport"),
        "an unmarked 8 a.m.->11 p.m. jump must be flagged: {hits:?}"
    );
}

/// Precision is preserved: the same meal jump, bridged with a transition,
/// produces no finding.
#[test]
fn signaled_meal_transition_passes() {
    let hits = scan_temporal_coherence(&prose(SIGNALED_MEAL));
    assert!(
        !hits.iter().any(|h| h.kind == "temporal_teleport"),
        "a bridged meal transition must not be flagged: {hits:?}"
    );
}

/// Edge path: a scene deliberately marked as a flashback may legitimately carry
/// a hard time jump; the non-linear mode suppresses every finding.
#[test]
fn flashback_mode_suppresses_every_finding() {
    for mode in ["flashback", "flashforward", "concurrent"] {
        let input = TemporalSceneInput {
            prose: UNMARKED_JUMP,
            temporal_mode: Some(mode),
            ..Default::default()
        };
        let hits = scan_temporal_coherence(&input);
        assert!(
            hits.is_empty(),
            "temporal_mode={mode} must suppress all findings: {hits:?}"
        );
    }
}
