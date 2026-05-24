//! World-extraction pass.
//!
//! Pure logic: walks normalized segment text, plus consolidated character /
//! location clusters and mentions, to draft world-rules, location candidates,
//! faction/religion/economy/term entities, and system signals. The service
//! layer persists the draft via `Repository::upsert_import_world_dossier`.
//! Ported from the SurrealDB-era `crate::import::world`; SQLite records use
//! plain `String` ids so `record_id_string(&x.id)` becomes `x.id.clone()`.

use std::collections::{BTreeMap, BTreeSet};

use spindle_core::models::{
    ImportConfidenceLevel, ImportEntityKind, ImportLocationCandidate, ImportSystemSignalSummary,
    ImportWorldDossierSummary, ImportWorldEntityCandidate, ImportWorldRuleCandidate,
};

use crate::sqlite::records::{ImportEntityCluster, ImportEntityMention, ImportSegment};

#[derive(Debug, Clone)]
pub struct WorldReviewDraft {
    pub title: String,
    pub description: String,
    pub source_segment_ids: Vec<String>,
    pub entity_kind: ImportEntityKind,
    pub canonical_name: String,
    pub confidence: f64,
    pub replacement_summary: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorldExtractionDraft {
    pub world_rules: Vec<ImportWorldRuleCandidate>,
    pub locations: Vec<ImportLocationCandidate>,
    pub entities: Vec<ImportWorldEntityCandidate>,
    pub system_signals: Vec<ImportSystemSignalSummary>,
    pub review_items: Vec<WorldReviewDraft>,
}

pub fn extract_world_dossier(
    segments: &[ImportSegment],
    segment_texts: &BTreeMap<String, String>,
    mentions: &[ImportEntityMention],
    clusters: &[ImportEntityCluster],
) -> WorldExtractionDraft {
    let world_rules = build_world_rules(segments, segment_texts);
    let locations = build_location_candidates(mentions, clusters);
    let entities = build_world_entities(segments, segment_texts, mentions, clusters);
    let system_signals = build_system_signals(segments, segment_texts);
    let review_items = build_review_items(&world_rules, &entities);

    WorldExtractionDraft {
        world_rules,
        locations,
        entities,
        system_signals,
        review_items,
    }
}

pub fn to_world_summary(draft: &WorldExtractionDraft) -> ImportWorldDossierSummary {
    ImportWorldDossierSummary {
        world_rules: draft.world_rules.clone(),
        locations: draft.locations.clone(),
        entities: draft.entities.clone(),
        system_signals: draft.system_signals.clone(),
    }
}

fn build_world_rules(
    segments: &[ImportSegment],
    segment_texts: &BTreeMap<String, String>,
) -> Vec<ImportWorldRuleCandidate> {
    let mut rules = BTreeMap::<String, WorldRuleAggregate>::new();

    for segment in segments
        .iter()
        .filter(|segment| segment.segment_type == "scene")
    {
        let segment_id = segment.id.clone();
        let Some(text) = segment_texts.get(&segment_id) else {
            continue;
        };

        for sentence in split_sentences(text) {
            let lowered = sentence.to_ascii_lowercase();
            let Some((confidence, rule_type)) = classify_rule_sentence(&lowered) else {
                continue;
            };
            let rule_name = summarize(sentence, 72);
            let entry = rules
                .entry(rule_name.to_ascii_lowercase())
                .or_insert_with(|| WorldRuleAggregate {
                    rule_name: rule_name.clone(),
                    rule_type: rule_type.to_string(),
                    description: sentence.to_string(),
                    source_segment_ids: BTreeSet::new(),
                    confidence,
                });
            entry.source_segment_ids.insert(segment_id.clone());
            if confidence > entry.confidence {
                entry.confidence = confidence;
            }
        }
    }

    rules
        .into_values()
        .map(|rule| ImportWorldRuleCandidate {
            rule_name: rule.rule_name,
            rule_type: rule.rule_type,
            description: rule.description,
            source_segment_ids: rule.source_segment_ids.into_iter().collect(),
            confidence: rule.confidence,
            confidence_level: confidence_level(rule.confidence),
        })
        .collect()
}

fn build_location_candidates(
    mentions: &[ImportEntityMention],
    clusters: &[ImportEntityCluster],
) -> Vec<ImportLocationCandidate> {
    let mentions_by_id = mentions
        .iter()
        .map(|mention| (mention.id.clone(), mention))
        .collect::<BTreeMap<_, _>>();
    let mut clustered_names = BTreeSet::new();
    let mut locations = clusters
        .iter()
        .filter(|cluster| cluster.entity_kind == "location")
        .map(|cluster| {
            clustered_names.insert(cluster.normalized_name.clone());
            let source_segment_ids = cluster
                .mention_ids
                .iter()
                .filter_map(|mention_id| mentions_by_id.get(mention_id))
                .map(|mention| mention.segment_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let summary = cluster
                .mention_ids
                .iter()
                .filter_map(|mention_id| mentions_by_id.get(mention_id))
                .find_map(|mention| mention.surrounding_text.clone())
                .unwrap_or_else(|| {
                    format!(
                        "{} appears as a recurring location in the imported manuscript.",
                        cluster.canonical_name
                    )
                });

            ImportLocationCandidate {
                cluster_id: Some(cluster.id.clone()),
                name: cluster.canonical_name.clone(),
                kind: infer_location_kind(&cluster.canonical_name),
                realm: None,
                summary,
                source_segment_ids,
                confidence: cluster.merge_confidence,
                confidence_level: confidence_level(cluster.merge_confidence),
            }
        })
        .collect::<Vec<_>>();

    let mut mention_groups = BTreeMap::<String, Vec<&ImportEntityMention>>::new();
    for mention in mentions
        .iter()
        .filter(|mention| mention.entity_kind == "location")
    {
        if clustered_names.contains(&mention.normalized_name) {
            continue;
        }
        mention_groups
            .entry(mention.normalized_name.clone())
            .or_default()
            .push(mention);
    }

    for group in mention_groups.into_values() {
        let Some(first) = group.first() else {
            continue;
        };
        let name = group
            .iter()
            .map(|mention| mention.surface_form.clone())
            .max_by_key(|value| value.len())
            .unwrap_or_else(|| first.surface_form.clone());
        let source_segment_ids = group
            .iter()
            .map(|mention| mention.segment_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let confidence =
            group.iter().map(|mention| mention.confidence).sum::<f64>() / group.len() as f64;
        locations.push(ImportLocationCandidate {
            cluster_id: None,
            name: name.clone(),
            kind: infer_location_kind(&name),
            realm: None,
            summary: first.surrounding_text.clone().unwrap_or_else(|| {
                format!("{} appears as a location in the imported manuscript.", name)
            }),
            source_segment_ids,
            confidence,
            confidence_level: confidence_level(confidence),
        });
    }

    locations.sort_by(|left, right| left.name.cmp(&right.name));
    locations
}

fn build_world_entities(
    segments: &[ImportSegment],
    segment_texts: &BTreeMap<String, String>,
    mentions: &[ImportEntityMention],
    clusters: &[ImportEntityCluster],
) -> Vec<ImportWorldEntityCandidate> {
    let reserved_names = clusters
        .iter()
        .filter(|cluster| matches!(cluster.entity_kind.as_str(), "character" | "location"))
        .map(|cluster| cluster.normalized_name.clone())
        .chain(
            mentions
                .iter()
                .filter(|mention| matches!(mention.entity_kind.as_str(), "character" | "location"))
                .map(|mention| mention.normalized_name.clone()),
        )
        .collect::<BTreeSet<_>>();
    let mut entities = BTreeMap::<String, EntityAggregate>::new();

    for segment in segments
        .iter()
        .filter(|segment| segment.segment_type == "scene")
    {
        let segment_id = segment.id.clone();
        let Some(text) = segment_texts.get(&segment_id) else {
            continue;
        };

        for seed in extract_faction_seeds(text, &segment_id)
            .into_iter()
            .chain(extract_religion_seeds(text, &segment_id))
            .chain(extract_economy_seeds(text, &segment_id))
            .chain(extract_term_seeds(text, &segment_id, &reserved_names))
        {
            let key = format!(
                "{}:{}",
                entity_kind_key(&seed.entity_kind),
                normalize_phrase(&seed.canonical_name)
            );
            let entry = entities.entry(key).or_insert_with(|| EntityAggregate {
                entity_kind: seed.entity_kind.clone(),
                canonical_name: seed.canonical_name.clone(),
                summary: seed.summary.clone(),
                realm: None,
                tags: BTreeSet::new(),
                source_segment_ids: BTreeSet::new(),
                confidence: seed.confidence,
            });
            entry.tags.extend(seed.tags);
            entry.source_segment_ids.insert(seed.source_segment_id);
            if seed.confidence > entry.confidence {
                entry.confidence = seed.confidence;
                entry.summary = seed.summary;
            }
        }
    }

    entities
        .into_values()
        .map(|entity| ImportWorldEntityCandidate {
            entity_kind: entity.entity_kind,
            cluster_id: None,
            canonical_name: entity.canonical_name,
            summary: entity.summary,
            realm: entity.realm,
            tags: entity.tags.into_iter().collect(),
            source_segment_ids: entity.source_segment_ids.into_iter().collect(),
            confidence: entity.confidence,
            confidence_level: confidence_level(entity.confidence),
        })
        .collect()
}

fn build_system_signals(
    segments: &[ImportSegment],
    segment_texts: &BTreeMap<String, String>,
) -> Vec<ImportSystemSignalSummary> {
    let mut signals = BTreeMap::<String, SignalAggregate>::new();

    for segment in segments
        .iter()
        .filter(|segment| segment.segment_type == "scene")
    {
        let segment_id = segment.id.clone();
        let Some(text) = segment_texts.get(&segment_id) else {
            continue;
        };
        let lowered = text.to_ascii_lowercase();

        let time_matches = count_matches(
            &lowered,
            &[
                "timeline",
                "reset",
                "rewind",
                "loop",
                "paradox",
                "future memory",
                "already lived",
            ],
        );
        if time_matches > 0 {
            upsert_signal(
                &mut signals,
                "time_awareness",
                "The imported text implies timeline interference or future-knowledge effects.",
                segment_id.clone(),
                if time_matches >= 2 { 0.84 } else { 0.68 },
            );
        }

        let litrpg_matches = count_matches(
            &lowered,
            &[
                "level",
                "quest",
                "mana",
                "status screen",
                "skill",
                "inventory",
                "xp",
            ],
        );
        if litrpg_matches > 0 {
            upsert_signal(
                &mut signals,
                "litrpg_overlay",
                "The imported text uses explicit system-overlay or LitRPG vocabulary.",
                segment_id,
                if litrpg_matches >= 2 { 0.86 } else { 0.7 },
            );
        }
    }

    signals
        .into_values()
        .map(|signal| ImportSystemSignalSummary {
            signal_type: signal.signal_type,
            summary: signal.summary,
            source_segment_ids: signal.source_segment_ids.into_iter().collect(),
            confidence: signal.confidence,
            confidence_level: confidence_level(signal.confidence),
        })
        .collect()
}

fn build_review_items(
    world_rules: &[ImportWorldRuleCandidate],
    entities: &[ImportWorldEntityCandidate],
) -> Vec<WorldReviewDraft> {
    let mut review_items = world_rules
        .iter()
        .filter(|rule| rule.confidence < 0.7)
        .map(|rule| WorldReviewDraft {
            title: format!("Review world rule '{}'", rule.rule_name),
            description: format!(
                "This rule was inferred from implicit setting language and should be confirmed before hydration. {}",
                rule.description
            ),
            source_segment_ids: rule.source_segment_ids.clone(),
            entity_kind: ImportEntityKind::WorldRule,
            canonical_name: rule.rule_name.clone(),
            confidence: rule.confidence,
            replacement_summary: Some(rule.description.clone()),
        })
        .collect::<Vec<_>>();

    review_items.extend(
        entities
            .iter()
            .filter(|entity| {
                matches!(entity.entity_kind, ImportEntityKind::Term) && entity.confidence < 0.65
            })
            .map(|entity| WorldReviewDraft {
                title: format!("Review imported term '{}'", entity.canonical_name),
                description: format!(
                    "This specialized term was inferred heuristically and should be validated before canon hydration. {}",
                    entity.summary
                ),
                source_segment_ids: entity.source_segment_ids.clone(),
                entity_kind: ImportEntityKind::Term,
                canonical_name: entity.canonical_name.clone(),
                confidence: entity.confidence,
                replacement_summary: Some(entity.summary.clone()),
            }),
    );

    review_items.sort_by(|left, right| {
        left.confidence
            .partial_cmp(&right.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.title.cmp(&right.title))
    });
    review_items
}

fn classify_rule_sentence(sentence: &str) -> Option<(f64, &'static str)> {
    let explicit = [
        "must",
        "cannot",
        "can't",
        "always",
        "never",
        "requires",
        "costs",
        "forbidden",
        "only if",
    ]
    .iter()
    .any(|marker| sentence.contains(marker));
    let implicit = [
        "ward", "wards", "sigil", "ritual", "mana", "protocol", "array", "quest", "timeline",
        "reset", "oath", "law",
    ]
    .iter()
    .any(|marker| sentence.contains(marker));

    if explicit && implicit {
        Some((0.84, infer_rule_type(sentence)))
    } else if implicit {
        Some((0.58, infer_rule_type(sentence)))
    } else {
        None
    }
}

fn infer_rule_type(sentence: &str) -> &'static str {
    if ["blood", "ward", "mana", "magic", "ritual", "sigil"]
        .iter()
        .any(|marker| sentence.contains(marker))
    {
        "magic"
    } else if [
        "protocol", "array", "status", "system", "quest", "reset", "timeline",
    ]
    .iter()
    .any(|marker| sentence.contains(marker))
    {
        "system"
    } else if ["law", "oath", "church", "council", "court"]
        .iter()
        .any(|marker| sentence.contains(marker))
    {
        "social"
    } else {
        "setting"
    }
}

fn infer_location_kind(name: &str) -> String {
    name.split_whitespace()
        .last()
        .map(|token| token.to_ascii_lowercase())
        .unwrap_or_else(|| "place".to_string())
}

fn extract_faction_seeds(text: &str, segment_id: &str) -> Vec<EntitySeed> {
    let tokens = title_tokens(text);
    let mut seeds = BTreeMap::new();
    let suffixes = [
        "Watch", "Guard", "Order", "Guild", "Council", "Legion", "Company", "Fleet",
    ];
    let prefixes = ["House", "Order", "Guild", "Fleet"];

    for window in tokens.windows(2) {
        if let [left, right] = window {
            if looks_like_title(left) && suffixes.contains(&right.as_str()) {
                let name = format!("{} {}", left, right);
                seeds.insert(
                    name.clone(),
                    EntitySeed {
                        entity_kind: ImportEntityKind::Faction,
                        canonical_name: name.clone(),
                        summary: format!(
                            "{} appears as an organized faction or institution in the imported manuscript.",
                            name
                        ),
                        tags: faction_tags(&name),
                        source_segment_id: segment_id.to_string(),
                        confidence: 0.78,
                    },
                );
            }
            if prefixes.contains(&left.as_str()) && looks_like_title(right) {
                let name = format!("{} {}", left, right);
                seeds.insert(
                    name.clone(),
                    EntitySeed {
                        entity_kind: ImportEntityKind::Faction,
                        canonical_name: name.clone(),
                        summary: format!(
                            "{} appears as an organized faction or institution in the imported manuscript.",
                            name
                        ),
                        tags: faction_tags(&name),
                        source_segment_id: segment_id.to_string(),
                        confidence: 0.76,
                    },
                );
            }
        }
    }

    seeds.into_values().collect()
}

fn extract_religion_seeds(text: &str, segment_id: &str) -> Vec<EntitySeed> {
    let tokens = title_tokens(text);
    let mut seeds = BTreeMap::new();
    let suffixes = ["Faith", "Creed", "Cult"];

    for window in tokens.windows(2) {
        if let [left, right] = window
            && ((looks_like_title(left) && suffixes.contains(&right.as_str()))
                || (left == "Old" && right == "Gods"))
        {
            let name = format!("{} {}", left, right);
            seeds.insert(
                name.clone(),
                EntitySeed {
                    entity_kind: ImportEntityKind::Religion,
                    canonical_name: name.clone(),
                    summary: format!(
                        "{} appears as a religious tradition or belief structure in the imported manuscript.",
                        name
                    ),
                    tags: vec!["belief".to_string()],
                    source_segment_id: segment_id.to_string(),
                    confidence: 0.76,
                },
            );
        }
    }

    for window in tokens.windows(3) {
        if let [left, middle, right] = window
            && left == "Church"
            && middle.eq_ignore_ascii_case("of")
            && looks_like_title(right)
        {
            let name = format!("{} {} {}", left, middle, right);
            seeds.insert(
                name.clone(),
                EntitySeed {
                    entity_kind: ImportEntityKind::Religion,
                    canonical_name: name.clone(),
                    summary: format!(
                        "{} appears as an organized religion in the imported manuscript.",
                        name
                    ),
                    tags: vec!["belief".to_string(), "institution".to_string()],
                    source_segment_id: segment_id.to_string(),
                    confidence: 0.8,
                },
            );
        }
    }

    seeds.into_values().collect()
}

fn extract_economy_seeds(text: &str, segment_id: &str) -> Vec<EntitySeed> {
    let lowered = text.to_ascii_lowercase();
    [
        ("tithe", "Tithe system", vec!["tribute".to_string()]),
        ("tariff", "Tariff regime", vec!["trade".to_string()]),
        ("ration", "Ration economy", vec!["scarcity".to_string()]),
        ("credit", "Credit ledger", vec!["credit".to_string()]),
        ("coin", "Coin economy", vec!["currency".to_string()]),
        ("ledger", "Trade ledger", vec!["accounting".to_string()]),
    ]
    .into_iter()
    .filter(|(marker, _, _)| lowered.contains(marker))
    .map(|(_, canonical_name, tags)| EntitySeed {
        entity_kind: ImportEntityKind::Economy,
        canonical_name: canonical_name.to_string(),
        summary: format!(
            "{} shapes exchange, scarcity, or trade in the imported manuscript.",
            canonical_name
        ),
        tags,
        source_segment_id: segment_id.to_string(),
        confidence: 0.72,
    })
    .collect()
}

fn extract_term_seeds(
    text: &str,
    segment_id: &str,
    reserved_names: &BTreeSet<String>,
) -> Vec<EntitySeed> {
    let tokens = title_tokens(text);
    let suffixes = [
        "Array",
        "Engine",
        "Matrix",
        "Protocol",
        "Core",
        "Interface",
        "Sigil",
        "Shard",
        "Screen",
    ];

    tokens
        .windows(2)
        .filter_map(|window| {
            let [left, right] = window else {
                return None;
            };
            if !looks_like_title(left) || !suffixes.contains(&right.as_str()) {
                return None;
            }
            let canonical_name = format!("{} {}", left, right);
            (!reserved_names.contains(&normalize_phrase(&canonical_name))).then_some(EntitySeed {
                entity_kind: ImportEntityKind::Term,
                canonical_name: canonical_name.clone(),
                summary: format!(
                    "{} appears as a specialized in-world term that likely needs glossary review.",
                    canonical_name
                ),
                tags: vec!["glossary".to_string()],
                source_segment_id: segment_id.to_string(),
                confidence: 0.56,
            })
        })
        .collect()
}

fn upsert_signal(
    signals: &mut BTreeMap<String, SignalAggregate>,
    signal_type: &str,
    summary: &str,
    segment_id: String,
    confidence: f64,
) {
    let entry = signals
        .entry(signal_type.to_string())
        .or_insert_with(|| SignalAggregate {
            signal_type: signal_type.to_string(),
            summary: summary.to_string(),
            source_segment_ids: BTreeSet::new(),
            confidence,
        });
    entry.source_segment_ids.insert(segment_id);
    if confidence > entry.confidence {
        entry.confidence = confidence;
    }
}

fn faction_tags(name: &str) -> Vec<String> {
    if name.contains("Watch") || name.contains("Guard") || name.contains("Legion") {
        vec!["military".to_string()]
    } else if name.contains("Guild") {
        vec!["trade".to_string()]
    } else if name.contains("Council") {
        vec!["civic".to_string()]
    } else if name.contains("House") {
        vec!["noble".to_string()]
    } else {
        vec!["organization".to_string()]
    }
}

fn count_matches(input: &str, markers: &[&str]) -> usize {
    markers
        .iter()
        .filter(|marker| input.contains(**marker))
        .count()
}

fn entity_kind_key(kind: &ImportEntityKind) -> &'static str {
    match kind {
        ImportEntityKind::Faction => "faction",
        ImportEntityKind::Religion => "religion",
        ImportEntityKind::Economy => "economy",
        ImportEntityKind::Term => "term",
        _ => "other",
    }
}

fn title_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(clean_token)
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn clean_token(token: &str) -> &str {
    token.trim_matches(|ch: char| !ch.is_ascii_alphabetic() && ch != '\'')
}

fn looks_like_title(token: &str) -> bool {
    let token = clean_token(token);
    token.len() >= 2
        && token
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
        && token
            .chars()
            .skip(1)
            .all(|ch| ch.is_ascii_lowercase() || ch == '\'')
}

fn normalize_phrase(input: &str) -> String {
    input
        .split_whitespace()
        .map(clean_token)
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn summarize(input: &str, max_chars: usize) -> String {
    let trimmed = input
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\''));
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let shortened = trimmed
            .chars()
            .take(max_chars.saturating_sub(3))
            .collect::<String>();
        format!("{}...", shortened)
    }
}

fn split_sentences(text: &str) -> Vec<&str> {
    text.split(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn confidence_level(confidence: f64) -> ImportConfidenceLevel {
    if confidence >= 0.8 {
        ImportConfidenceLevel::High
    } else if confidence >= 0.55 {
        ImportConfidenceLevel::Medium
    } else {
        ImportConfidenceLevel::Low
    }
}

struct WorldRuleAggregate {
    rule_name: String,
    rule_type: String,
    description: String,
    source_segment_ids: BTreeSet<String>,
    confidence: f64,
}

struct EntitySeed {
    entity_kind: ImportEntityKind,
    canonical_name: String,
    summary: String,
    tags: Vec<String>,
    source_segment_id: String,
    confidence: f64,
}

struct EntityAggregate {
    entity_kind: ImportEntityKind,
    canonical_name: String,
    summary: String,
    realm: Option<String>,
    tags: BTreeSet<String>,
    source_segment_ids: BTreeSet<String>,
    confidence: f64,
}

struct SignalAggregate {
    signal_type: String,
    summary: String,
    source_segment_ids: BTreeSet<String>,
    confidence: f64,
}
