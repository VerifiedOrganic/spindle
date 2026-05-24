use crate::models::{CharacterVoiceProfileData, TextByteRange, VoiceDriftFinding, VoiceDriftKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceDriftCharacter {
    pub id: String,
    pub name: String,
}

pub fn character_present_in_scene(scene_text: &str, character: &VoiceDriftCharacter) -> bool {
    character_aliases(&character.name)
        .into_iter()
        .any(|alias| contains_alias(scene_text, &alias))
}

pub fn check_voice_drift(
    scene_text: &str,
    character: &VoiceDriftCharacter,
    voice_profile: &CharacterVoiceProfileData,
) -> Vec<VoiceDriftFinding> {
    if !character_present_in_scene(scene_text, character) {
        return Vec::new();
    }

    let dialogue_ranges = attributed_dialogue_ranges(scene_text, character);
    if dialogue_ranges.is_empty() {
        // We only emit character-specific findings when dialogue is attributable.
        return Vec::new();
    }

    let mut findings = Vec::new();
    for phrase in voice_profile
        .forbidden_words
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
    {
        let Some(regex) = build_case_insensitive_literal_regex(phrase) else {
            continue;
        };
        for quote in &dialogue_ranges {
            if quote.end <= quote.start || quote.end > scene_text.len() {
                continue;
            }
            let excerpt = &scene_text[quote.start..quote.end];
            for mat in regex.find_iter(excerpt) {
                findings.push(VoiceDriftFinding {
                    character_id: character.id.clone(),
                    character_name: character.name.clone(),
                    kind: VoiceDriftKind::ForbiddenPhrase,
                    message: format!(
                        "Character '{}' uses forbidden phrase '{}'.",
                        character.name, phrase
                    ),
                    forbidden_phrase: Some(phrase.to_string()),
                    matched_text: Some(excerpt[mat.start()..mat.end()].to_string()),
                    byte_range: Some(TextByteRange {
                        start: quote.start + mat.start(),
                        end: quote.start + mat.end(),
                    }),
                });
            }
        }
    }
    findings
}

fn attributed_dialogue_ranges(
    scene_text: &str,
    character: &VoiceDriftCharacter,
) -> Vec<TextByteRange> {
    let aliases = character_aliases(&character.name);
    let mut ranges = Vec::new();
    collect_attributed_ranges(scene_text, &aliases, r#""([^"]+)""#, &mut ranges);
    collect_attributed_ranges(scene_text, &aliases, r#"“([^”]+)”"#, &mut ranges);
    ranges
}

fn collect_attributed_ranges(
    scene_text: &str,
    aliases: &[String],
    quote_pattern: &str,
    out: &mut Vec<TextByteRange>,
) {
    let Some(quote_re) = regex::Regex::new(quote_pattern).ok() else {
        return;
    };
    for caps in quote_re.captures_iter(scene_text) {
        let (Some(full_match), Some(inner_quote)) = (caps.get(0), caps.get(1)) else {
            continue;
        };
        if is_quote_attributed_to_alias(scene_text, aliases, full_match.start(), full_match.end()) {
            out.push(TextByteRange {
                start: inner_quote.start(),
                end: inner_quote.end(),
            });
        }
    }
}

fn is_quote_attributed_to_alias(
    scene_text: &str,
    aliases: &[String],
    full_quote_start: usize,
    full_quote_end: usize,
) -> bool {
    // Clip the attribution windows so they cannot reach into adjacent
    // quotes. Without this, a later "<NAME> replied" attached to a
    // different quote could wrongly attribute this quote — e.g. given
    // `"<A swears>" Sam said. "<B replies>" CLAUDIA replied.`, the after
    // window for the first quote contains "CLAUDIA replied", which would
    // otherwise attribute Sam's line to CLAUDIA. The full_quote_start /
    // full_quote_end positions include the surrounding quote characters
    // so we don't mistake this very quote's opening or closing mark for an
    // adjacent quote.
    let before_start_raw = full_quote_start.saturating_sub(140);
    let before_start = previous_quote_boundary(scene_text, full_quote_start)
        .map(|boundary| boundary.max(before_start_raw))
        .unwrap_or(before_start_raw);
    let before_start = ceil_char_boundary(scene_text, before_start);

    let after_end_raw = full_quote_end.saturating_add(140).min(scene_text.len());
    let after_end = next_quote_boundary(scene_text, full_quote_end)
        .map(|boundary| boundary.min(after_end_raw))
        .unwrap_or(after_end_raw);
    let after_end = floor_char_boundary(scene_text, after_end);

    let Some(before) = scene_text
        .get(before_start..full_quote_start)
        .map(str::to_ascii_lowercase)
    else {
        return false;
    };
    let Some(after) = scene_text
        .get(full_quote_end..after_end)
        .map(str::to_ascii_lowercase)
    else {
        return false;
    };
    let speech_verbs = [
        "said",
        "asked",
        "replied",
        "muttered",
        "whispered",
        "shouted",
        "snapped",
        "added",
        "called",
    ];

    aliases.iter().any(|alias| {
        let alias = alias.to_ascii_lowercase();
        speech_verbs.iter().any(|verb| {
            window_has_alias_verb_pair(&before, &alias, verb)
                || window_has_alias_verb_pair(&after, &alias, verb)
        })
    })
}

/// Find the byte offset of the first quote-character boundary strictly
/// before `pos`, i.e. the position just after the previous quote ends.
/// Returns `None` if there is no quote before `pos`.
fn previous_quote_boundary(text: &str, pos: usize) -> Option<usize> {
    let prefix = &text[..pos];
    let mut last: Option<usize> = None;
    for (idx, ch) in prefix.char_indices() {
        if ch == '"' || ch == '“' || ch == '”' {
            last = Some(idx + ch.len_utf8());
        }
    }
    last
}

/// Find the byte offset of the next quote character at or after `pos`.
/// Returns `None` if there is no quote at or after `pos`.
fn next_quote_boundary(text: &str, pos: usize) -> Option<usize> {
    if pos >= text.len() {
        return None;
    }
    text[pos..].char_indices().find_map(|(idx, ch)| {
        if ch == '"' || ch == '“' || ch == '”' {
            Some(pos + idx)
        } else {
            None
        }
    })
}

fn window_has_alias_verb_pair(window: &str, alias: &str, verb: &str) -> bool {
    let alias_hits = window.match_indices(alias).collect::<Vec<_>>();
    let verb_hits = window.match_indices(verb).collect::<Vec<_>>();
    for (alias_pos, _) in &alias_hits {
        for (verb_pos, _) in &verb_hits {
            let (left_end, right_start) = if alias_pos <= verb_pos {
                (alias_pos + alias.len(), *verb_pos)
            } else {
                (verb_pos + verb.len(), *alias_pos)
            };
            let gap = right_start.saturating_sub(left_end);
            if gap > 32 {
                continue;
            }
            let between = &window[left_end..right_start];
            if !between.contains('.') && !between.contains('!') && !between.contains('?') {
                return true;
            }
        }
    }
    false
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn character_aliases(name: &str) -> Vec<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut aliases = vec![trimmed.to_string()];
    if let Some(last) = trimmed.split_whitespace().last()
        && last.len() >= 3
        && !aliases
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(last))
    {
        aliases.push(last.to_string());
    }
    aliases
}

fn contains_alias(text: &str, alias: &str) -> bool {
    let escaped = regex::escape(alias);
    regex::RegexBuilder::new(&format!(r"(?i)\b{escaped}\b"))
        .size_limit(1 << 16)
        .build()
        .is_ok_and(|re| re.is_match(text))
}

fn build_case_insensitive_literal_regex(phrase: &str) -> Option<regex::Regex> {
    regex::RegexBuilder::new(&regex::escape(phrase))
        .case_insensitive(true)
        .size_limit(1 << 16)
        .build()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jim_dalton_character() -> VoiceDriftCharacter {
        VoiceDriftCharacter {
            id: "character:jim_dalton".to_string(),
            name: "Jim Dalton".to_string(),
        }
    }

    #[test]
    fn forbidden_phrase_regression_fixture_jim_dalton_voice() {
        let profile = CharacterVoiceProfileData {
            tone: Some("plainspoken".to_string()),
            vocabulary: vec!["dock".to_string(), "ledger".to_string()],
            sentence_structure: vec!["short declarative".to_string()],
            tics: vec!["keeps answers clipped".to_string()],
            forbidden_words: vec!["as you know".to_string()],
            example_lines: vec!["Keep it clean and keep it moving.".to_string()],
            established_in_scene_id: None,
            updated_at: None,
        };
        let scene_text = "Jim Dalton said, \"As you know, the ledgers are already burned.\" and tapped the table.";
        let findings = check_voice_drift(scene_text, &jim_dalton_character(), &profile);
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.kind, VoiceDriftKind::ForbiddenPhrase);
        assert_eq!(finding.character_id, "character:jim_dalton");
        assert_eq!(finding.forbidden_phrase.as_deref(), Some("as you know"));
        assert_eq!(finding.matched_text.as_deref(), Some("As you know"));
    }

    #[test]
    fn forbidden_phrase_handles_multibyte_boundary_in_pre_quote_window() {
        let profile = CharacterVoiceProfileData {
            tone: Some("plainspoken".to_string()),
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec!["as you know".to_string()],
            example_lines: vec![],
            established_in_scene_id: None,
            updated_at: None,
        };
        let scene_text = format!("a—{}\"As you know,\" Jim Dalton said.", "b".repeat(138));

        let findings = check_voice_drift(&scene_text, &jim_dalton_character(), &profile);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].matched_text.as_deref(), Some("As you know"));
    }

    #[test]
    fn forbidden_phrase_handles_multibyte_boundary_in_post_quote_window() {
        let profile = CharacterVoiceProfileData {
            tone: Some("plainspoken".to_string()),
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec!["as you know".to_string()],
            example_lines: vec![],
            established_in_scene_id: None,
            updated_at: None,
        };
        let scene_text = format!("\"As you know,\" Jim Dalton said. {}—z", "b".repeat(139));

        let findings = check_voice_drift(&scene_text, &jim_dalton_character(), &profile);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].matched_text.as_deref(), Some("As you know"));
    }

    #[test]
    fn no_findings_when_forbidden_phrase_is_absent() {
        let profile = CharacterVoiceProfileData {
            tone: None,
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec!["as you know".to_string()],
            example_lines: vec![],
            established_in_scene_id: None,
            updated_at: None,
        };
        let scene_text = "Jim Dalton said, \"Keep it moving.\"";
        let findings = check_voice_drift(scene_text, &jim_dalton_character(), &profile);
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_misattribute_phrase_spoken_by_another_character() {
        let profile = CharacterVoiceProfileData {
            tone: None,
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec!["as you know".to_string()],
            example_lines: vec![],
            established_in_scene_id: None,
            updated_at: None,
        };
        let scene_text =
            "Jim Dalton watched the door while Sara Vale said, \"As you know, the lock was cut.\"";
        let findings = check_voice_drift(scene_text, &jim_dalton_character(), &profile);
        assert!(findings.is_empty());
    }

    #[test]
    fn recognizes_trailing_attribution_quote_then_name_verb() {
        let profile = CharacterVoiceProfileData {
            tone: None,
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec!["as you know".to_string()],
            example_lines: vec![],
            established_in_scene_id: None,
            updated_at: None,
        };
        let scene_text = "\"As you know, the lock was cut,\" Jim Dalton said.";
        let findings = check_voice_drift(scene_text, &jim_dalton_character(), &profile);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn recognizes_leading_attribution_said_name_then_quote() {
        let profile = CharacterVoiceProfileData {
            tone: None,
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec!["as you know".to_string()],
            example_lines: vec![],
            established_in_scene_id: None,
            updated_at: None,
        };
        let scene_text = "Said Jim Dalton, \"As you know, the lock was cut.\"";
        let findings = check_voice_drift(scene_text, &jim_dalton_character(), &profile);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn supports_curly_quotes_for_attributed_dialogue() {
        let profile = CharacterVoiceProfileData {
            tone: None,
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec!["as you know".to_string()],
            example_lines: vec![],
            established_in_scene_id: None,
            updated_at: None,
        };
        let scene_text = "Jim Dalton said, “As you know, the lock was cut.”";
        let findings = check_voice_drift(scene_text, &jim_dalton_character(), &profile);
        assert_eq!(findings.len(), 1);
    }
}
