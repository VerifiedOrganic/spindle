//! Mention → cluster consolidator.
//!
//! Pure logic: groups `ImportEntityMention` rows by `(entity_kind,
//! normalized_name)` and emits canonical clusters the service layer then
//! routes through `Repository::upsert_import_entity_cluster`. Ported from
//! the SurrealDB-era `crate::import::consolidate`; the SQLite records use
//! plain `String` ids, so `record_id_string(&x.id)` becomes `x.id.clone()`.

use std::collections::{BTreeMap, BTreeSet};

use crate::sqlite::records::ImportEntityMention;

#[derive(Debug, Clone)]
pub struct ConsolidatedEntityCluster {
    pub entity_kind: String,
    pub canonical_name: String,
    pub normalized_name: String,
    pub aliases: Vec<String>,
    pub mention_ids: Vec<String>,
    pub first_segment_id: Option<String>,
    pub last_segment_id: Option<String>,
    pub importance_rank: i32,
    pub merge_confidence: f64,
    pub review_required: bool,
    pub notes: Vec<String>,
}

pub fn consolidate_mentions(
    mentions: &[ImportEntityMention],
    entity_kind_filter: Option<&BTreeSet<String>>,
) -> Vec<ConsolidatedEntityCluster> {
    let mut grouped = BTreeMap::<(String, String), Vec<&ImportEntityMention>>::new();

    for mention in mentions {
        if let Some(filter) = entity_kind_filter
            && !filter.contains(&mention.entity_kind)
        {
            continue;
        }
        let normalized_name = canonical_normalized_name(mention);
        grouped
            .entry((mention.entity_kind.clone(), normalized_name))
            .or_default()
            .push(mention);
    }

    let mut clusters = grouped
        .into_iter()
        .map(|((entity_kind, normalized_name), mentions)| {
            build_cluster(entity_kind, normalized_name, mentions)
        })
        .collect::<Vec<_>>();
    clusters.sort_by(|left, right| {
        right
            .importance_rank
            .cmp(&left.importance_rank)
            .then_with(|| left.entity_kind.cmp(&right.entity_kind))
            .then_with(|| left.normalized_name.cmp(&right.normalized_name))
    });
    clusters
}

fn build_cluster(
    entity_kind: String,
    normalized_name: String,
    mentions: Vec<&ImportEntityMention>,
) -> ConsolidatedEntityCluster {
    let mut aliases = BTreeSet::new();
    let mut canonical_name_counts = BTreeMap::<String, usize>::new();
    let mut mention_ids = Vec::with_capacity(mentions.len());
    let mut first_segment_id: Option<String> = None;
    let mut last_segment_id: Option<String> = None;
    let mut notes = Vec::new();
    let mut lowest_confidence = 1.0f64;

    for mention in &mentions {
        *canonical_name_counts
            .entry(mention.surface_form.clone())
            .or_insert(0) += 1;
        aliases.insert(mention.surface_form.clone());
        if let Some(alias_hint) = mention.alias_hint.as_ref() {
            aliases.insert(alias_hint.clone());
        }
        mention_ids.push(mention.id.clone());
        let segment_id = mention.segment_id.clone();
        if first_segment_id
            .as_ref()
            .is_none_or(|current| segment_id < *current)
        {
            first_segment_id = Some(segment_id.clone());
        }
        if last_segment_id
            .as_ref()
            .is_none_or(|current| segment_id > *current)
        {
            last_segment_id = Some(segment_id);
        }
        lowest_confidence = lowest_confidence.min(mention.confidence);
    }

    let canonical_name = canonical_name_counts
        .into_iter()
        .max_by(|left, right| {
            left.1
                .cmp(&right.1)
                .then_with(|| right.0.len().cmp(&left.0.len()))
        })
        .map(|(name, _)| name)
        .unwrap_or_else(|| normalized_name.clone());
    let alias_vec = aliases.into_iter().collect::<Vec<_>>();
    let alias_count = alias_vec.len();
    let review_required = alias_vec.len() >= 3 || lowest_confidence < 0.65;
    if alias_vec.len() >= 3 {
        notes.push("cluster merged several aliases and should be reviewed".to_string());
    }
    if lowest_confidence < 0.65 {
        notes.push("one or more source mentions had low extraction confidence".to_string());
    }

    ConsolidatedEntityCluster {
        entity_kind,
        canonical_name,
        normalized_name,
        aliases: alias_vec,
        mention_ids,
        first_segment_id,
        last_segment_id,
        importance_rank: mentions.len() as i32,
        merge_confidence: merge_confidence(mentions.len(), alias_count, lowest_confidence),
        review_required,
        notes,
    }
}

fn canonical_normalized_name(mention: &ImportEntityMention) -> String {
    mention
        .alias_hint
        .as_deref()
        .map(strip_honorific)
        .map(normalize)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| normalize(&mention.normalized_name))
}

fn strip_honorific(value: &str) -> &str {
    for prefix in ["lord ", "lady ", "captain ", "prince ", "princess "] {
        if let Some(rest) = value.to_ascii_lowercase().strip_prefix(prefix) {
            let offset = value.len() - rest.len();
            return &value[offset..];
        }
    }
    value
}

fn normalize(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != ' ' && ch != '\'')
        .to_ascii_lowercase()
}

fn merge_confidence(count: usize, alias_count: usize, lowest_confidence: f64) -> f64 {
    let density: f64 = if count >= 4 {
        0.92
    } else if count >= 2 {
        0.78
    } else {
        0.58
    };
    let alias_penalty: f64 = if alias_count >= 3 {
        0.12
    } else if alias_count == 2 {
        0.04
    } else {
        0.0
    };
    (density - alias_penalty)
        .min(lowest_confidence + 0.18)
        .clamp(0.0, 0.99)
}
