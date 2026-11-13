//! Term matching for the project-local research library.
//!
//! Queries and tags are free text. Authors type `supply-lines`,
//! `supply_lines`, or `supply lines` for the same topic, so matching folds
//! case, treats hyphens/underscores and other punctuation as word separators,
//! and compares normalized terms. Apostrophes are dropped inside a word so
//! decade spellings such as `70's` become `70s`.
//!
//! A multi-word query is split into terms. Search keeps an item when it
//! matches any term and ranks it by how many distinct terms it matches.

use std::collections::HashSet;

/// Split free text into normalized, deduplicated terms.
///
/// Terms are lowercase alphanumeric runs. Hyphens, underscores, spaces and
/// other punctuation separate terms; apostrophes are ignored so decade and
/// contraction spellings stay together (`70's` becomes `70s`).
pub fn research_terms(input: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for c in input.chars() {
        if c.is_alphanumeric() {
            current.extend(c.to_lowercase());
        } else if c == '\'' || c == '\u{2019}' {
            continue;
        } else if !current.is_empty() {
            terms.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        terms.push(current);
    }

    let mut seen = HashSet::new();
    terms.retain(|term| seen.insert(term.clone()));
    terms
}

/// Normalized, space-separated form of free text.
pub fn normalize_research_text(input: &str) -> String {
    research_terms(input).join(" ")
}

/// Number of distinct query terms that occur in any of `fields` or `tags`.
///
/// Matching is case-insensitive and punctuation-insensitive. Text fields and
/// tags are both normalized to term text before the check, so a query term
/// `dealers` matches a tag written `slave-dealers`.
pub fn research_match_score(terms: &[String], fields: &[&str], tags: &[String]) -> usize {
    if terms.is_empty() {
        return 0;
    }
    let haystack = fields
        .iter()
        .copied()
        .chain(tags.iter().map(String::as_str))
        .map(normalize_research_text)
        .collect::<Vec<_>>()
        .join(" ");

    terms
        .iter()
        .filter(|term| !term.is_empty() && haystack.contains(term.as_str()))
        .count()
}

/// True when at least one term of `wanted` occurs in any `available` tag.
pub fn research_tag_matches(wanted: &str, available: &[String]) -> bool {
    let wanted_terms = research_terms(wanted);
    if wanted_terms.is_empty() {
        return false;
    }
    available.iter().any(|tag| {
        let tag_terms = research_terms(tag);
        wanted_terms
            .iter()
            .any(|term| tag_terms.iter().any(|candidate| candidate == term))
    })
}

/// True when any `wanted` tag overlaps any `available` tag.
pub fn research_tags_overlap(wanted: &[String], available: &[String]) -> bool {
    wanted
        .iter()
        .any(|tag| research_tag_matches(tag, available))
}

/// True when one `available` tag carries every term of `wanted`.
///
/// This is the stricter comparison used for the optional tag filter: a filter
/// such as `supply lines` matches a stored `supply-lines` tag, but a tag that
/// only carries one of the words does not satisfy the whole filter.
pub fn research_tag_filter_matches(wanted: &str, available: &[String]) -> bool {
    let wanted_terms = research_terms(wanted);
    if wanted_terms.is_empty() {
        return true;
    }
    available.iter().any(|tag| {
        let tag_terms = research_terms(tag);
        wanted_terms
            .iter()
            .all(|term| tag_terms.iter().any(|candidate| candidate == term))
    })
}

/// True when every filter tag matches at least one available tag.
pub fn research_tag_filters_match(filters: &[String], available: &[String]) -> bool {
    filters
        .iter()
        .all(|filter| research_tag_filter_matches(filter, available))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_split_hyphens_underscores_and_spaces() {
        assert_eq!(
            research_terms("Supply-Lines"),
            vec!["supply".to_string(), "lines".to_string()]
        );
        assert_eq!(
            research_terms("supply_lines"),
            vec!["supply".to_string(), "lines".to_string()]
        );
        assert_eq!(
            research_terms("  supply   lines  "),
            vec!["supply".to_string(), "lines".to_string()]
        );
    }

    #[test]
    fn terms_fold_case_and_keep_decades_together() {
        assert_eq!(research_terms("70's"), vec!["70s".to_string()]);
        assert_eq!(
            research_terms("Camp FOLLOWERS"),
            vec!["camp".to_string(), "followers".to_string()]
        );
    }

    #[test]
    fn score_counts_distinct_matching_terms_across_fields_and_tags() {
        let terms = research_terms("army camp");
        assert_eq!(
            research_match_score(
                &terms,
                &["The column pitched tents beside the river."],
                &["camp".to_string()]
            ),
            1
        );
        assert_eq!(
            research_match_score(&terms, &["The army camped beside the river."], &[]),
            2
        );
        assert_eq!(
            research_match_score(&terms, &["A mounted patrol crossed at dawn."], &[]),
            0
        );
    }

    #[test]
    fn tag_matching_normalizes_hyphens_underscores_and_case() {
        let stored = vec!["Supply-Lines".to_string()];
        assert!(research_tag_matches("supply lines", &stored));
        assert!(research_tag_matches("SUPPLY_LINES", &stored));
        assert!(research_tag_matches("lines", &stored));
        assert!(!research_tag_matches("winter quarters", &stored));
    }

    #[test]
    fn tag_overlap_accepts_partial_multi_word_tags() {
        let wanted = vec!["army camp".to_string(), "winter quarters".to_string()];
        let available = vec![
            "camp".to_string(),
            "army".to_string(),
            "supply-lines".to_string(),
        ];
        assert!(research_tags_overlap(&wanted, &available));
    }

    #[test]
    fn tag_filters_require_every_term_in_one_tag() {
        let stored = vec!["supply-lines".to_string(), "camp".to_string()];
        assert!(research_tag_filters_match(
            &["supply lines".to_string()],
            &stored
        ));
        assert!(research_tag_filters_match(&["CAMP".to_string()], &stored));
        assert!(!research_tag_filters_match(
            &["supply camp".to_string()],
            &stored
        ));
    }
}
