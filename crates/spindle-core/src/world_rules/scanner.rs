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
/// * v4 — plain verb alternations match common inflections ("invoked",
///   "channeled") and explicit cost-omission prose promotes a hit to Likely.
/// * v5 — required cost/effect nouns are derived from rule descriptions, and
///   their explicit absence promotes a hit to Likely even when the prose does
///   not repeat the rule's scan label.
/// * v6 — an in-dialogue hit on a secrecy-class rule is itself the prohibited
///   disclosure, so it is Likely rather than a vague Possible mention.
///
/// The adapters layer mixes this into the `world_rule_semantic_drift` cache
/// context hash, so deploying new scanner semantics invalidates cached
/// findings computed by the old logic instead of serving them forever.
pub const SCANNER_SEMANTICS_VERSION: u32 = 6;

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

/// Rule descriptions that state a required cost/effect. A nearby explicit
/// absence marker is a semantic omission even when the prose never says
/// "bypass": "invoked ... with dry, unbloodied palms" or "zero battery charge".
const COST_LANGUAGE_MARKERS: &[&str] = &[
    "require",
    "requires",
    "required",
    "cost",
    "costs",
    "toll",
    "price",
    "drain",
    "drains",
    "sacrifice",
    "must pay",
    "must spend",
    "cannot",
    "with no",
    "without cost",
];

const STRONG_COST_ABSENCE_MARKERS: &[&str] = &[
    "without cost",
    "without paying",
    "no cost",
    "paid nothing",
    "effortless",
    "effortlessly",
    "unharmed",
    "unscathed",
    "unbloodied",
    "free of charge",
    "pleasant warmth",
    "no eye strain",
];

/// Weak markers only promote when a cost/effect noun from the rule is in the
/// same adjacency window; otherwise "zero" is ordinary prose.
const WEAK_COST_ABSENCE_MARKERS: &[&str] = &["zero", "dry palms"];

const COST_EFFECT_NOUNS: &[&str] = &[
    "battery",
    "charge",
    "blood",
    "toll",
    "frostbite",
    "cost",
    "price",
    "fuel",
    "sacrifice",
    "eye strain",
    "strain",
    "cooldown",
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
        let mut rule_hits = Vec::new();
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
                    let severity = if secrecy_class {
                        // For a secrecy-class rule, an in-dialogue hit is the
                        // prohibited disclosure itself, not a vague mention.
                        WorldRuleSeverity::Likely
                    } else {
                        severity_for_hit(prose, mat.start(), mat.end(), &rule.description)
                    };
                    rule_hits.push(build_hit(
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
        if !secrecy_class
            && !rule_hits
                .iter()
                .any(|hit| hit.severity == WorldRuleSeverity::Likely)
            && let Some((start, end)) = derived_cost_omission_hit(prose, &rule.description)
        {
            rule_hits.push(build_hit(
                &rule.rule_id,
                start,
                end,
                prose,
                WorldRuleSeverity::Likely,
            ));
        }
        hits.extend(rule_hits);
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

/// Extract required cost/effect noun phrases from a rule description.
///
/// These are general syntactic cues ("opens only when X is fed", "costs the
/// scribe one memory", "requires X", "without X"), not corpus-specific words.
fn cost_requirement_phrases(description: &str) -> Vec<String> {
    let mut phrases = Vec::new();
    let lower = description.to_ascii_lowercase();
    let patterns = [
        r"only when (?:a |an |the )?(?:fed |given |paid |spent )?(?:a |an |the )?([a-z][a-z ' -]{1,45})",
        r"(?:requires?|needs?|drains?|spends?|consumes?|sacrifices?|pays?) (?:a |an |the |one |[0-9]+ )?([a-z][a-z ' -]{1,45})",
        r"(?:costs?|costing) (?:the )?[a-z][a-z' -]{0,25} (?:one|a|an|the|[0-9]+) ([a-z][a-z ' -]{1,35})",
        r"(?:costs?|costing) (?:one|a|an|[0-9]+) ([a-z][a-z ' -]{1,35})",
        r"must (?:pay|spend|feed|use|sacrifice|give up) (?:a |an |the |one |[0-9]+ )?([a-z][a-z ' -]{1,35})",
        r"(?:cannot|can't|can not|never|must not|may not|will not|won't)\s+[^.;]{0,40}\bwithout\s+(?:a |an |the )?([a-z][a-z ' -]{1,40})",
    ];
    for pattern in patterns {
        let Ok(regex) = regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .size_limit(1 << 20)
            .build()
        else {
            continue;
        };
        for capture in regex.captures_iter(&lower) {
            let Some(raw) = capture.get(1).map(|m| m.as_str()) else {
                continue;
            };
            let full_start = capture.get(0).map(|m| m.start()).unwrap_or(0);
            if has_negated_absence(&lower, full_start) {
                continue;
            }
            if let Some(phrase) = truncate_cost_phrase(raw)
                && !phrases.contains(&phrase)
            {
                phrases.push(phrase);
            }
        }
    }
    phrases
}

fn truncate_cost_phrase(raw: &str) -> Option<String> {
    const STOP: &[&str] = &[
        "is", "are", "was", "were", "be", "been", "being", "to", "for", "from", "that", "which",
        "when", "if", "and", "but", "or", "with", "without", "as", "at", "by", "of", "on", "in",
        "the", "a", "an", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
        "ten", "hundred", "thousand", "percent", "its", "his", "her", "their", "cannot", "can",
        "must", "should",
    ];
    let mut words = Vec::new();
    for word in raw.split_whitespace() {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
        if word.is_empty() || STOP.contains(&word) {
            break;
        }
        words.push(word);
        if words.len() == 4 {
            break;
        }
    }
    if words.is_empty() {
        return None;
    }
    Some(words.join(" "))
}

fn cost_head_noun(phrase: &str) -> Option<&str> {
    phrase
        .split_whitespace()
        .last()
        .filter(|word| word.len() >= 3)
}

fn sentence_window(prose: &str, start: usize, end: usize) -> &str {
    let bytes = prose.as_bytes();
    let mut begin = start;
    while begin > 0 && !matches!(bytes[begin - 1], b'.' | b'!' | b'?' | b';' | b'\n') {
        begin -= 1;
    }
    let mut finish = end.min(prose.len());
    while finish < prose.len() && !matches!(bytes[finish], b'.' | b'!' | b'?' | b';' | b'\n') {
        finish += 1;
    }
    &prose[begin..finish]
}

fn has_negated_absence(sentence: &str, absence_start: usize) -> bool {
    const NEGATIONS: &[&str] = &[
        "not",
        "never",
        "cannot",
        "can't",
        "didn't",
        "doesn't",
        "isn't",
        "wasn't",
        "wouldn't",
        "won't",
        "no longer",
    ];
    let lower = sentence.to_ascii_lowercase();
    let before_start = floor_char_boundary(&lower, absence_start.saturating_sub(48));
    let before = &lower[before_start..absence_start];
    NEGATIONS.iter().any(|negation| {
        if negation.contains(' ') {
            before.contains(negation)
        } else {
            window_contains_word(before, negation)
        }
    })
}

fn has_deferred_requirement(sentence: &str, absence_end: usize) -> bool {
    let lower = sentence.to_ascii_lowercase();
    let after_end = ceil_char_boundary(&lower, (absence_end + 36).min(lower.len()));
    let after = &lower[absence_end..after_end];
    [
        "required",
        "needed",
        "necessary",
        "allowed",
        "permitted",
        "lost",
        "forgotten",
    ]
    .iter()
    .any(|word| after.contains(word))
}

/// Absence pattern for one marker. `without` may have a gerund between it and
/// the cost noun ("without giving up a memory"); `no` stays close to the noun
/// so an unrelated earlier "no" cannot span a whole clause.
fn cost_absence_patterns(variant: &str) -> [String; 2] {
    let escaped = regex::escape(variant);
    [
        format!(r"\bwithout\b[^.!?;]{{0,60}}\b{escaped}\b"),
        format!(r"\bno\b(?:\s+[a-z][a-z'-]*){{0,2}}\s+\b{escaped}\b"),
    ]
}

/// Content words from the rule description that identify its domain. Cost
/// words themselves are excluded so "no memory" alone cannot trigger a rule
/// whose description happens to mention memory.
fn cost_anchor_tokens(description: &str, phrases: &[String]) -> Vec<String> {
    const STOP: &[&str] = &[
        "only",
        "when",
        "with",
        "without",
        "from",
        "that",
        "which",
        "this",
        "these",
        "those",
        "into",
        "onto",
        "over",
        "under",
        "than",
        "then",
        "there",
        "their",
        "them",
        "they",
        "cost",
        "costs",
        "costing",
        "require",
        "requires",
        "required",
        "need",
        "needs",
        "needed",
        "must",
        "should",
        "cannot",
        "can",
        "will",
        "would",
        "could",
        "have",
        "has",
        "had",
        "been",
        "being",
        "each",
        "every",
        "some",
        "more",
        "most",
        "less",
        "least",
        "opens",
        "open",
        "fed",
        "feed",
        "given",
        "give",
        "paid",
        "pay",
        "spends",
        "spend",
        "consumes",
        "consume",
        "sacrifices",
        "sacrifice",
        "uses",
        "use",
        "rings",
        "ring",
    ];
    let cost_tokens: Vec<String> = phrases
        .iter()
        .flat_map(|phrase| phrase.split_whitespace())
        .map(|word| word.to_ascii_lowercase())
        .collect();
    let mut anchors = Vec::new();
    for raw in description.split(|c: char| !c.is_alphanumeric() && c != '\'') {
        let word = raw.trim_matches('\'').to_ascii_lowercase();
        if word.len() < 4
            || STOP.contains(&word.as_str())
            || cost_tokens.contains(&word)
            || anchors.contains(&word)
        {
            continue;
        }
        anchors.push(word);
    }
    anchors
}

fn contains_anchor(sentence: &str, anchors: &[String]) -> bool {
    if anchors.is_empty() {
        return true;
    }
    let words: Vec<String> = sentence
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|word| word.trim_matches('\'').to_ascii_lowercase())
        .filter(|word| !word.is_empty())
        .collect();
    anchors.iter().any(|anchor| words.contains(anchor))
}

fn sentence_ranges(prose: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, ch) in prose.char_indices() {
        if matches!(ch, '.' | '!' | '?' | ';' | '\n') {
            ranges.push((start, index));
            start = index + ch.len_utf8();
        }
    }
    if start < prose.len() {
        ranges.push((start, prose.len()));
    }
    ranges
}

/// Find the first cost-omission phrase anywhere in the prose, independent of
/// whether the rule's scan pattern appears. The rule must still be active and
/// declared with a scan pattern; this only closes the gap where prose names
/// the omitted cost without repeating the rule's label.
fn match_cost_omission(sentence: &str, phrases: &[String]) -> Option<(usize, usize)> {
    let lower = sentence.to_ascii_lowercase();
    for phrase in phrases {
        let mut variants = vec![phrase.clone()];
        if let Some(head) = cost_head_noun(phrase)
            && head != phrase
        {
            variants.push(head.to_string());
        }
        for variant in variants {
            for pattern in cost_absence_patterns(&variant) {
                let Ok(regex) = regex::RegexBuilder::new(&pattern)
                    .case_insensitive(true)
                    .size_limit(1 << 20)
                    .build()
                else {
                    continue;
                };
                for capture in regex.find_iter(&lower) {
                    let matched = capture.as_str().to_ascii_lowercase();
                    if matched.starts_with("no one")
                        || matched.starts_with("no body")
                        || matched.starts_with("no thing")
                    {
                        continue;
                    }
                    let absence_marker = if matched.starts_with("without") {
                        "without"
                    } else {
                        "no"
                    };
                    if has_negated_absence(&lower, capture.start()) {
                        continue;
                    }
                    if absence_marker == "no" && has_deferred_requirement(&lower, capture.end()) {
                        continue;
                    }
                    return Some((capture.start(), capture.end()));
                }
            }
        }
    }
    None
}

/// Find the first cost-omission phrase anywhere in the prose, independent of
/// whether the rule's scan pattern appears. The rule must still be active and
/// declared with a scan pattern; this only closes the gap where prose names
/// the omitted cost without repeating the rule's label.
fn derived_cost_omission_hit(prose: &str, description: &str) -> Option<(usize, usize)> {
    let phrases = cost_requirement_phrases(description);
    if phrases.is_empty() {
        return None;
    }
    let anchors = cost_anchor_tokens(description, &phrases);
    for (range_start, range_end) in sentence_ranges(prose) {
        let sentence = &prose[range_start..range_end];
        if !contains_anchor(sentence, &anchors) {
            continue;
        }
        if let Some((start, end)) = match_cost_omission(sentence, &phrases) {
            return Some((range_start + start, range_start + end));
        }
    }
    None
}

/// True when the prose names a required cost/effect noun as absent. Handles
/// "without a/the <cost>", "without <cost>", "no <cost>", and an intervening
/// gerund ("without giving up a memory"). Negated requirements stay neutral.
fn has_derived_cost_omission(
    prose: &str,
    hit_start: usize,
    hit_end: usize,
    description: &str,
) -> bool {
    let phrases = cost_requirement_phrases(description);
    if phrases.is_empty() {
        return false;
    }
    match_cost_omission(sentence_window(prose, hit_start, hit_end), &phrases).is_some()
}

fn severity_for_hit(
    prose: &str,
    hit_start: usize,
    hit_end: usize,
    rule_description: &str,
) -> WorldRuleSeverity {
    let raw_start = hit_start.saturating_sub(VIOLATION_CONTEXT_RADIUS);
    let raw_end = hit_end
        .saturating_add(VIOLATION_CONTEXT_RADIUS)
        .min(prose.len());
    let window_start = floor_char_boundary(prose, raw_start);
    let window_end = ceil_char_boundary(prose, raw_end);
    let window = prose[window_start..window_end].to_ascii_lowercase();

    if has_derived_cost_omission(prose, hit_start, hit_end, rule_description) {
        return WorldRuleSeverity::Likely;
    }

    if VIOLATION_CONTEXT_MARKERS
        .iter()
        .any(|marker| window_contains_word(&window, marker))
    {
        return WorldRuleSeverity::Likely;
    }

    let description = rule_description.to_ascii_lowercase();
    let has_cost_language = COST_LANGUAGE_MARKERS
        .iter()
        .any(|marker| description.contains(marker));
    let strong_absence = STRONG_COST_ABSENCE_MARKERS
        .iter()
        .any(|marker| window_contains_marker(&window, marker));
    let weak_absence = WEAK_COST_ABSENCE_MARKERS
        .iter()
        .any(|marker| window_contains_marker(&window, marker));
    let cost_noun_present = COST_EFFECT_NOUNS
        .iter()
        .any(|noun| window_contains_marker(&window, noun));
    let has_cost_absence = strong_absence || (weak_absence && cost_noun_present);
    if has_cost_language && has_cost_absence {
        WorldRuleSeverity::Likely
    } else {
        WorldRuleSeverity::Possible
    }
}

fn window_contains_marker(window: &str, marker: &str) -> bool {
    if marker.contains(' ') {
        window.contains(marker)
    } else {
        window_contains_word(window, marker)
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

/// Plain verb alternations such as `channel|cast|invoke|unleash` are written
/// as stems, so prose uses `invoked`/`channeled`. Expand each long alternative
/// with ordinary inflections rather than making every short noun pattern stem
/// (which would re-introduce "stat" matching "stated").
fn expand_plain_verb_alternation(pattern: &str) -> Option<String> {
    if !pattern.contains('|') {
        return None;
    }
    let alternatives: Vec<&str> = pattern.split('|').map(str::trim).collect();
    if alternatives.len() < 2
        || alternatives.iter().any(|alternative| {
            alternative.is_empty()
                || !alternative
                    .chars()
                    .all(|character| character.is_ascii_alphabetic())
        })
        || !alternatives
            .iter()
            .any(|alternative| alternative.len() >= 5)
    {
        return None;
    }
    let expanded = alternatives
        .into_iter()
        .map(|alternative| {
            if alternative.len() < 5 {
                return alternative.to_string();
            }
            if let Some(stem) = alternative.strip_suffix('e') {
                format!("(?:{alternative}(?:s|d)?|{stem}ing)")
            } else {
                format!("(?:{alternative}(?:s|es|ed|ing)?)")
            }
        })
        .collect::<Vec<_>>()
        .join("|");
    Some(expanded)
}

fn build_regex(pattern: &str) -> Option<regex::Regex> {
    let anchored = match expand_plain_verb_alternation(pattern) {
        // Expansion is only produced for plain alphabetic alternatives, so the
        // whole group always needs both word boundaries.
        Some(expanded) => format!(r"\b(?:{expanded})\b"),
        None => anchor_pattern_word_boundaries(pattern),
    };
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
    fn inflected_plain_verb_alternation_matches_invoked() {
        let prose = "Tarek Nouri then invoked a deep neural splice on the relay.";
        let rules = vec![make_rule(
            "world_rule:splice",
            "channel|cast|invoke|unleash",
            "Neural Splice Cooldown",
            "Running a deep neural splice drains battery and causes eye strain.",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1, "expected an inflected invoke hit");
    }

    #[test]
    fn inflected_plain_verb_alternation_preserves_base_and_matches_forms() {
        let rules = vec![make_rule(
            "world_rule:splice",
            "channel|cast|invoke|unleash",
            "Neural Splice Cooldown",
            "Running a deep neural splice drains battery and causes eye strain.",
        )];
        for prose in [
            "They invoke the splice without paying its cost.",
            "She invokes the splice.",
            "He invoked the splice.",
            "It is invoking the splice.",
        ] {
            let hits = scan_prose_for_world_rules(prose, &rules);
            assert_eq!(hits.len(), 1, "expected one hit for: {prose}");
        }

        let prose = "They invoke the splice without paying its cost.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Likely);
    }

    #[test]
    fn implicit_cost_omission_promotes_to_likely() {
        let prose = "Tarek Nouri then invoked a deep neural splice on the relay using zero battery charge and suffering no eye strain at all.";
        let rules = vec![make_rule(
            "world_rule:splice",
            "channel|cast|invoke|unleash",
            "Neural Splice Cooldown",
            "Running a deep neural splice drains thirty percent of the rig battery and causes eye strain.",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Likely);
    }

    #[test]
    fn cost_absence_without_rule_anchor_is_ignored() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Cost",
            "The moon gate opens only when a silver key is fed to the lock.",
        )];
        let prose = "The door opened without a key and the hall stayed quiet.";
        assert!(scan_prose_for_world_rules(prose, &rules).is_empty());
    }

    #[test]
    fn waived_cost_description_does_not_promote() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Waiver",
            "The moon gate opens without a silver key.",
        )];
        let prose = "The moon gate opened without a key.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Possible);
    }

    #[test]
    fn negated_requirement_description_does_not_promote() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Waiver",
            "The moon gate does not require a silver key.",
        )];
        let prose = "The moon gate opened without a key.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Possible);
    }

    #[test]
    fn negation_word_substring_does_not_suppress_affirmative_omission() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Cost",
            "The moon gate opens only when a silver key is fed to the lock.",
        )];
        let prose = "She noticed the moon gate open without a key.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Likely);
    }

    #[test]
    fn multibyte_context_is_boundary_safe() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Cost",
            "The moon gate opens only when a silver key is fed to the lock.",
        )];
        let prose = "Ééééééééééééééééééééééééééééééééééééééééééééééééé the moon gate opened without a key ✦.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Likely);

        let deferred = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Cost",
            "The moon gate opens only when a silver key is fed to the lock.",
        )];
        let prose = "No silver key was required ✦ Élodie’s note.";
        let hits = scan_prose_for_world_rules(prose, &deferred);
        assert!(
            hits.iter()
                .all(|hit| hit.severity == WorldRuleSeverity::Possible)
        );
    }

    #[test]
    fn no_pronoun_and_negated_loss_are_not_cost_omissions() {
        let ink = vec![make_rule(
            "world_rule:ink",
            "ink binding",
            "Ink Binding Cost",
            "Ink bindings cost the scribe one remembered word.",
        )];
        let prose = "No one spoke the missing word.";
        assert!(scan_prose_for_world_rules(prose, &ink).is_empty());

        let song = vec![make_rule(
            "world_rule:song",
            "bell-song",
            "Bell Song Cost",
            "A bell-song costs the singer one recollection.",
        )];
        let prose = "No hymn was sung, and no recollection was lost.";
        assert!(scan_prose_for_world_rules(prose, &song).is_empty());
    }

    #[test]
    fn derived_cost_omission_without_rule_label_is_likely() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Cost",
            "The moon gate opens only when a silver key is fed to the lock.",
        )];
        let prose = "The lock opened without a key and the keeper shrugged.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Likely);
    }

    #[test]
    fn derived_cost_noun_without_head_is_likely() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Cost",
            "The moon gate opens only when a silver key is fed to the lock.",
        )];
        let prose = "The moon gate opened without a key and nobody noticed.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Likely);
    }

    #[test]
    fn derived_cost_noun_with_intervening_gerund_is_likely() {
        let rules = vec![make_rule(
            "world_rule:ash",
            "ash-ink",
            "Ash Ink Cost",
            "Ash-ink bindings cost the scribe one memory.",
        )];
        let prose = "She bound the map with ash-ink without surrendering a memory.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Likely);
    }

    #[test]
    fn derived_cost_noun_from_without_clause_is_likely() {
        let rules = vec![make_rule(
            "world_rule:signal",
            "signal",
            "Signal Relay",
            "The signal cannot cross the marsh without a living relay.",
        )];
        let prose = "The signal crossed the marsh without a relay and arrived clear.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Likely);
    }

    #[test]
    fn negated_requirement_is_not_cost_omission() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Cost",
            "The moon gate opens only when a silver key is fed to the lock.",
        )];
        let prose = "The moon gate did not open without a key.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Possible);
    }

    #[test]
    fn stated_requirement_is_not_cost_omission() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Cost",
            "The moon gate opens only when a silver key is fed to the lock.",
        )];
        let prose = "No silver key was required to open the moon gate.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Possible);
    }

    #[test]
    fn paid_cost_is_not_cost_omission() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Cost",
            "The moon gate opens only when a silver key is fed to the lock.",
        )];
        let prose = "The moon gate opened with a silver key.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Possible);
    }

    #[test]
    fn unrelated_without_phrase_is_not_cost_omission() {
        let rules = vec![make_rule(
            "world_rule:moon",
            "moon gate",
            "Moon Gate Cost",
            "The moon gate opens only when a silver key is fed to the lock.",
        )];
        let prose = "The moon gate opened without delay.";
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Possible);
    }

    #[test]
    fn neutral_cost_rule_mention_stays_possible() {
        let prose = "Elena channeled the splice and paid the battery cost before the cooldown.";
        let rules = vec![make_rule(
            "world_rule:splice",
            "channel|cast|invoke|unleash",
            "Neural Splice Cooldown",
            "Running a deep neural splice drains battery and causes eye strain.",
        )];
        let hits = scan_prose_for_world_rules(prose, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, WorldRuleSeverity::Possible);
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
