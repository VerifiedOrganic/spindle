use crate::models::{CanonicalFactReadModel, CanonicalValue, StoryPlacement};
use regex::Regex;
use serde_json::Value;
use std::ops::Range;
use std::sync::LazyLock;

static TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\p{L}[\p{L}\p{M}'’\-]*|\d[\d,]*(?:\.\d+)?").expect("token regex compiles")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationSeverity {
    Hard,
    Soft,
}

#[derive(Debug, Clone)]
pub struct CanonicalFactViolation {
    pub fact_id: String,
    pub predicate: String,
    pub expected: CanonicalValue,
    pub observed: String,
    pub byte_range: Range<usize>,
    pub severity: ViolationSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalFactForValidation {
    pub canonical_fact_id: String,
    pub predicate: String,
    pub value_kind: String,
    pub value_text: Option<String>,
    pub value_number: Option<f64>,
    pub value_unit: Option<String>,
    pub value_json: Option<Value>,
    pub aliases: Vec<String>,
    pub valid_from: Option<StoryPlacement>,
    pub valid_until: Option<StoryPlacement>,
    pub legacy_untyped: bool,
}

impl From<CanonicalFactReadModel> for CanonicalFactForValidation {
    fn from(value: CanonicalFactReadModel) -> Self {
        Self {
            canonical_fact_id: value.canonical_fact_id,
            predicate: value.predicate,
            value_kind: value.value_kind,
            value_text: value.value_text,
            value_number: value.value_number,
            value_unit: value.value_unit,
            value_json: value.value_json,
            aliases: value.aliases,
            valid_from: value.valid_from,
            valid_until: value.valid_until,
            legacy_untyped: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatorConfig {
    pub alias_window_tokens: usize,
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self {
            // Window of 12 tokens balances nearby contradiction detection with low false positives.
            alias_window_tokens: 12,
        }
    }
}

#[derive(Debug, Clone)]
struct Token {
    normalized: String,
    start: usize,
    end: usize,
}

pub fn validate_prose_against_facts(
    prose: &str,
    facts: &[CanonicalFactForValidation],
) -> Vec<CanonicalFactViolation> {
    validate_prose_against_facts_with_config(prose, facts, ValidatorConfig::default())
}

fn validate_prose_against_facts_with_config(
    prose: &str,
    facts: &[CanonicalFactForValidation],
    config: ValidatorConfig,
) -> Vec<CanonicalFactViolation> {
    let tokens = tokenize(prose, &TOKEN_RE);
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut violations = Vec::new();

    for fact in facts {
        if fact.legacy_untyped {
            continue;
        }
        if fact.aliases.is_empty() {
            continue;
        }
        let alias_hits = find_alias_hits(fact, &tokens);
        if alias_hits.is_empty() {
            continue;
        }
        let expected = expected_value_for_fact(fact);
        match fact.value_kind.as_str() {
            "number" => validate_number(
                prose,
                &tokens,
                fact,
                &expected,
                &alias_hits,
                config.alias_window_tokens,
                &mut violations,
            ),
            "date" => validate_date(
                prose,
                &tokens,
                fact,
                &expected,
                &alias_hits,
                config.alias_window_tokens,
                &mut violations,
            ),
            "string" => validate_string(
                prose,
                &tokens,
                fact,
                &expected,
                &alias_hits,
                &mut violations,
                false,
            ),
            "enum" => validate_enum(
                prose,
                &tokens,
                fact,
                &expected,
                &alias_hits,
                &mut violations,
                false,
            ),
            "range" => validate_range(
                prose,
                &tokens,
                fact,
                &expected,
                &alias_hits,
                config.alias_window_tokens,
                &mut violations,
            ),
            "list" => validate_list(
                prose,
                &tokens,
                fact,
                &expected,
                &alias_hits,
                config.alias_window_tokens,
                &mut violations,
            ),
            "boolean" => validate_enum(
                prose,
                &tokens,
                fact,
                &expected,
                &alias_hits,
                &mut violations,
                true,
            ),
            _ => {}
        }
    }

    violations
}

pub fn parse_written_numeral(input: &str) -> Option<i64> {
    let tokens = tokenize(input, &TOKEN_RE)
        .into_iter()
        .map(|token| token.normalized)
        .collect::<Vec<_>>();
    let (value, consumed) = parse_written_numeral_tokens(&tokens, 0)?;
    if consumed == tokens.len() {
        Some(value)
    } else {
        None
    }
}

fn tokenize(prose: &str, token_re: &Regex) -> Vec<Token> {
    token_re
        .find_iter(prose)
        .map(|m| Token {
            normalized: normalize_token(m.as_str()),
            start: m.start(),
            end: m.end(),
        })
        .filter(|token| !token.normalized.is_empty())
        .collect()
}

fn normalize_token(input: &str) -> String {
    input
        .trim_matches(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '.' | ','
                        | ';'
                        | ':'
                        | '!'
                        | '?'
                        | '"'
                        | '\''
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '—'
                        | '–'
                )
        })
        .to_lowercase()
}

fn normalize_text_for_contains(prose: &str) -> String {
    prose
        .chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
}

fn expected_value_for_fact(fact: &CanonicalFactForValidation) -> CanonicalValue {
    match fact.value_kind.as_str() {
        "number" => CanonicalValue::Number {
            value: fact.value_number.unwrap_or_default(),
            unit: fact.value_unit.clone(),
        },
        "date" => date_expected_from_fact(fact)
            .map_or_else(|| CanonicalValue::Text(String::new()), CanonicalValue::Date),
        "string" => CanonicalValue::Text(fact.value_text.clone().unwrap_or_default()),
        "enum" => {
            let choice = fact.value_text.clone().unwrap_or_default();
            let choices = fact
                .value_json
                .as_ref()
                .and_then(|json| json.get("choices"))
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            CanonicalValue::Enum { choice, choices }
        }
        "range" => CanonicalValue::Range {
            min: fact
                .value_json
                .as_ref()
                .and_then(|json| json.get("min"))
                .and_then(Value::as_f64)
                .unwrap_or_default(),
            max: fact
                .value_json
                .as_ref()
                .and_then(|json| json.get("max"))
                .and_then(Value::as_f64)
                .unwrap_or_default(),
            unit: fact.value_unit.clone(),
        },
        "list" => CanonicalValue::List {
            required: fact
                .value_json
                .as_ref()
                .and_then(|json| json.get("required"))
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            forbidden: fact
                .value_json
                .as_ref()
                .and_then(|json| json.get("forbidden"))
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        },
        "boolean" => CanonicalValue::Boolean(
            fact.value_json
                .as_ref()
                .and_then(Value::as_bool)
                .or_else(|| fact.value_text.as_deref().and_then(parse_bool))
                .unwrap_or(false),
        ),
        _ => CanonicalValue::Text(fact.value_text.clone().unwrap_or_default()),
    }
}

fn date_expected_from_fact(fact: &CanonicalFactForValidation) -> Option<StoryPlacement> {
    if let Some(json) = fact.value_json.as_ref() {
        if let Some(year) = json.get("year").and_then(Value::as_i64) {
            return Some(StoryPlacement {
                book_number: year as i32,
                chapter_number: 1,
                scene_order: None,
                note: None,
            });
        }
        if let Some(book_number) = json.get("book_number").and_then(Value::as_i64) {
            let chapter_number = json
                .get("chapter_number")
                .and_then(Value::as_i64)
                .unwrap_or(1);
            let scene_order = json
                .get("scene_order")
                .and_then(Value::as_i64)
                .map(|value| value as i32);
            return Some(StoryPlacement {
                book_number: book_number as i32,
                chapter_number: chapter_number as i32,
                scene_order,
                note: None,
            });
        }
    }
    fact.value_text
        .as_deref()
        .and_then(extract_year)
        .map(|year| StoryPlacement {
            book_number: year,
            chapter_number: 1,
            scene_order: None,
            note: None,
        })
}

fn find_alias_hits(fact: &CanonicalFactForValidation, tokens: &[Token]) -> Vec<(usize, usize)> {
    let mut hits = Vec::new();
    for alias in &fact.aliases {
        let alias_tokens = alias
            .split_whitespace()
            .map(normalize_token)
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>();
        if alias_tokens.is_empty() || alias_tokens.len() > tokens.len() {
            continue;
        }
        for idx in 0..=(tokens.len() - alias_tokens.len()) {
            if alias_tokens
                .iter()
                .zip(tokens[idx..idx + alias_tokens.len()].iter())
                .all(|(expected, actual)| expected == &actual.normalized)
            {
                hits.push((idx, idx + alias_tokens.len() - 1));
            }
        }
    }
    hits.sort_unstable();
    hits.dedup();
    hits
}

fn parse_numeric_candidate(token: &str) -> Option<f64> {
    let cleaned = token.replace(',', "");
    cleaned.parse::<f64>().ok()
}

fn parse_written_numeral_tokens(tokens: &[String], start_idx: usize) -> Option<(i64, usize)> {
    if start_idx >= tokens.len() {
        return None;
    }
    let mut idx = start_idx;
    let mut total = 0_i64;
    let mut current = 0_i64;
    let mut consumed_any = false;

    while idx < tokens.len() {
        let token = tokens[idx].replace('-', " ");
        let parts = token.split_whitespace().collect::<Vec<_>>();
        let mut consumed_this = false;
        for part in parts {
            if part == "and" {
                consumed_this = consumed_any;
                continue;
            }
            if let Some(unit) = numeral_unit_value(part) {
                current += unit;
                consumed_this = true;
                consumed_any = true;
                continue;
            }
            if let Some(tens) = numeral_tens_value(part) {
                current += tens;
                consumed_this = true;
                consumed_any = true;
                continue;
            }
            if part == "hundred" && current > 0 {
                current *= 100;
                consumed_this = true;
                consumed_any = true;
                continue;
            }
            if part == "thousand" && current > 0 {
                total += current * 1000;
                current = 0;
                consumed_this = true;
                consumed_any = true;
                continue;
            }
            if !consumed_this {
                return if consumed_any {
                    Some((total + current, idx - start_idx))
                } else {
                    None
                };
            }
        }
        if !consumed_this {
            break;
        }
        idx += 1;
    }

    if consumed_any {
        Some((total + current, idx - start_idx))
    } else {
        None
    }
}

fn numeral_unit_value(token: &str) -> Option<i64> {
    match token {
        "zero" => Some(0),
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
        "thirteen" => Some(13),
        "fourteen" => Some(14),
        "fifteen" => Some(15),
        "sixteen" => Some(16),
        "seventeen" => Some(17),
        "eighteen" => Some(18),
        "nineteen" => Some(19),
        _ => None,
    }
}

fn numeral_tens_value(token: &str) -> Option<i64> {
    match token {
        "twenty" => Some(20),
        "thirty" => Some(30),
        "forty" => Some(40),
        "fifty" => Some(50),
        "sixty" => Some(60),
        "seventy" => Some(70),
        "eighty" => Some(80),
        "ninety" => Some(90),
        _ => None,
    }
}

fn validate_number(
    prose: &str,
    tokens: &[Token],
    fact: &CanonicalFactForValidation,
    expected: &CanonicalValue,
    alias_hits: &[(usize, usize)],
    window: usize,
    violations: &mut Vec<CanonicalFactViolation>,
) {
    let Some(expected_value) = fact.value_number else {
        return;
    };

    for (alias_start, alias_end) in alias_hits {
        let start = alias_start.saturating_sub(window);
        let end = (*alias_end + window + 1).min(tokens.len());
        let slice = &tokens[start..end];
        let mut observed_number: Option<(f64, Range<usize>)> = None;

        for token in slice {
            if let Some(number) = parse_numeric_candidate(&token.normalized) {
                observed_number = Some((number, token.start..token.end));
                if (number - expected_value).abs() < f64::EPSILON {
                    observed_number = None;
                    break;
                }
            }
        }
        if observed_number.is_none() {
            let words = slice
                .iter()
                .map(|token| token.normalized.clone())
                .collect::<Vec<_>>();
            for idx in 0..words.len() {
                if let Some((value, consumed)) = parse_written_numeral_tokens(&words, idx) {
                    if consumed == 0 {
                        continue;
                    }
                    let range_start = slice[idx].start;
                    let range_end = slice[(idx + consumed - 1).min(slice.len() - 1)].end;
                    let parsed = value as f64;
                    if (parsed - expected_value).abs() < f64::EPSILON {
                        observed_number = None;
                        break;
                    }
                    observed_number = Some((parsed, range_start..range_end));
                }
            }
        }

        if let Some((observed, byte_range)) = observed_number {
            violations.push(CanonicalFactViolation {
                fact_id: fact.canonical_fact_id.clone(),
                predicate: fact.predicate.clone(),
                expected: expected.clone(),
                observed: observed.to_string(),
                byte_range,
                severity: ViolationSeverity::Hard,
                message: format!(
                    "numeric drift for '{}' expected {} but observed {}",
                    fact.predicate, expected_value, observed
                ),
            });
            break;
        }
    }

    let _ = prose;
}

fn validate_date(
    _prose: &str,
    tokens: &[Token],
    fact: &CanonicalFactForValidation,
    expected: &CanonicalValue,
    alias_hits: &[(usize, usize)],
    window: usize,
    violations: &mut Vec<CanonicalFactViolation>,
) {
    let Some(expected_year) = date_expected_from_fact(fact).map(|placement| placement.book_number)
    else {
        return;
    };

    for (alias_start, alias_end) in alias_hits {
        let start = alias_start.saturating_sub(window);
        let end = (*alias_end + window + 1).min(tokens.len());
        let slice = &tokens[start..end];
        for token in slice {
            let Some(year) = extract_year(&token.normalized) else {
                continue;
            };
            if year != expected_year {
                violations.push(CanonicalFactViolation {
                    fact_id: fact.canonical_fact_id.clone(),
                    predicate: fact.predicate.clone(),
                    expected: expected.clone(),
                    observed: year.to_string(),
                    byte_range: token.start..token.end,
                    severity: ViolationSeverity::Hard,
                    message: format!(
                        "date drift for '{}' expected year {} but observed {}",
                        fact.predicate, expected_year, year
                    ),
                });
                return;
            }
        }
    }
}

fn validate_string(
    prose: &str,
    tokens: &[Token],
    fact: &CanonicalFactForValidation,
    expected: &CanonicalValue,
    alias_hits: &[(usize, usize)],
    violations: &mut Vec<CanonicalFactViolation>,
    is_boolean: bool,
) {
    let expected_text = if is_boolean {
        match fact
            .value_json
            .as_ref()
            .and_then(Value::as_bool)
            .or_else(|| fact.value_text.as_deref().and_then(parse_bool))
        {
            Some(true) => "true".to_string(),
            Some(false) => "false".to_string(),
            None => return,
        }
    } else {
        fact.value_text.clone().unwrap_or_default().to_lowercase()
    };
    if expected_text.is_empty() {
        return;
    }

    let contrast_terms = if is_boolean {
        if expected_text == "true" {
            vec![
                "false".to_string(),
                "dead".to_string(),
                "absent".to_string(),
            ]
        } else {
            vec![
                "true".to_string(),
                "alive".to_string(),
                "present".to_string(),
            ]
        }
    } else if fact.value_kind == "enum" {
        fact.value_json
            .as_ref()
            .and_then(|json| json.get("choices"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(|value| value.to_lowercase())
                    .filter(|value| value != &expected_text)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        string_contrast_terms(fact, &expected_text)
    };

    let expected_normalized = normalize_text_for_contains(&expected_text);
    let expected_normalized = expected_normalized.trim().to_string();

    for (alias_start, alias_end) in alias_hits {
        if *alias_start >= tokens.len() || *alias_end >= tokens.len() || *alias_start > *alias_end {
            continue;
        }
        let alias_range = tokens[*alias_start].start..tokens[*alias_end].end;
        let sentence = sentence_bounds(prose, alias_range.start);
        let sentence_text = normalize_text_for_contains(&prose[sentence.clone()]);
        let sentence_trimmed = sentence_text.trim();
        for contrast in &contrast_terms {
            let normalized = normalize_text_for_contains(contrast);
            if normalized.trim().is_empty() {
                continue;
            }
            if let Some(relative_idx) = sentence_text.find(normalized.trim()) {
                violations.push(CanonicalFactViolation {
                    fact_id: fact.canonical_fact_id.clone(),
                    predicate: fact.predicate.clone(),
                    expected: expected.clone(),
                    observed: contrast.clone(),
                    byte_range: (sentence.start + relative_idx)
                        ..(sentence.start + relative_idx + normalized.trim().len()),
                    severity: ViolationSeverity::Hard,
                    message: format!(
                        "text drift for '{}' expected '{}' but observed '{}'",
                        fact.predicate, expected_text, contrast
                    ),
                });
                return;
            }
        }

        if is_boolean || fact.value_kind == "enum" || expected_normalized.is_empty() {
            continue;
        }

        if sentence_trimmed.contains(expected_normalized.as_str()) {
            continue;
        }

        let cue_terms = predicate_cue_terms(&fact.predicate);
        if let Some(observed) =
            extract_generic_string_observed(sentence_trimmed, &expected_normalized, &cue_terms)
        {
            violations.push(CanonicalFactViolation {
                fact_id: fact.canonical_fact_id.clone(),
                predicate: fact.predicate.clone(),
                expected: expected.clone(),
                observed: observed.clone(),
                // Normalization can shift byte offsets for non-ASCII; keep this span coarse.
                byte_range: sentence.start..sentence.end,
                severity: ViolationSeverity::Hard,
                message: format!(
                    "text drift for '{}' expected '{}' but observed '{}'",
                    fact.predicate, expected_text, observed
                ),
            });
            return;
        }
    }
}

fn validate_enum(
    prose: &str,
    tokens: &[Token],
    fact: &CanonicalFactForValidation,
    expected: &CanonicalValue,
    alias_hits: &[(usize, usize)],
    violations: &mut Vec<CanonicalFactViolation>,
    boolean_mode: bool,
) {
    validate_string(
        prose,
        tokens,
        fact,
        expected,
        alias_hits,
        violations,
        boolean_mode,
    );
}

fn validate_range(
    prose: &str,
    tokens: &[Token],
    fact: &CanonicalFactForValidation,
    expected: &CanonicalValue,
    alias_hits: &[(usize, usize)],
    window: usize,
    violations: &mut Vec<CanonicalFactViolation>,
) {
    let min = fact
        .value_json
        .as_ref()
        .and_then(|json| json.get("min"))
        .and_then(Value::as_f64);
    let max = fact
        .value_json
        .as_ref()
        .and_then(|json| json.get("max"))
        .and_then(Value::as_f64);
    let (Some(min), Some(max)) = (min, max) else {
        return;
    };
    for (alias_start, alias_end) in alias_hits {
        let start = alias_start.saturating_sub(window);
        let end = (*alias_end + window + 1).min(tokens.len());
        for token in &tokens[start..end] {
            let Some(number) = parse_numeric_candidate(&token.normalized) else {
                continue;
            };
            if number < min || number > max {
                violations.push(CanonicalFactViolation {
                    fact_id: fact.canonical_fact_id.clone(),
                    predicate: fact.predicate.clone(),
                    expected: expected.clone(),
                    observed: number.to_string(),
                    byte_range: token.start..token.end,
                    severity: ViolationSeverity::Hard,
                    message: format!(
                        "range violation for '{}' expected {}..{} but observed {}",
                        fact.predicate, min, max, number
                    ),
                });
                return;
            }
        }
    }
    let _ = prose;
}

fn validate_list(
    prose: &str,
    tokens: &[Token],
    fact: &CanonicalFactForValidation,
    expected: &CanonicalValue,
    alias_hits: &[(usize, usize)],
    window: usize,
    violations: &mut Vec<CanonicalFactViolation>,
) {
    let required = fact
        .value_json
        .as_ref()
        .and_then(|json| json.get("required"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let forbidden = fact
        .value_json
        .as_ref()
        .and_then(|json| json.get("forbidden"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let windows = alias_hits
        .iter()
        .filter_map(|(alias_start, alias_end)| {
            if *alias_start >= tokens.len()
                || *alias_end >= tokens.len()
                || *alias_start > *alias_end
            {
                return None;
            }
            let start = alias_start.saturating_sub(window);
            let end = (*alias_end + window + 1).min(tokens.len());
            (start < end).then_some((start, end))
        })
        .collect::<Vec<_>>();

    if windows.is_empty() {
        return;
    }

    for item in required {
        let needle = normalize_text_for_contains(&item);
        let needle = needle.trim();
        if needle.is_empty() {
            continue;
        }
        let found = windows.iter().any(|(start, end)| {
            let local =
                normalize_text_for_contains(&prose[tokens[*start].start..tokens[end - 1].end]);
            local.contains(needle)
        });
        if !found {
            violations.push(CanonicalFactViolation {
                fact_id: fact.canonical_fact_id.clone(),
                predicate: fact.predicate.clone(),
                expected: expected.clone(),
                observed: format!("missing '{}'", item),
                byte_range: 0..0,
                severity: ViolationSeverity::Soft,
                message: format!(
                    "list requirement '{}' missing for '{}'",
                    item, fact.predicate
                ),
            });
        }
    }
    for item in forbidden {
        let needle = normalize_text_for_contains(&item);
        let needle = needle.trim();
        if needle.is_empty() {
            continue;
        }
        for (start, end) in &windows {
            let local =
                normalize_text_for_contains(&prose[tokens[*start].start..tokens[end - 1].end]);
            if local.contains(needle) {
                violations.push(CanonicalFactViolation {
                    fact_id: fact.canonical_fact_id.clone(),
                    predicate: fact.predicate.clone(),
                    expected: expected.clone(),
                    observed: item.clone(),
                    // Normalization can shift byte offsets for non-ASCII; keep this span coarse.
                    byte_range: tokens[*start].start..tokens[end - 1].end,
                    severity: ViolationSeverity::Hard,
                    message: format!(
                        "forbidden list item '{}' present for '{}'",
                        item, fact.predicate
                    ),
                });
                return;
            }
        }
    }
}

fn extract_year(input: &str) -> Option<i32> {
    let cleaned = input.trim();
    let value = cleaned.parse::<i32>().ok()?;
    // This parser is tuned for common contemporary story timelines.
    if (1000..=2999).contains(&value) {
        Some(value)
    } else {
        None
    }
}

fn parse_bool(input: &str) -> Option<bool> {
    match input.to_ascii_lowercase().as_str() {
        "true" | "yes" | "alive" | "present" | "enabled" | "open" => Some(true),
        "false" | "no" | "dead" | "absent" | "disabled" | "closed" => Some(false),
        _ => None,
    }
}

fn sentence_bounds(prose: &str, anchor: usize) -> Range<usize> {
    let bytes = prose.as_bytes();
    let mut start = 0;
    for idx in (0..anchor.min(bytes.len())).rev() {
        if matches!(bytes[idx] as char, '.' | '!' | '?' | '\n') {
            start = idx + 1;
            break;
        }
    }
    let mut end = bytes.len();
    for (idx, byte) in bytes.iter().enumerate().skip(anchor.min(bytes.len())) {
        if matches!(*byte as char, '.' | '!' | '?' | '\n') {
            end = idx;
            break;
        }
    }
    start..end
}

fn string_contrast_terms(fact: &CanonicalFactForValidation, expected: &str) -> Vec<String> {
    if let Some(contrast) = fact
        .value_json
        .as_ref()
        .and_then(|json| json.get("contrast"))
        .and_then(Value::as_array)
    {
        return contrast
            .iter()
            .filter_map(Value::as_str)
            .map(|value| value.to_lowercase())
            .collect();
    }

    let hair_markers = ["hair", "hair_color", "color"];
    if hair_markers
        .iter()
        .any(|marker| fact.predicate.to_ascii_lowercase().contains(marker))
    {
        let colors = [
            "black", "blonde", "blond", "red", "brown", "grey", "gray", "white",
        ];
        return colors
            .iter()
            .map(|color| (*color).to_string())
            .filter(|color| color != expected)
            .collect();
    }
    Vec::new()
}

fn extract_generic_string_observed(
    sentence: &str,
    expected_normalized: &str,
    cue_terms: &[String],
) -> Option<String> {
    if cue_terms.is_empty() {
        return None;
    }
    let words = sentence.split_whitespace().collect::<Vec<_>>();
    for (idx, word) in words.iter().enumerate() {
        if !cue_terms.iter().any(|cue| cue == word) {
            continue;
        }
        for lookahead in 1..=3 {
            let copula_idx = idx + lookahead;
            if copula_idx + 1 >= words.len() {
                break;
            }
            if !is_copula(words[copula_idx]) {
                continue;
            }
            let observed = words[copula_idx + 1].trim();
            if observed.is_empty() || observed.contains(expected_normalized) {
                continue;
            }
            return Some(observed.to_string());
        }
    }
    None
}

fn predicate_cue_terms(predicate: &str) -> Vec<String> {
    let field = predicate
        .rsplit('.')
        .next()
        .unwrap_or(predicate)
        .to_ascii_lowercase();
    field
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| part.len() >= 3)
        .map(ToString::to_string)
        .collect()
}

fn is_copula(token: &str) -> bool {
    matches!(
        token,
        "is" | "was"
            | "are"
            | "were"
            | "become"
            | "becomes"
            | "became"
            | "remain"
            | "remains"
            | "stay"
            | "stays"
            | "hold"
            | "holds"
    )
}
