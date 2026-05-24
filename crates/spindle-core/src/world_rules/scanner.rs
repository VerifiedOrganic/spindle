use crate::models::{TextByteRange, WorldRuleHit, WorldRuleSeverity};

pub struct ScanRule {
    pub rule_id: String,
    pub scan_pattern: Option<String>,
    pub rule_name: String,
    pub description: String,
}

/// Adjacency window around a pattern hit (in bytes) used to detect prose that
/// contextually suggests a violation. Matches the window used by the Phase-4
/// validator so the commit gate and `check_consistency` agree.
const VIOLATION_CONTEXT_RADIUS: usize = 80;

/// Words in the prose surrounding a pattern hit that promote severity from
/// Possible to Likely. Kept intentionally conservative: these are unambiguous
/// markers of intent to violate. Match is case-insensitive.
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
    "without",
    "despite",
    "circumvent",
    "circumvents",
    "bypass",
    "bypasses",
    "bypassed",
    "bypassing",
];

pub fn scan_prose_for_world_rules(prose: &str, rules: &[ScanRule]) -> Vec<WorldRuleHit> {
    let mut hits = Vec::new();
    for rule in rules {
        let pattern = match rule.scan_pattern.as_deref() {
            Some(p) if !p.trim().is_empty() => p.trim(),
            _ => continue,
        };
        match build_regex(pattern) {
            Some(re) => {
                for mat in re.find_iter(prose) {
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
    let prefixed = format!("(?i){}", pattern);
    regex::RegexBuilder::new(&prefixed)
        .size_limit(1 << 16)
        .build()
        .ok()
        .or_else(|| {
            let escaped = regex::escape(pattern);
            regex::RegexBuilder::new(&format!("(?i){}", escaped))
                .size_limit(1 << 16)
                .build()
                .ok()
        })
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
}
