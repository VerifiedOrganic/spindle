//! Extraction helpers for model output that may carry narration around a JSON
//! payload.
//!
//! # Why this exists
//!
//! Some agent CLIs (notably `grok-cli` running grok-4.5) emit short narration
//! before and between the tool-call turns that lead up to a structured answer —
//! e.g. `"I'll pull Spindle canon first…"` — and then place the actual JSON
//! document at the tail of the transcript. Feeding that whole concatenation to a
//! JSON parser fails with `expected value at line 1 column 1`, which broke ~100%
//! of draft dispatches in a real 5-chapter run. System-prompt discipline does
//! not suppress the narration; grok narrates regardless.
//!
//! The field-proven salvage the operator applied by hand was: scan from the tail
//! and take the last *complete, balanced* top-level `{…}` object. That is what
//! [`extract_trailing_json_object`] implements. It is deliberately conservative:
//! it never returns a truncated object, and it prefers the LATEST complete
//! object because the payload reliably trails the narration.

/// Return the last balanced top-level `{…}` JSON object in `text`, if any.
///
/// "Balanced" means brace depth returns to zero, respecting JSON string literals
/// (braces inside strings do not count) and backslash escapes inside strings.
/// The scan prefers the LATEST complete object: it walks candidate `{` start
/// positions from the last `{` backwards and returns the first one that closes
/// into a balanced object whose `}` is the final structural close. A trailing
/// object that is truncated (never closes) is skipped in favor of an earlier
/// complete one; when no complete object exists, returns `None`.
///
/// The returned slice borrows from `text` and includes the outer braces.
pub fn extract_trailing_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();

    // Collect the byte offsets of every `{` that starts a string-aware object,
    // partitioned by nesting depth. `top_level_starts` holds the depth-0 opens
    // (a real JSON document is one of these); `all_starts` holds every open,
    // including nested ones, for the salvage fallback below.
    let mut top_level_starts: Vec<usize> = Vec::new();
    let mut all_starts: Vec<usize> = Vec::new();
    {
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escaped = false;
        for (idx, &b) in bytes.iter().enumerate() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_string = false;
                }
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'{' => {
                    if depth == 0 {
                        top_level_starts.push(idx);
                    }
                    all_starts.push(idx);
                    depth += 1;
                }
                b'}' if depth > 0 => depth -= 1,
                _ => {}
            }
        }
    }

    // Preferred: the LATEST complete top-level object. The payload trails the
    // narration, so scanning depth-0 opens from the last one backwards yields
    // the intended document.
    for &start in top_level_starts.iter().rev() {
        if let Some(end) = balanced_object_end(bytes, start) {
            // `end` is the index of the matching `}` (inclusive).
            return std::str::from_utf8(&bytes[start..=end]).ok();
        }
    }

    // Salvage fallback: every top-level open is truncated (e.g. an unterminated
    // narration brace that swallowed the real object). Take the LATEST complete
    // object at any nesting depth — this recovers a well-formed payload embedded
    // inside an unbalanced outer run. Still returns None when nothing closes.
    for &start in all_starts.iter().rev() {
        if let Some(end) = balanced_object_end(bytes, start) {
            return std::str::from_utf8(&bytes[start..=end]).ok();
        }
    }
    None
}

/// Given `bytes[start] == b'{'`, return the index of the matching `}` that
/// closes it into a balanced object, respecting JSON strings/escapes. Returns
/// `None` when the object never closes (truncated).
fn balanced_object_end(bytes: &[u8], start: usize) -> Option<usize> {
    debug_assert_eq!(bytes.get(start), Some(&b'{'));
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, &b) in bytes[start..].iter().enumerate() {
        let idx = start + offset;
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narration_then_json_takes_the_json() {
        let text = "I'll pull Spindle canon first, then draft.\n\n{\"full_text\":\"hi\"}";
        assert_eq!(
            extract_trailing_json_object(text),
            Some("{\"full_text\":\"hi\"}")
        );
    }

    #[test]
    fn json_then_narration_then_json_takes_the_last_complete_object() {
        let text = "{\"a\":1} some chatter about tools {\"b\":2}";
        assert_eq!(extract_trailing_json_object(text), Some("{\"b\":2}"));
    }

    #[test]
    fn braces_inside_json_strings_do_not_break_balance() {
        let text = "prose {\"k\":\"a{b}c\"} trailing";
        assert_eq!(
            extract_trailing_json_object(text),
            Some("{\"k\":\"a{b}c\"}")
        );
    }

    #[test]
    fn escaped_quotes_inside_strings_are_respected() {
        let text = "narration {\"quote\":\"she said \\\"hi\\\" then {left}\"}";
        assert_eq!(
            extract_trailing_json_object(text),
            Some("{\"quote\":\"she said \\\"hi\\\" then {left}\"}")
        );
    }

    #[test]
    fn truncated_trailing_object_falls_back_to_earlier_complete_object() {
        // First object is complete; the trailing one is truncated (never closes).
        let text = "{\"a\":1} then narration {\"b\":2, \"c\": [1,2,3";
        assert_eq!(extract_trailing_json_object(text), Some("{\"a\":1}"));
    }

    #[test]
    fn only_a_truncated_object_yields_none() {
        let text = "narration {\"b\":2, \"c\": [1,2,3";
        assert_eq!(extract_trailing_json_object(text), None);
    }

    #[test]
    fn no_json_at_all_yields_none() {
        let text = "I'll pull Spindle canon first. No JSON here at all.";
        assert_eq!(extract_trailing_json_object(text), None);
    }

    #[test]
    fn nested_object_is_returned_whole() {
        let text = "chatter {\"outer\":{\"inner\":{\"deep\":true}}}";
        assert_eq!(
            extract_trailing_json_object(text),
            Some("{\"outer\":{\"inner\":{\"deep\":true}}}")
        );
    }

    #[test]
    fn multi_fragment_grok_turn_concatenation_parses_the_trailing_object() {
        // Mimics grok concatenating narration + tool-call chatter + final JSON.
        let text = "I'll pull Spindle canon first…\n\
             Calling search_bible for the Ash Gate.\n\
             Found it. Now drafting the scene.\n\n\
             {\"full_text\":\"Mara stood watch at the Ash Gate.\",\
             \"summary\":\"Mara watch\",\"tone\":\"grim\"}";
        assert_eq!(
            extract_trailing_json_object(text),
            Some(
                "{\"full_text\":\"Mara stood watch at the Ash Gate.\",\
                 \"summary\":\"Mara watch\",\"tone\":\"grim\"}"
            )
        );
    }

    #[test]
    fn clean_json_object_is_returned_unchanged() {
        let text = "{\"ok\":true}";
        assert_eq!(extract_trailing_json_object(text), Some("{\"ok\":true}"));
    }

    #[test]
    fn complete_object_nested_inside_a_truncated_outer_is_salvaged() {
        // An unterminated narration brace swallows the tail as one big
        // never-closing top-level "object"; the real payload is complete inside
        // it and must still be recovered (the field-proven salvage).
        let text = "note: the set is {incomplete here\nNow the answer:\n{\"findings\":[]}";
        assert_eq!(
            extract_trailing_json_object(text),
            Some("{\"findings\":[]}")
        );
    }

    #[test]
    fn brace_inside_string_before_a_later_real_object() {
        // A string containing an unbalanced-looking brace must not fool the
        // top-level start scan into a bad candidate.
        let text = "note: \"the set is {incomplete\" then {\"real\":1}";
        assert_eq!(extract_trailing_json_object(text), Some("{\"real\":1}"));
    }
}
