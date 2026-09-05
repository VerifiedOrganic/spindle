use crate::models::{TextByteRange, WorldRuleHit, WorldRuleSeverity};

/// Semantics version of the world-rule prose scanner. Bump this whenever the
/// scanner's hit-producing behavior changes:
///
/// * v1 — original: every rule's `scan_pattern` matched anywhere in the scene
///   text, severity from adjacency markers.
/// * v2 — secrecy-class rules (see `ScanRule::is_secrecy_class`) only flag
///   hits inside quoted dialogue spans; narration and HUD readouts are
///   excluded.
/// * v3 — plain patterns anchor to word boundaries ("stat" no longer matches
///   inside "statement"); stem matching is explicit ("resurrect\\w*").
///
/// The adapters layer mixes this into the `world_rule_semantic_drift` cache
/// context hash, so deploying new scanner semantics invalidates cached
/// findings computed by the old logic instead of serving them forever.
pub const SCANNER_SEMANTICS_VERSION: u32 = 3;

pub struct ScanRule {
    pub rule_id: String,
    pub scan_pattern: Option<String>,
    pub rule_name: String,
    pub description: String,
    /// Free-form rule classification from the world_rule row (e.g.
    /// "magic_limitation", "secrecy"). Drives secrecy-class scoping; may be
    /// empty for older call paths.
    pub rule_type: String,
}

impl ScanRule {
    /// Secrecy-class rules ("Nate must never disclose the N.A.I.P.") constrain
    /// what characters SAY to each other — disclosure — not what the narration
    /// or a private interface readout mentions. A literal scan_pattern like
    /// "the system" therefore false-positives on every correct usage: interior
    /// narration ("a process the system had optimized...") and HUD blocks
    /// ("*[The system notes: Well done]*") both match while disclosing
    /// nothing. For these rules the scanner keeps only hits inside dialogue
    /// spans, so real violations (a character saying the secret aloud) still
    /// fire and narration/HUD noise is dropped.
    ///
    /// Classification is heuristic so existing rows need no migration: a rule
    /// is secrecy-class when its type or name says so, or its description
    /// phrases the constraint as non-disclosure.
    pub fn is_secrecy_class(&self) -> bool {
        let type_or_name = format!("{} {}", self.rule_type, self.rule_name).to_ascii_lowercase();
        if type_or_name.contains("secrecy") || type_or_name.contains("secret") {
            return true;
        }
        let description = self.description.to_ascii_lowercase();
        const NON_DISCLOSURE_PHRASES: &[&str] = &[
            "disclose",
            "never reveal",
            "not reveal",
            "never tell",
            "must not tell",
            "keep secret",
            "keeps secret",
            "keeping secret",
            "secret from",
            "secrecy",
        ];
        NON_DISCLOSURE_PHRASES
            .iter()
            .any(|phrase| description.contains(phrase))
    }
}

/// Adjacency window around a pattern hit (in bytes) used to detect prose that
/// contextually suggests a violation. Matches the window used by the Phase-4
/// validator so the commit gate and `check_consistency` agree.
const VIOLATION_CONTEXT_RADIUS: usize = 80;

/// Words in the prose surrounding a pattern hit that promote severity from
/// Possible to Likely. Kept intentionally conservative: these are unambiguous
/// markers of intent to violate. Match is case-insensitive. "without" and
/// "despite" were removed (defect item 6): they are ordinary function words —
/// "went quiet without dying" is not a violation admission — and promoted far
/// more neutral prose than they caught.
const VIOLATION_CONTEXT_MARKERS: &[&str] = &[
    "violate",
    "violates",
    "violated",
    "violation",
    "violations",
    "break",
    "breaks",
    "broke",
    "breaking",
    "ignore",
    "ignores",
    "ignored",
    "ignoring",
    "circumvent",
    "circumvents",
    "bypass",
    "bypasses",
    "bypassed",
    "bypassing",
];

pub fn scan_prose_for_world_rules(prose: &str, rules: &[ScanRule]) -> Vec<WorldRuleHit> {
    // Dialogue spans are only needed when at least one secrecy-class rule is
    // in play; compute them lazily, once per scene.
    let mut dialogue_spans: Option<Vec<(usize, usize)>> = None;
    let mut hits = Vec::new();
    for rule in rules {
        let pattern = match rule.scan_pattern.as_deref() {
            Some(p) if !p.trim().is_empty() => p.trim(),
            _ => continue,
        };
        let secrecy_class = rule.is_secrecy_class();
        if secrecy_class && dialogue_spans.is_none() {
            dialogue_spans = Some(compute_dialogue_spans(prose));
        }
        match build_regex(pattern) {
            Some(re) => {
                for mat in re.find_iter(prose) {
                    // Secrecy-class scope: narration and bracketed interface
                    // readouts are not disclosure; only spoken words count.
                    if secrecy_class
                        && !spans_contain(
                            dialogue_spans.as_deref().unwrap_or(&[]),
                            mat.start(),
                            mat.end(),
                        )
                    {
                        continue;
                    }
                    let severity = severity_for_hit(prose, mat.start(), mat.end());
                    hits.push(build_hit(
                        &rule.rule_id,
                        mat.start(),
                        mat.end(),
                        prose,
                        severity,
                    ));
                }
            }
            None => continue,
        }
    }
    hits
}

fn spans_contain(spans: &[(usize, usize)], start: usize, end: usize) -> bool {
    spans
        .iter()
        .any(|&(span_start, span_end)| start >= span_start && end <= span_end)
}

/// Byte ranges of quoted dialogue in `prose`, covering both straight (`"`)
/// and curly (`“…”`) quote pairs. Straight quotes toggle open/close; curly
/// quotes use their dedicated glyphs. Unbalanced quotes degrade to "no span"
/// rather than a panic — this is a heuristic filter, not a parser.
fn compute_dialogue_spans(prose: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut open: Option<usize> = None;
    for (index, ch) in prose.char_indices() {
        match ch {
            '"' | '“' | '”' => {
                if let Some(start) = open.take() {
                    // A closing glyph (or a second straight quote) ends the
                    // span; an opener while one is open treats the previous
                    // opener as unmatched and restarts.
                    if ch != '“' {
                        spans.push((start, index + ch.len_utf8()));
                    } else {
                        open = Some(index);
                    }
                } else if ch != '”' {
                    open = Some(index);
                }
            }
            _ => {}
        }
    }
    spans
}

fn severity_for_hit(prose: &str, hit_start: usize, hit_end: usize) -> WorldRuleSeverity {
    let raw_start = hit_start.saturating_sub(VIOLATION_CONTEXT_RADIUS);
    let raw_end = hit_end
        .saturating_add(VIOLATION_CONTEXT_RADIUS)
        .min(prose.len());
    let window_start = floor_char_boundary(prose, raw_start);
    let window_end = ceil_char_boundary(prose, raw_end);
    let window = prose[window_start..window_end].to_ascii_lowercase();

    if VIOLATION_CONTEXT_MARKERS
        .iter()
        .any(|marker| window_contains_word(&window, marker))
    {
        WorldRuleSeverity::Likely
    } else {
        WorldRuleSeverity::Possible
    }
}

fn window_contains_word(window: &str, needle: &str) -> bool {
    let mut search_from = 0usize;
    while let Some(rel) = window[search_from..].find(needle) {
        let start = search_from + rel;
        let end = start + needle.len();
        let prev_is_alnum = window[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let next_is_alnum = window[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !prev_is_alnum && !next_is_alnum {
            return true;
        }
        search_from = end;
    }
    false
}

fn build_regex(pattern: &str) -> Option<regex::Regex> {
    let anchored = anchor_pattern_word_boundaries(pattern);
    regex::RegexBuilder::new(&format!("(?i){anchored}"))
        .size_limit(1 << 16)
        .build()
        .ok()
        .or_else(|| {
            let escaped = regex::escape(pattern);
            let anchored = anchor_pattern_word_boundaries(&escaped);
            regex::RegexBuilder::new(&format!("(?i){anchored}"))
                .size_limit(1 << 16)
                .build()
                .ok()
        })
}

/// A plain substring pattern means "this word or phrase", not "these bytes
/// anywhere": a Stat Growth Rate rule with pattern "stat" fired inside
/// "statement" (and would fire inside "station"/"status"). Anchor each end
/// of the pattern with `\b` when the adjacent pattern char is a word char.
/// The non-capturing group keeps alternations ("stat|stats") correctly
/// bounded on both sides. Patterns that already carry explicit boundaries
/// (`\bsigil\b`) get a redundant, harmless `\b`. Patterns ending in
/// punctuation ("N.A.I.P.") get no trailing boundary. Callers who WANT stem
/// matching ("resurrect" catching "resurrected"/"resurrection") write it
/// explicitly, e.g. "resurrect\\w*".
fn anchor_pattern_word_boundaries(pattern: &str) -> String {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let starts_word = pattern.chars().next().is_some_and(is_word);
    let ends_word = pattern.chars().last().is_some_and(is_word);
    match (starts_word, ends_word) {
        (true, true) => format!(r"\b(?:{pattern})\b"),
        (true, false) => format!(r"\b(?:{pattern})"),
        (false, true) => format!(r"(?:{pattern})\b"),
        (false, false) => pattern.to_string(),
    }
}

fn build_hit(
    rule_id: &str,
    start: usize,
    end: usize,
    prose: &str,
    severity: WorldRuleSeverity,
) -> WorldRuleHit {
    let surrounding_text = extract_surrounding_text(prose, start, end);
    WorldRuleHit {
        rule_id: rule_id.to_string(),
        byte_range: TextByteRange { start, end },
        severity,
        surrounding_text,
    }
}

fn extract_surrounding_text(prose: &str, start: usize, end: usize) -> String {
    let context_radius = 40usize;
    let raw_start = start.saturating_sub(context_radius);
    let raw_end = (end + context_radius).min(prose.len());
    let ctx_start = floor_char_boundary(prose, raw_start);
    let ctx_end = ceil_char_boundary(prose, raw_end);
    prose[ctx_start..ctx_end].to_string()
}

fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(id: &str, pattern: &str, name: &str, desc: &str) -> ScanRule {
        ScanRule {
            rule_id: id.to_string(),
            scan_pattern: Some(pattern.to_string()),
            rule_name: name.to_string(),
            description: desc.to_string(),
            rule_type: String::new(),
        }
    }

    fn make_typed_rule(
        id: &str,
        pattern: &str,
        name: &str,
        desc: &str,
        rule_type: &str,
    ) -> ScanRule {
        ScanRule {
            rule_type: rule_type.to_string(),
            ..make_rule(id, pattern, name, desc)
        }
    }

    #[test]
    fn regex_pattern_match_neutral_context_is_possible() {
        // Pattern hit with no violation language in the surrounding prose
        // should be flagged as Possible, not Likely. Severity is determined
        // by prose context, not by rule metadata (the rule description still
        // contains "must" but that no longer affects severity).
        let prose = "Eldrin cast a flame sigil across the room.";
        let rules = vec![make_rule(
            "world_rule:abc",
            r"\bsigil\b",
            "Sigil Rule",
            "Magic sigils must require physical contact",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rule_id, "world_rule:abc");
        assert_eq!(hits[0].severity, WorldRuleSeverity::Possible);
        assert!(hits[0].surrounding_text.contains("sigil"));
    }

    #[test]
    fn regex_pattern_match_violation_context_is_likely() {
        // Same pattern, but the surrounding prose now signals intent to
        // violate. Severity should promote to Likely.
        let prose = "Eldrin tried to ignore the sigil and cast at range anyway.";
        let rules = vec![make_rule(
            "world_rule:abc",
            r"\bsigil\b",
            "Sigil Rule",
            "Magic sigils must require physical contact",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Likely);
    }

    #[test]
    fn substring_match_fallback() {
        let prose = "The blood seal required physical contact to activate.";
        let rules = vec![make_rule(
            "world_rule:xyz",
            "blood seal",
            "Blood Seal",
            "Blood seal contracts",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rule_id, "world_rule:xyz");
        assert!(hits[0].byte_range.start <= prose.find("blood seal").unwrap());
        assert!(hits[0].surrounding_text.contains("blood seal"));
    }

    #[test]
    fn no_false_positives_on_unrelated_prose() {
        let prose = "The cat sat on the mat and looked out the window.";
        let rules = vec![make_rule(
            "world_rule:irrelevant",
            r"\bsigil\b",
            "Sigil Rule",
            "Magic sigils require physical contact",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert!(hits.is_empty());
    }

    #[test]
    fn rule_without_scan_pattern_is_skipped() {
        let prose = "The blood seal activated.";
        let rules = vec![ScanRule {
            rule_id: "world_rule:skip".to_string(),
            scan_pattern: None,
            rule_name: "SkipRule".to_string(),
            description: "No pattern".to_string(),
            rule_type: String::new(),
        }];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert!(hits.is_empty());
    }

    #[test]
    fn empty_scan_pattern_is_skipped() {
        let prose = "Any text at all.";
        let rules = vec![make_rule("world_rule:empty", "", "Empty", "Empty pattern")];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert!(hits.is_empty());
    }

    #[test]
    fn multiple_hits_from_one_rule() {
        let prose = "He drew a sigil, then another sigil appeared.";
        let rules = vec![make_rule(
            "world_rule:multi",
            r"\bsigil\b",
            "Sigil Rule",
            "Magic sigils must be drawn by hand",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn multiple_rules_match_independently() {
        let prose = "The blood seal required a sigil to activate.";
        let rules = vec![
            make_rule(
                "world_rule:1",
                r"\bsigil\b",
                "Sigil Rule",
                "Sigil requires contact",
            ),
            make_rule(
                "world_rule:2",
                "blood seal",
                "Blood Seal",
                "A seal of blood",
            ),
        ];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn possible_severity_when_no_violation_context_in_prose() {
        // Pattern hit, but the surrounding prose carries no violation
        // markers. Severity stays Possible regardless of rule metadata.
        let prose = "A gentle breeze carried the scent.";
        let rules = vec![make_rule(
            "world_rule:breeze",
            r"\bbreeze\b",
            "Gentle Wind",
            "A breeze may portend change",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Possible);
    }

    #[test]
    fn metadata_keywords_no_longer_promote_severity() {
        // Regression: a rule whose name or description contains words like
        // "must", "requires", "never", "forbidden" should NOT auto-promote
        // hits to Likely. Severity must come from prose context only.
        // This is the exact failure mode users hit when a routine noun
        // appears in scene prose and the rule description happens to use
        // strong language.
        let prose = "The party walked past the System interface and continued on.";
        let rules = vec![make_rule(
            "world_rule:system",
            r"\bSystem\b",
            "System Interaction Rule",
            "Players must interact with the System through Quest panels",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].severity,
            WorldRuleSeverity::Possible,
            "neutral prose must not be promoted to Likely by rule metadata"
        );
    }

    #[test]
    fn bare_without_near_hit_does_not_promote_to_likely() {
        // Defect item 6: "the comedy went quiet without dying" was promoted to
        // Likely because "without" sat in the context window. "without" (and
        // "despite") are ordinary function words, not unambiguous markers of
        // intent to violate — a neutral sentence must stay Possible.
        let prose = "The comedy went quiet without dying, and the room held it.";
        let rules = vec![make_rule(
            "world_rule:tone",
            "comedy",
            "Tone Mandate",
            "The narration must remain a comedy",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].severity,
            WorldRuleSeverity::Possible,
            "'without' alone must not read as a violation admission"
        );
    }

    #[test]
    fn case_insensitive_match_via_substring() {
        let prose = "The Blood Seal glowed brightly.";
        let rules = vec![make_rule(
            "world_rule:cs",
            "blood seal",
            "Blood Seal",
            "A blood seal contract",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0]
                .surrounding_text
                .to_lowercase()
                .contains("blood seal")
        );
    }

    #[test]
    fn surrounding_text_does_not_panic_on_multibyte_utf8() {
        let prose = "Voilà le blood seal dans la forêt enchantée.";
        let rules = vec![make_rule(
            "world_rule:utf8",
            "blood seal",
            "Blood Seal",
            "A blood seal",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0]
                .surrounding_text
                .to_lowercase()
                .contains("blood seal")
        );
    }

    #[test]
    fn regex_with_special_chars_escaped_fallback() {
        let prose = "The [forbidden] gate opened wider.";
        let rules = vec![make_rule(
            "world_rule:bracket",
            "[forbidden]",
            "Forbidden Mark",
            "A forbidden marker",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert!(!hits.is_empty());
        let hit_text = hits[0].surrounding_text.to_lowercase();
        assert!(hit_text.contains("forbidden"));
    }

    #[test]
    fn invalid_regex_falls_back_to_escaped_literal() {
        let prose = "He invoked the unclosed( bracket pattern.";
        let rules = vec![make_rule(
            "world_rule:unclosed",
            "unclosed(",
            "Unclosed Regex",
            "An invalid regex pattern",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].surrounding_text.to_lowercase().contains("unclosed"));
    }

    #[test]
    fn secrecy_rule_ignores_narration_hits() {
        // Vegas-skip FP: first-person interior narration mentioning the system
        // is not disclosure to another character.
        let prose =
            "It was a process the system had optimized by recommending specific bristle angles.";
        let rules = vec![make_typed_rule(
            "world_rule:naip",
            "the system",
            "System Secrecy",
            "Nate must never disclose the N.A.I.P.",
            "secrecy",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert!(hits.is_empty(), "narration must not fire a secrecy rule");
    }

    #[test]
    fn secrecy_rule_ignores_hud_readout_hits() {
        // Vegas-skip FP: a bracketed interface block is the system addressing
        // the holder privately, not the holder disclosing it.
        let prose =
            "He spat into the sink.\n\n*[The system notes: Well done]*\n\nHe stared at the mirror.";
        let rules = vec![make_typed_rule(
            "world_rule:naip",
            "the system",
            "System Secrecy",
            "Nate must never disclose the N.A.I.P.",
            "secrecy",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert!(hits.is_empty(), "HUD readouts must not fire a secrecy rule");
    }

    #[test]
    fn secrecy_rule_still_flags_dialogue_disclosure() {
        // The real violation the rule exists for: a character says the secret
        // out loud. Straight-quote dialogue must still fire.
        let prose = "Nate shrugged. \"The system told me which bristles to buy,\" he said.";
        let rules = vec![make_typed_rule(
            "world_rule:naip",
            "the system",
            "System Secrecy",
            "Nate must never disclose the N.A.I.P.",
            "secrecy",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1, "spoken disclosure must still be flagged");
    }

    #[test]
    fn secrecy_rule_flags_curly_quote_dialogue() {
        let prose = "She leaned in. “The system picks my battles now,” he admitted.";
        let rules = vec![make_typed_rule(
            "world_rule:naip",
            "the system",
            "System Secrecy",
            "Nate must never disclose the N.A.I.P.",
            "secrecy",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1, "curly-quoted dialogue must still be flagged");
    }

    #[test]
    fn secrecy_classification_from_description_without_type() {
        // Rows authored before any rule_type convention: the non-disclosure
        // phrasing in the description alone marks the rule secrecy-class.
        let prose = "The system hummed quietly in the back of his mind.";
        let rules = vec![make_rule(
            "world_rule:naip",
            "the system",
            "NAIP",
            "Nate must never disclose the N.A.I.P.",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert!(hits.is_empty());
    }

    #[test]
    fn non_secrecy_rule_still_scans_narration() {
        // Guard against over-scoping: an ordinary rule keeps matching
        // narration hits exactly as before.
        let prose = "Eldrin cast a flame sigil across the room.";
        let rules = vec![make_typed_rule(
            "world_rule:abc",
            r"\bsigil\b",
            "Sigil Rule",
            "Magic sigils must require physical contact",
            "magic_limitation",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn substring_pattern_does_not_match_inside_longer_words() {
        // Stat Growth Rate FP: pattern "stat" fired on the four bytes of
        // "stat" inside "statement" — and would fire inside "station" and
        // "status". Plain patterns mean whole words.
        let prose = "His statement at the station updated his status.";
        let rules = vec![make_typed_rule(
            "world_rule:growth",
            "stat",
            "Stat Growth Rate",
            "Stats must grow slowly",
            "power_cost",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert!(
            hits.is_empty(),
            "pattern 'stat' must not match inside statement/station/status"
        );
    }

    #[test]
    fn substring_pattern_still_matches_the_standalone_word() {
        let prose = "He checked his stat sheet, then his other stat.";
        let rules = vec![make_typed_rule(
            "world_rule:growth",
            "stat",
            "Stat Growth Rate",
            "Stats must grow slowly",
            "power_cost",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 2, "the standalone word still matches");
        assert_eq!(
            &prose[hits[0].byte_range.start..hits[0].byte_range.end],
            "stat"
        );
    }

    #[test]
    fn alternation_pattern_is_bounded_on_both_sides() {
        // Without the non-capturing group, naive \b wrapping would produce
        // `\bstat|stats\b` — a leading anchor on only the first alternative.
        let prose = "The statement of stats.";
        let rules = vec![make_typed_rule(
            "world_rule:growth",
            "stat|stats",
            "Stat Growth Rate",
            "Stats must grow slowly",
            "power_cost",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            &prose[hits[0].byte_range.start..hits[0].byte_range.end],
            "stats"
        );
    }

    #[test]
    fn explicit_stem_pattern_opt_in_still_works() {
        // Word-boundary anchoring is the default, but a caller who wants
        // stem matching writes it explicitly.
        let prose = "She refused to resurrect him; resurrection was impossible.";
        let rules = vec![make_typed_rule(
            "world_rule:necro",
            r"resurrect\w*",
            "No Resurrection",
            "Resurrection magic is impossible",
            "magic_limitation",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(
            hits.len(),
            2,
            "the explicit stem catches 'resurrect' and 'resurrection'"
        );
        // "resurrection" is NOT matched by the plain pattern anymore —
        // that is the intended breaking change.
        let plain = vec![make_typed_rule(
            "world_rule:necro",
            "resurrect",
            "No Resurrection",
            "Resurrection magic is impossible",
            "magic_limitation",
        )];
        let plain_hits = scan_prose_for_world_rules(prose, &plain);
        assert_eq!(plain_hits.len(), 1);
        assert_eq!(
            &prose[plain_hits[0].byte_range.start..plain_hits[0].byte_range.end],
            "resurrect"
        );
    }

    #[test]
    fn pattern_ending_in_punctuation_gets_no_trailing_boundary() {
        let prose = "The N.A.I.P. protocol was classified.";
        let rules = vec![make_typed_rule(
            "world_rule:naip",
            "N.A.I.P.",
            "NAIP Secrecy",
            "Never disclose the N.A.I.P.",
            "secrecy",
        )];
        // Narration hit: suppressed by secrecy scoping, but the pattern
        // itself must still compile and match (verified via a dialogue hit).
        assert!(scan_prose_for_world_rules(prose, &rules).is_empty());
        let dialogue = "He said, \"The N.A.I.P. protocol,\" and stopped.";
        let hits = scan_prose_for_world_rules(dialogue, &rules);
        assert_eq!(hits.len(), 1);
    }
}
