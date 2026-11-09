//! Shared, ambiguity-safe character mention resolution.
//!
//! One resolver is built from the cast (canonical name + explicit aliases) and
//! used by knowledge-timing, secret-leak and dead-character detectors. It
//! expands each name into full/forename/surname candidates and disables any
//! candidate that could refer to more than one character.
//!
//! Pronouns are deliberately not resolved: reliable pronoun resolution needs
//! gender/number and discourse state that the deterministic detectors do not
//! have. Callers may use [`CharacterMentionResolver::single_mentioned_character`]
//! only where a scene has exactly one unambiguous named character; the current
//! detectors do not guess from pronouns.

use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterMentionRecord {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
}

/// Precompiled mention resolver for one project/branch cast.
pub struct CharacterMentionResolver {
    aliases_by_id: BTreeMap<String, Vec<String>>,
    patterns_by_id: BTreeMap<String, Vec<Regex>>,
    ambiguous_aliases: BTreeSet<String>,
    ids_by_alias: BTreeMap<String, BTreeSet<String>>,
}

impl CharacterMentionResolver {
    pub fn new(records: &[CharacterMentionRecord]) -> Self {
        let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for record in records {
            let aliases = candidate_aliases(&record.name, &record.aliases);
            candidates.entry(record.id.clone()).or_default().extend(
                aliases
                    .into_iter()
                    .map(|alias| normalize_alias(&alias))
                    .filter(|alias| !alias.is_empty()),
            );
        }

        let mut ids_by_alias: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (id, aliases) in &candidates {
            for alias in aliases {
                ids_by_alias
                    .entry(alias.clone())
                    .or_default()
                    .insert(id.clone());
            }
        }
        let ambiguous_aliases: BTreeSet<String> = ids_by_alias
            .iter()
            .filter(|(_, ids)| ids.len() > 1)
            .map(|(alias, _)| alias.clone())
            .collect();

        let mut aliases_by_id = BTreeMap::new();
        let mut patterns_by_id = BTreeMap::new();
        for (id, aliases) in candidates {
            let mut kept: Vec<String> = aliases
                .into_iter()
                .filter(|alias| !ambiguous_aliases.contains(alias))
                .collect();
            // Longest first: a full name should win over a forename when both
            // are present, though detectors only need membership.
            kept.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
            let patterns = kept
                .iter()
                .filter_map(|alias| build_alias_regex(alias))
                .collect();
            aliases_by_id.insert(id.clone(), kept);
            patterns_by_id.insert(id, patterns);
        }

        Self {
            aliases_by_id,
            patterns_by_id,
            ambiguous_aliases,
            ids_by_alias,
        }
    }

    /// Unambiguous aliases for one character, longest first.
    pub fn aliases_for(&self, character_id: &str) -> &[String] {
        self.aliases_by_id
            .get(character_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn mentions(&self, text: &str, character_id: &str) -> bool {
        self.patterns_by_id
            .get(character_id)
            .is_some_and(|patterns| {
                patterns
                    .iter()
                    .any(|pattern| pattern_has_capitalized_match(pattern, text))
            })
    }

    pub fn mentioned_ids(&self, text: &str) -> BTreeSet<String> {
        self.patterns_by_id
            .iter()
            .filter(|(_, patterns)| {
                patterns
                    .iter()
                    .any(|pattern| pattern_has_capitalized_match(pattern, text))
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Mention check that additionally accepts first-person narration when the
    /// caller has established that `pov_character_id` is this scene's single
    /// POV character. Never supplies a POV by itself.
    pub fn mentions_with_pov(
        &self,
        text: &str,
        character_id: &str,
        pov_character_id: Option<&str>,
    ) -> bool {
        self.mentions(text, character_id)
            || (pov_character_id == Some(character_id) && first_person_narration(text))
    }

    /// Nearest unambiguous character named before the end of `text`. Useful
    /// for conservative pronoun resolution: only the most recent antecedent is
    /// considered, never a guess across unrelated names.
    pub fn last_mentioned_character(&self, text: &str) -> Option<String> {
        let mut best: Option<(usize, String)> = None;
        for (id, patterns) in &self.patterns_by_id {
            for pattern in patterns {
                if let Some(found) = pattern
                    .find_iter(text)
                    .filter(|found| starts_with_uppercase(found.as_str()))
                    .last()
                {
                    let end = found.end();
                    if best.as_ref().is_none_or(|(best_end, _)| end > *best_end) {
                        best = Some((end, id.clone()));
                    }
                }
            }
        }
        best.map(|(_, id)| id)
    }

    /// Exactly one unambiguous named character appears in `text`, if any.
    /// Useful only for callers that have separately established that a pronoun
    /// cannot refer to an unnamed person; detectors do not guess from this.
    pub fn single_mentioned_character(&self, text: &str) -> Option<String> {
        let ids = self.mentioned_ids(text);
        if ids.len() == 1 {
            ids.into_iter().next()
        } else {
            None
        }
    }

    pub fn is_ambiguous(&self, alias: &str) -> bool {
        self.ambiguous_aliases.contains(&normalize_alias(alias))
    }

    pub fn alias_owner_count(&self, alias: &str) -> usize {
        self.ids_by_alias
            .get(&normalize_alias(alias))
            .map(BTreeSet::len)
            .unwrap_or(0)
    }
}

fn candidate_aliases(name: &str, explicit_aliases: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let trimmed_name = name.trim();
    if !trimmed_name.is_empty() {
        out.push(trimmed_name.to_string());
    }
    for alias in explicit_aliases {
        let alias = alias.trim();
        if alias.len() >= 2 {
            out.push(alias.to_string());
        }
    }
    let parts: Vec<&str> = trimmed_name.split_whitespace().collect();
    if parts.len() >= 2 {
        let first = parts[0];
        let last = parts[parts.len() - 1];
        if usable_name_part(first) {
            out.push(first.to_string());
        }
        if usable_name_part(last) && !last.eq_ignore_ascii_case(first) {
            out.push(last.to_string());
        }
    }
    out
}

fn usable_name_part(part: &str) -> bool {
    let lower = part
        .trim_matches(|ch: char| !ch.is_alphanumeric())
        .to_ascii_lowercase();
    lower.len() >= 3
        && !matches!(
            lower.as_str(),
            "the"
                | "captain"
                | "commander"
                | "doctor"
                | "dr"
                | "lady"
                | "lord"
                | "sir"
                | "dame"
                | "mr"
                | "mrs"
                | "ms"
                | "miss"
                | "professor"
                | "prof"
                | "general"
                | "admiral"
                | "sergeant"
                | "lieutenant"
                | "de"
                | "del"
                | "da"
                | "van"
                | "von"
                | "la"
                | "le"
                | "of"
        )
}

fn normalize_alias(alias: &str) -> String {
    alias
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn starts_with_uppercase(text: &str) -> bool {
    text.chars().next().is_some_and(char::is_uppercase)
}

fn pattern_has_capitalized_match(pattern: &Regex, text: &str) -> bool {
    pattern
        .find_iter(text)
        .any(|found| starts_with_uppercase(found.as_str()))
}

fn build_alias_regex(alias: &str) -> Option<Regex> {
    let escaped = regex::escape(alias);
    regex::RegexBuilder::new(&format!(r"(?i)\b{escaped}\b"))
        .size_limit(1 << 18)
        .build()
        .ok()
}

/// True when unquoted prose contains a first-person narrator token
/// (`I`, `me`, `my`, `mine`, `myself`). Quoted dialogue is ignored so a
/// character saying "I" does not make the scene first-person.
pub fn first_person_narration(text: &str) -> bool {
    let mut unquoted = String::with_capacity(text.len());
    let mut in_quote = false;
    for ch in text.chars() {
        match ch {
            '"' | '“' | '”' => in_quote = !in_quote,
            _ if !in_quote => unquoted.push(ch),
            _ => {}
        }
    }
    unquoted
        .split(|ch: char| !ch.is_alphanumeric() && ch != '\'')
        .filter(|token| !token.is_empty())
        .any(|token| {
            matches!(
                token.trim_matches('\'').to_ascii_lowercase().as_str(),
                "i" | "me" | "my" | "mine" | "myself"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, name: &str, aliases: &[&str]) -> CharacterMentionRecord {
        CharacterMentionRecord {
            id: id.into(),
            name: name.into(),
            aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
        }
    }

    #[test]
    fn resolves_full_forename_surname_and_cast_nickname() {
        let resolver = CharacterMentionResolver::new(&[record(
            "character:sela",
            "Sela Voss",
            &["The Kestrel", "Sel"],
        )]);
        assert!(resolver.mentions("Sela Voss crossed the yard.", "character:sela"));
        assert!(resolver.mentions("Sela crossed the yard.", "character:sela"));
        assert!(resolver.mentions("Voss crossed the yard.", "character:sela"));
        assert!(resolver.mentions("The Kestrel crossed the yard.", "character:sela"));
        assert!(resolver.mentions("Sel crossed the yard.", "character:sela"));
        assert!(!resolver.mentions("Selene crossed the yard.", "character:sela"));
        assert!(!resolver.mentions("the bell rang.", "character:sela"));
    }

    #[test]
    fn shared_forename_is_ambiguous_but_unique_full_names_still_work() {
        let resolver = CharacterMentionResolver::new(&[
            record("character:a", "Sela Voss", &[]),
            record("character:b", "Sela Thorn", &[]),
        ]);
        assert!(resolver.is_ambiguous("Sela"));
        assert!(!resolver.mentions("Sela waited.", "character:a"));
        assert!(!resolver.mentions("Sela waited.", "character:b"));
        assert!(resolver.mentions("Sela Voss waited.", "character:a"));
        assert!(resolver.mentions("Sela Thorn waited.", "character:b"));
        assert!(resolver.mentions("Voss waited.", "character:a"));
        assert!(resolver.mentions("Thorn waited.", "character:b"));
    }

    #[test]
    fn explicit_alias_collision_is_ambiguous() {
        let resolver = CharacterMentionResolver::new(&[
            record("character:a", "Mara Vale", &["Wren"]),
            record("character:b", "Tomas Reed", &["Wren"]),
        ]);
        assert!(resolver.is_ambiguous("Wren"));
        assert!(!resolver.mentions("Wren waited.", "character:a"));
        assert!(!resolver.mentions("Wren waited.", "character:b"));
        assert_eq!(resolver.alias_owner_count("Wren"), 2);
    }

    #[test]
    fn titles_and_particles_are_not_forenames() {
        let resolver = CharacterMentionResolver::new(&[
            record("character:a", "Captain Sela Voss", &[]),
            record("character:b", "Lucia van der Meer", &[]),
        ]);
        assert!(!resolver.is_ambiguous("Captain"));
        assert!(!resolver.mentions("The captain nodded.", "character:a"));
        assert!(!resolver.is_ambiguous("van"));
        assert!(resolver.mentions("Lucia waited.", "character:b"));
        assert!(resolver.mentions("Meer waited.", "character:b"));
    }

    #[test]
    fn pronoun_is_not_guessed_from_a_single_name() {
        let resolver = CharacterMentionResolver::new(&[record("character:a", "Sela Voss", &[])]);
        let text = "Sela waited. She checked the lock.";
        assert_eq!(
            resolver.single_mentioned_character(text),
            Some("character:a".to_string())
        );
        assert!(!resolver.mentions("She checked the lock.", "character:a"));
    }

    #[test]
    fn first_person_is_unquoted_narration_only() {
        let resolver =
            CharacterMentionResolver::new(&[record("character:tam", "Tamsin Okoro", &["Tam"])]);
        assert!(resolver.mentions_with_pov(
            "I write the date in the ledger.",
            "character:tam",
            Some("character:tam"),
        ));
        assert!(!resolver.mentions_with_pov(
            "I write the date in the ledger.",
            "character:tam",
            None,
        ));
        assert!(!resolver.mentions_with_pov(
            "\"I write the date,\" the clerk said.",
            "character:tam",
            Some("character:tam"),
        ));
        assert!(!first_person_narration("Rook said, \"I saw the crack.\""));
        assert!(first_person_narration(
            "Rook said nothing; my own hands were shaking."
        ));
    }
}
