//! Deterministic candidate construction for the 0.4 semantic continuity lane.
//!
//! P2 deliberately stops before any model or host judgment. It selects the
//! bounded canon items that a future judge may inspect, ranks/caps them, and
//! records honest coverage. No findings are emitted and no model route is
//! called.

use super::*;
use crate::sqlite::records::{CanonicalFact, Scene, StoredQuantityState, WorldRule};
use sha2::{Digest, Sha256};
use spindle_core::character_mentions::{CharacterMentionRecord, CharacterMentionResolver};
use spindle_core::models::{
    PrepareSemanticContinuityInput, PrepareSemanticContinuityOutput, SemanticCandidateAudit,
};
use spindle_core::semantic_continuity::{
    SEMANTIC_CONTRACT_VERSION, SemanticCandidateFamily, SemanticCandidateSetV1,
    SemanticCandidateV1, SemanticCanonRefV1, SemanticEvidenceHintV1, SemanticHostRequestV1,
    scene_sha256, semantic_host_request_id,
};
use std::collections::{BTreeMap, BTreeSet};

const MAX_CANDIDATES: usize = 12;
const MAX_EXCERPT_CHARS: usize = 600;
const MAX_EXCERPT_HINT_BYTES: usize = 180;

#[derive(Debug, Clone)]
struct CandidateDraft {
    family: SemanticCandidateFamily,
    check_type: String,
    canon_refs: Vec<SemanticCanonRefV1>,
    subject_ids: Vec<String>,
    constraint: String,
    hint_start: usize,
    hint_end: usize,
    hint_origin: String,
    exact_mention: bool,
    scanner_hit: bool,
    updated_at: i64,
}

impl CandidateDraft {
    #[allow(clippy::too_many_arguments)]
    fn new(
        family: SemanticCandidateFamily,
        check_type: &str,
        refs: Vec<SemanticCanonRefV1>,
        subjects: Vec<String>,
        constraint: String,
        range: (usize, usize),
        origin: &str,
        exact_mention: bool,
        scanner_hit: bool,
        updated_at: i64,
    ) -> Self {
        Self {
            family,
            check_type: check_type.to_string(),
            canon_refs: refs,
            subject_ids: subjects,
            constraint,
            hint_start: range.0,
            hint_end: range.1,
            hint_origin: origin.to_string(),
            exact_mention,
            scanner_hit,
            updated_at,
        }
    }
}

impl SqliteSpindleService {
    /// Read-only host-request preparation for every scene in scope.
    pub async fn prepare_semantic_continuity(
        &self,
        input: PrepareSemanticContinuityInput,
    ) -> Result<PrepareSemanticContinuityOutput> {
        use crate::format::scoped_scenes;

        let project_id = input.project_id;
        let branch = self.repository.get_active_branch(&project_id).await?;
        let scope = input
            .scope
            .to_scope()
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let scenes = scoped_scenes(
            self.repository
                .list_scenes_by_project_and_branch(&project_id, &branch.id)
                .await?,
            &scope,
        );
        let scenes = self
            .narrow_scenes_by_subjects(&project_id, &branch.id, scenes, &input.subjects)
            .await?;
        let (requests, coverage) = self
            .semantic_requests_for_scenes(&project_id, &branch.id, &scenes)
            .await?;
        Ok(PrepareSemanticContinuityOutput { requests, coverage })
    }

    pub(super) async fn semantic_requests_for_scenes(
        &self,
        project_id: &str,
        branch_id: &str,
        scenes: &[Scene],
    ) -> Result<(Vec<SemanticHostRequestV1>, SemanticCandidateAudit)> {
        let characters = self
            .repository
            .list_characters_by_project_and_branch(project_id, branch_id)
            .await?;
        let character_by_id: BTreeMap<&str, &crate::sqlite::records::Character> = characters
            .iter()
            .map(|character| (character.id.as_str(), character))
            .collect();
        let mention_records: Vec<CharacterMentionRecord> = characters
            .iter()
            .map(|character| CharacterMentionRecord {
                id: character.id.clone(),
                name: character.name.clone(),
                aliases: character.aliases.clone(),
            })
            .collect();
        let resolver = CharacterMentionResolver::new(&mention_records);

        let states = self
            .repository
            .list_character_states_by_project_and_branch(project_id, branch_id)
            .await?;
        let facts = self
            .repository
            .list_active_canonical_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let knowledge = self
            .repository
            .list_knowledge_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let rules = self
            .repository
            .list_world_rules_by_project_and_branch(project_id, branch_id)
            .await?;
        let world_declares_respawn = rules.iter().any(world_rule_declares_respawn);
        let quantities = self
            .repository
            .list_quantity_states_by_project_and_branch(project_id, branch_id)
            .await?;
        let clocks = self
            .repository
            .list_scene_clocks_by_project_and_branch(project_id, branch_id)
            .await?;
        let timeline = self
            .repository
            .list_timeline_events_by_project_and_branch(project_id, branch_id)
            .await?;
        let mut requests = Vec::with_capacity(scenes.len());
        let mut audit = SemanticCandidateAudit {
            scene_count: scenes.len(),
            ..Default::default()
        };

        for scene in scenes {
            let mentioned = resolver.mentioned_ids(&scene.full_text);
            let scene_position = (scene.book_number, scene.chapter_number, scene.scene_order);
            let holders_by_secret = secret_holders_at(&knowledge, scene_position);
            let mut drafts = Vec::new();

            for fact in &facts {
                if fact.secret {
                    if let Some(draft) = secret_draft(
                        scene,
                        fact,
                        &mentioned,
                        &holders_by_secret,
                        &character_by_id,
                        &resolver,
                        &knowledge,
                    ) {
                        drafts.push(draft);
                    }
                    // A secret fact is only ever selected by the secret family.
                    continue;
                }
                if let Some(subject_id) = fact.subject_id.as_deref()
                    && mentioned.contains(subject_id)
                    && !is_non_attribute_predicate(&fact.predicate)
                    && position_lte(
                        (fact.book_number, fact.chapter_number, None),
                        scene_position,
                    )
                    && let Some(range) = fact_hint_range(scene, fact)
                        .or_else(|| subject_range(scene, subject_id, &resolver))
                {
                    drafts.push(CandidateDraft::new(
                        SemanticCandidateFamily::Attribute,
                        "canonical_fact_prose_drift",
                        vec![fact_ref(fact)],
                        vec![subject_id.to_string()],
                        fact_constraint(fact),
                        range,
                        "canonical_fact",
                        true,
                        false,
                        fact.updated_at.timestamp(),
                    ));
                }
            }

            for fact in &knowledge {
                if !mentioned.contains(&fact.character_id) {
                    continue;
                }
                let learned_later = fact.learned_at.as_ref().is_some_and(|placement| {
                    position_gt(
                        (
                            placement.book_number,
                            placement.chapter_number,
                            placement.scene_order,
                        ),
                        scene_position,
                    )
                });
                if !learned_later && fact.secret_of_fact_id.is_none() {
                    continue;
                }
                let range = first_phrase_range_str(
                    scene,
                    &[fact.normalized_fact.as_str(), fact.fact.as_str()],
                )
                .or_else(|| subject_range(scene, &fact.character_id, &resolver));
                let Some(range) = range else {
                    continue;
                };
                drafts.push(CandidateDraft::new(
                    SemanticCandidateFamily::Knowledge,
                    "knowledge_timing",
                    vec![canon_ref(
                        "knowledge_fact",
                        &fact.id,
                        hash_parts(&[
                            fact.id.clone(),
                            fact.character_id.clone(),
                            fact.normalized_fact.clone(),
                            fact.learned_at
                                .as_ref()
                                .map(|placement| {
                                    format!(
                                        "{}:{}:{}",
                                        placement.book_number,
                                        placement.chapter_number,
                                        placement.scene_order.unwrap_or(0)
                                    )
                                })
                                .unwrap_or_default(),
                        ]),
                    )],
                    vec![fact.character_id.clone()],
                    format!(
                        "knowledge learned later: {}",
                        truncate(&fact.normalized_fact, 160)
                    ),
                    range,
                    "resolver",
                    true,
                    false,
                    fact.updated_at.timestamp(),
                ));
            }

            for character_id in &mentioned {
                if world_declares_respawn || character_declares_respawn(character_id, &facts) {
                    continue;
                }
                // Include same-scene states in the selection so an explicit
                // respawn status supersedes an earlier death. A state at the
                // scene's own position is ambiguous for the opposite reason:
                // the character may die in this scene, so it never seeds a
                // "was already dead" candidate.
                let Some(state) = crate::sqlite::repository::select_latest_character_state(
                    states.iter(),
                    character_id,
                    scene_position,
                ) else {
                    continue;
                };
                if (state.book_number, state.chapter_number, state.scene_order) == scene_position {
                    continue;
                }
                let dead = state.status.iter().find(|status| is_dead_status(status));
                let Some(dead) = dead else {
                    continue;
                };
                if !character_by_id.contains_key(character_id.as_str()) {
                    continue;
                }
                let Some(range) = character_range(scene, resolver.aliases_for(character_id)) else {
                    continue;
                };
                drafts.push(CandidateDraft::new(
                    SemanticCandidateFamily::Death,
                    "dead_character_acting",
                    vec![canon_ref(
                        "character_state",
                        &state.id,
                        hash_parts(&[
                            state.id.clone(),
                            state.character_id.clone(),
                            state.status.join("|"),
                            format!(
                                "{}:{}:{}",
                                state.book_number, state.chapter_number, state.scene_order
                            ),
                        ]),
                    )],
                    vec![state.character_id.clone()],
                    format!("character is marked dead ({dead}) before this scene"),
                    range,
                    "resolver",
                    true,
                    false,
                    state.created_at.timestamp(),
                ));
            }

            for rule in &rules {
                if let Some(established) = &rule.established_in
                    && position_gt(
                        (established.book_number, established.chapter_number, None),
                        scene_position,
                    )
                {
                    continue;
                }
                let Some(range) = rule_hint_range(scene, rule) else {
                    continue;
                };
                let scanner = rule
                    .scan_pattern
                    .as_deref()
                    .filter(|pattern| !pattern.trim().is_empty())
                    .and_then(|pattern| {
                        regex::RegexBuilder::new(pattern)
                            .case_insensitive(true)
                            .build()
                            .ok()
                    })
                    .and_then(|regex| regex.find(&scene.full_text))
                    .is_some();
                drafts.push(CandidateDraft::new(
                    SemanticCandidateFamily::WorldRule,
                    "world_rule_semantic_drift",
                    vec![canon_ref(
                        "world_rule",
                        &rule.id,
                        hash_parts(&[
                            rule.id.clone(),
                            rule.rule_name.clone(),
                            rule.description.clone(),
                            rule.updated_at
                                .map_or(0, |time| time.timestamp())
                                .to_string(),
                        ]),
                    )],
                    Vec::new(),
                    format!(
                        "world rule '{}': {}",
                        rule.rule_name,
                        truncate(&rule.description, 220)
                    ),
                    range,
                    if scanner {
                        "world_rule_scan"
                    } else {
                        "resolver"
                    },
                    !scanner,
                    scanner,
                    rule.updated_at.map_or(0, |time| time.timestamp()),
                ));
            }

            for state in latest_quantities(&quantities, scene_position) {
                if !mentioned.contains(&state.subject_id) {
                    continue;
                }
                let Some(range) = quantity_hint_range(scene, state) else {
                    continue;
                };
                drafts.push(CandidateDraft::new(
                    SemanticCandidateFamily::Quantity,
                    "quantity_drift",
                    vec![canon_ref(
                        "quantity_state",
                        &state.id,
                        hash_parts(&[
                            state.id.clone(),
                            state.measure.clone(),
                            state
                                .state
                                .amount
                                .map(|value| value.to_string())
                                .unwrap_or_default(),
                            state.state.unit.clone().unwrap_or_default(),
                            state.state.band.clone().unwrap_or_default(),
                        ]),
                    )],
                    vec![state.subject_id.clone()],
                    quantity_constraint(state),
                    range,
                    "price_scan",
                    false,
                    true,
                    state.created_at.timestamp(),
                ));
            }

            for event in &timeline {
                if position_gt(
                    (
                        event.placement.book_number,
                        event.placement.chapter_number,
                        event.placement.scene_order,
                    ),
                    scene_position,
                ) {
                    continue;
                }
                let Some(range) =
                    first_phrase_range_str(scene, &[event.title.as_str(), event.summary.as_str()])
                else {
                    continue;
                };
                drafts.push(CandidateDraft::new(
                    SemanticCandidateFamily::Timeline,
                    "temporal_coherence",
                    vec![canon_ref(
                        "timeline_event",
                        &event.id,
                        hash_parts(&[
                            event.id.clone(),
                            event.title.clone(),
                            event.summary.clone(),
                            format!(
                                "{}:{}:{}",
                                event.placement.book_number,
                                event.placement.chapter_number,
                                event.placement.scene_order.unwrap_or(0)
                            ),
                        ]),
                    )],
                    event.related_entity_ids.clone(),
                    format!(
                        "timeline event '{}': {}",
                        event.title,
                        truncate(&event.summary, 180)
                    ),
                    range,
                    "resolver",
                    true,
                    false,
                    event.updated_at.timestamp(),
                ));
            }

            if let Some(clock) = clocks.iter().find(|clock| clock.scene_id == scene.id)
                && (clock.clock.duration_days.is_some() || clock.clock.day_index.is_some())
                && let Some(range) = temporal_phrase_range(scene)
            {
                drafts.push(CandidateDraft::new(
                    SemanticCandidateFamily::Timeline,
                    "temporal_coherence",
                    vec![canon_ref(
                        "scene_clock",
                        &clock.scene_id,
                        hash_parts(&[
                            clock.scene_id.clone(),
                            clock.clock.day_index.unwrap_or_default().to_string(),
                            clock.clock.time_of_day.unwrap_or_default().to_string(),
                            clock
                                .clock
                                .duration_days
                                .map(|value| value.to_string())
                                .unwrap_or_default(),
                            clock.temporal_mode.clone().unwrap_or_default(),
                            clock.thread_key.clone().unwrap_or_default(),
                        ]),
                    )],
                    Vec::new(),
                    clock_constraint(&clock.clock),
                    range,
                    "temporal_anchor",
                    false,
                    true,
                    clock.updated_at.timestamp(),
                ));
            }

            let (candidates, omitted) = rank_and_cap(scene, drafts);
            let canon_version = canon_version_for(&candidates);
            let candidate_set = SemanticCandidateSetV1 {
                schema_version: SEMANTIC_CONTRACT_VERSION.to_string(),
                scene_id: scene.id.clone(),
                scene_sha256: scene_sha256(&scene.full_text),
                canon_version,
                candidates,
                omitted_by_family: omitted.clone(),
            };
            let request = SemanticHostRequestV1 {
                request_id: semantic_host_request_id(&candidate_set),
                contract_version: SEMANTIC_CONTRACT_VERSION.to_string(),
                candidate_set,
            };
            if !request.candidate_set.candidates.is_empty() {
                audit.candidate_scene_count += 1;
                audit.candidate_count += request.candidate_set.candidates.len();
                for (family, count) in &omitted {
                    *audit.omitted_by_family.entry(family.clone()).or_default() += count;
                }
            }
            requests.push(request);
        }
        Ok((requests, audit))
    }
}

fn position_lte(left: (i32, i32, Option<i32>), right: (i32, i32, i32)) -> bool {
    (left.0, left.1, left.2.unwrap_or(0)) <= (right.0, right.1, right.2)
}

fn position_gt(left: (i32, i32, Option<i32>), right: (i32, i32, i32)) -> bool {
    (left.0, left.1, left.2.unwrap_or(0)) > (right.0, right.1, right.2)
}

fn is_non_attribute_predicate(predicate: &str) -> bool {
    matches!(
        predicate.trim().to_ascii_lowercase().as_str(),
        "life_status" | "status" | "vital_status" | "vital" | "state" | "condition" | "secret"
    )
}

fn fact_value(fact: &CanonicalFact) -> String {
    if let Some(text) = fact.value_text.as_deref() {
        return text.to_string();
    }
    if let Some(number) = fact.value_number {
        return format!("{number}{}", fact.unit.as_deref().unwrap_or(""));
    }
    if let Some(value) = fact.value_json.as_ref() {
        return value.to_string();
    }
    String::new()
}

fn fact_constraint(fact: &CanonicalFact) -> String {
    format!("{}: {}", fact.predicate, truncate(&fact_value(fact), 220))
}

fn fact_ref(fact: &CanonicalFact) -> SemanticCanonRefV1 {
    canon_ref(
        "canonical_fact",
        &fact.id,
        hash_parts(&[
            fact.id.clone(),
            fact.predicate.clone(),
            fact_value(fact),
            format!("{:?}", fact.valid_from),
            format!("{:?}", fact.valid_until),
            fact.updated_at.timestamp().to_string(),
        ]),
    )
}

fn fact_hint_range(scene: &Scene, fact: &CanonicalFact) -> Option<(usize, usize)> {
    let mut phrases = vec![fact_value(fact)];
    phrases.extend(fact.aliases.iter().cloned());
    first_phrase_range(scene, &phrases)
}

fn secret_holders_at(
    knowledge: &[crate::sqlite::records::KnowledgeFact],
    scene_position: (i32, i32, i32),
) -> BTreeMap<String, BTreeSet<String>> {
    let mut holders: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for fact in knowledge {
        let Some(secret_id) = fact.secret_of_fact_id.as_deref() else {
            continue;
        };
        let known_by_scene = fact.learned_at.as_ref().is_none_or(|placement| {
            (
                placement.book_number,
                placement.chapter_number,
                placement.scene_order.unwrap_or(0),
            ) <= scene_position
        });
        if known_by_scene {
            holders
                .entry(secret_id.to_string())
                .or_default()
                .insert(fact.character_id.clone());
        }
    }
    holders
}

fn secret_knowledge_refs(
    knowledge: &[crate::sqlite::records::KnowledgeFact],
    secret_id: &str,
) -> Vec<SemanticCanonRefV1> {
    let mut refs: Vec<SemanticCanonRefV1> = knowledge
        .iter()
        .filter(|fact| fact.secret_of_fact_id.as_deref() == Some(secret_id))
        .map(|fact| {
            let learned_at = fact
                .learned_at
                .as_ref()
                .map(|placement| {
                    format!(
                        "{}:{}:{}",
                        placement.book_number,
                        placement.chapter_number,
                        placement.scene_order.unwrap_or(0)
                    )
                })
                .unwrap_or_else(|| "start".to_string());
            canon_ref(
                "knowledge_fact",
                &fact.id,
                hash_parts(&[
                    fact.id.clone(),
                    fact.character_id.clone(),
                    fact.secret_of_fact_id.clone().unwrap_or_default(),
                    learned_at,
                ]),
            )
        })
        .collect();
    refs.sort();
    refs
}

fn subject_range(
    scene: &Scene,
    subject_id: &str,
    resolver: &CharacterMentionResolver,
) -> Option<(usize, usize)> {
    character_range(scene, resolver.aliases_for(subject_id))
}

fn secret_draft(
    scene: &Scene,
    fact: &CanonicalFact,
    mentioned: &BTreeSet<String>,
    holders_by_secret: &BTreeMap<String, BTreeSet<String>>,
    characters: &BTreeMap<&str, &crate::sqlite::records::Character>,
    resolver: &CharacterMentionResolver,
    knowledge: &[crate::sqlite::records::KnowledgeFact],
) -> Option<CandidateDraft> {
    let holders = holders_by_secret.get(&fact.id);
    let out_of_circle: Vec<String> = mentioned
        .iter()
        .filter(|character_id| {
            holders.is_none_or(|holders| !holders.contains(character_id.as_str()))
        })
        .filter(|character_id| characters.contains_key(character_id.as_str()))
        .cloned()
        .collect();
    if out_of_circle.is_empty() {
        return None;
    }
    let mut phrases = vec![fact_value(fact)];
    phrases.extend(fact.aliases.iter().cloned());
    phrases.push(fact.predicate.clone());
    let range = first_phrase_range(scene, &phrases).or_else(|| {
        out_of_circle
            .iter()
            .find_map(|id| subject_range(scene, id, resolver))
    })?;
    let mut refs = vec![fact_ref(fact)];
    refs.extend(secret_knowledge_refs(knowledge, &fact.id));
    Some(CandidateDraft::new(
        SemanticCandidateFamily::Secret,
        "secret_leak",
        refs,
        out_of_circle,
        format!(
            "secret fact '{}' ({}): {}",
            fact.predicate,
            fact.id,
            truncate(&fact_value(fact), 180)
        ),
        range,
        "resolver",
        true,
        false,
        fact.updated_at.timestamp(),
    ))
}

fn latest_quantities(
    states: &[StoredQuantityState],
    scene_position: (i32, i32, i32),
) -> Vec<&StoredQuantityState> {
    let mut latest: BTreeMap<(String, String, String), &StoredQuantityState> = BTreeMap::new();
    for state in states {
        if !position_lte(
            (
                state.book_number,
                state.chapter_number,
                Some(state.scene_order),
            ),
            scene_position,
        ) {
            continue;
        }
        let key = (
            state.subject_table.clone(),
            state.subject_id.clone(),
            state.measure.clone(),
        );
        let replace = latest.get(&key).is_none_or(|current| {
            (
                state.book_number,
                state.chapter_number,
                state.scene_order,
                state.created_at,
            ) > (
                current.book_number,
                current.chapter_number,
                current.scene_order,
                current.created_at,
            )
        });
        if replace {
            latest.insert(key, state);
        }
    }
    latest.into_values().collect()
}

fn quantity_constraint(state: &StoredQuantityState) -> String {
    let amount = state
        .state
        .amount
        .map(|value| value.to_string())
        .or_else(|| state.state.band.clone())
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "{} {} for {}:{}",
        amount,
        state.state.unit.as_deref().unwrap_or(""),
        state.subject_table,
        state.subject_id
    )
}

fn quantity_hint_range(scene: &Scene, state: &StoredQuantityState) -> Option<(usize, usize)> {
    let mut phrases = vec![state.measure.clone()];
    if let Some(unit) = state.state.unit.as_deref() {
        phrases.push(unit.to_string());
    }
    if let Some(amount) = state.state.amount {
        let full = format!("{amount} {}", state.state.unit.as_deref().unwrap_or(""));
        phrases.push(full);
        phrases.push(amount.to_string());
    }
    first_phrase_range(scene, &phrases)
}

fn clock_constraint(clock: &spindle_core::models::StoryClock) -> String {
    format!(
        "scene clock day={:?} time={:?} duration={:?}",
        clock.day_index, clock.time_of_day, clock.duration_days
    )
}

fn rule_hint_range(scene: &Scene, rule: &WorldRule) -> Option<(usize, usize)> {
    if let Some(pattern) = rule.scan_pattern.as_deref()
        && let Ok(regex) = regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
        && let Some(found) = regex.find(&scene.full_text)
    {
        return Some((found.start(), found.end()));
    }
    let mut phrases = vec![rule.rule_name.clone()];
    if let Some(tags) = &rule.relevance_tags {
        phrases.extend(tags.iter().cloned());
    }
    first_phrase_range(scene, &phrases)
}

fn character_range(scene: &Scene, aliases: &[String]) -> Option<(usize, usize)> {
    let phrases: Vec<&str> = aliases.iter().map(String::as_str).collect();
    first_phrase_range_str(scene, &phrases)
}

fn temporal_phrase_range(scene: &Scene) -> Option<(usize, usize)> {
    const MARKERS: &[&str] = &[
        "dawn",
        "dusk",
        "morning",
        "afternoon",
        "evening",
        "night",
        "midnight",
        "noon",
        "hours later",
        "minutes later",
        "days later",
        "later that day",
        "the next day",
        "meanwhile",
        "afterward",
        "afterwards",
        "before",
        "after",
    ];
    MARKERS
        .iter()
        .filter_map(|marker| first_phrase_range_str(scene, &[*marker]))
        .min_by_key(|range| range.0)
}

fn first_phrase_range(scene: &Scene, phrases: &[String]) -> Option<(usize, usize)> {
    first_phrase_range_str(
        scene,
        &phrases.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

fn first_phrase_range_str(scene: &Scene, phrases: &[&str]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for phrase in phrases {
        let phrase = phrase.trim();
        if phrase.chars().count() < 3 {
            continue;
        }
        let Ok(regex) = regex::RegexBuilder::new(&regex::escape(phrase))
            .case_insensitive(true)
            .build()
        else {
            continue;
        };
        if let Some(found) = regex.find(&scene.full_text) {
            let candidate = (found.start(), found.end());
            if best.is_none_or(|best| candidate.0 < best.0) {
                best = Some(candidate);
            }
        }
    }
    best
}

fn rank_and_cap(
    scene: &Scene,
    drafts: Vec<CandidateDraft>,
) -> (Vec<SemanticCandidateV1>, BTreeMap<String, usize>) {
    let mut omitted: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_family: BTreeMap<SemanticCandidateFamily, Vec<CandidateDraft>> = BTreeMap::new();
    for draft in drafts {
        by_family.entry(draft.family).or_default().push(draft);
    }
    let mut ranked: Vec<(CandidateDraft, SemanticCandidateV1)> = Vec::new();
    let mut seen_ids = BTreeSet::new();
    for (family, mut family_drafts) in by_family {
        family_drafts.sort_by(compare_drafts);
        let cap = family.default_cap();
        if family_drafts.len() > cap {
            *omitted.entry(family.as_str().to_string()).or_default() += family_drafts.len() - cap;
            family_drafts.truncate(cap);
        }
        for draft in family_drafts {
            if let Some(candidate) = candidate_from_draft(scene, &draft) {
                if !seen_ids.insert(candidate.candidate_id.clone()) {
                    *omitted.entry(family.as_str().to_string()).or_default() += 1;
                    continue;
                }
                ranked.push((draft, candidate));
            }
        }
    }
    ranked.sort_by(|left, right| compare_drafts(&left.0, &right.0));
    if ranked.len() > MAX_CANDIDATES {
        let dropped = &ranked[MAX_CANDIDATES..];
        for (draft, _) in dropped {
            *omitted
                .entry(draft.family.as_str().to_string())
                .or_default() += 1;
        }
        ranked.truncate(MAX_CANDIDATES);
    }
    let candidates = ranked.into_iter().map(|(_, candidate)| candidate).collect();
    (candidates, omitted)
}

fn compare_drafts(left: &CandidateDraft, right: &CandidateDraft) -> std::cmp::Ordering {
    right
        .exact_mention
        .cmp(&left.exact_mention)
        .then_with(|| right.scanner_hit.cmp(&left.scanner_hit))
        .then_with(|| family_priority(left.family).cmp(&family_priority(right.family)))
        .then_with(|| right.updated_at.cmp(&left.updated_at))
        .then_with(|| draft_candidate_id(left).cmp(&draft_candidate_id(right)))
}

fn family_priority(family: SemanticCandidateFamily) -> u8 {
    match family {
        SemanticCandidateFamily::Attribute => 6,
        SemanticCandidateFamily::Secret => 5,
        SemanticCandidateFamily::Knowledge => 4,
        SemanticCandidateFamily::Death => 3,
        SemanticCandidateFamily::WorldRule => 2,
        SemanticCandidateFamily::Quantity => 1,
        SemanticCandidateFamily::Timeline => 0,
    }
}

fn candidate_from_draft(scene: &Scene, draft: &CandidateDraft) -> Option<SemanticCandidateV1> {
    let (start, end) = excerpt_window(scene, draft.hint_start, draft.hint_end)?;
    let hint = (draft.hint_start, draft.hint_end);
    let mut hints = vec![SemanticEvidenceHintV1 {
        start_byte: hint.0,
        end_byte: hint.1,
        exact_quote: scene.full_text[hint.0..hint.1].to_string(),
        origin: draft.hint_origin.clone(),
    }];
    hints.retain(|hint| hint.start_byte < end && start < hint.end_byte);
    let excerpt = scene.full_text[start..end].to_string();
    let mut refs = draft.canon_refs.clone();
    refs.sort();
    refs.dedup();
    let mut subjects = draft.subject_ids.clone();
    subjects.sort();
    subjects.dedup();
    let candidate_id = candidate_id_for(draft, &refs, &excerpt);
    Some(SemanticCandidateV1 {
        schema_version: SEMANTIC_CONTRACT_VERSION.to_string(),
        candidate_id,
        family: draft.family,
        check_type: draft.check_type.clone(),
        canon_refs: refs,
        subject_ids: subjects,
        constraint: truncate(&draft.constraint, 320),
        excerpt,
        excerpt_start_byte: start,
        excerpt_end_byte: end,
        evidence_hints: hints,
    })
}

fn excerpt_window(scene: &Scene, hint_start: usize, hint_end: usize) -> Option<(usize, usize)> {
    if hint_start >= hint_end || hint_end > scene.full_text.len() {
        return None;
    }
    let mut start = hint_start.saturating_sub(MAX_EXCERPT_HINT_BYTES);
    while start < hint_start && !scene.full_text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = hint_end
        .saturating_add(MAX_EXCERPT_HINT_BYTES)
        .min(scene.full_text.len());
    while end > hint_end && !scene.full_text.is_char_boundary(end) {
        end -= 1;
    }
    while scene.full_text[start..end].chars().count() > MAX_EXCERPT_CHARS {
        end -= 1;
        while end > hint_end && !scene.full_text.is_char_boundary(end) {
            end -= 1;
        }
        if end <= hint_end {
            start = hint_start;
            end = hint_end;
            break;
        }
    }
    Some((start, end))
}

fn candidate_id_for(draft: &CandidateDraft, refs: &[SemanticCanonRefV1], excerpt: &str) -> String {
    let hash = hash_parts(&[
        draft.family.as_str().to_string(),
        draft.check_type.clone(),
        serde_json::to_string(refs).unwrap_or_default(),
        excerpt.to_string(),
    ]);
    format!("semcand:{hash}")
}

fn draft_candidate_id(draft: &CandidateDraft) -> String {
    candidate_id_for(draft, &draft.canon_refs, &draft.constraint)
}

fn canon_version_for(candidates: &[SemanticCandidateV1]) -> String {
    let refs: BTreeSet<String> = candidates
        .iter()
        .flat_map(|candidate| candidate.canon_refs.iter())
        .map(|reference| format!("{}:{}:{}", reference.table, reference.id, reference.version))
        .collect();
    hash_parts(&refs.into_iter().collect::<Vec<_>>())
}

fn canon_ref(table: &str, id: &str, version: String) -> SemanticCanonRefV1 {
    SemanticCanonRefV1 {
        table: table.to_string(),
        id: id.to_string(),
        version,
    }
}

fn hash_parts(parts: &[String]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn scene(text: &str) -> Scene {
        Scene {
            id: "scene:test".into(),
            project_id: "project:test".into(),
            branch_id: "branch:test".into(),
            book_id: "book:test".into(),
            chapter_id: "chapter:test".into(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: text.into(),
            summary: "test".into(),
            content_rating: "General".into(),
            tone: None,
            draft_origin: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            location_id: None,
        }
    }

    fn draft(index: usize) -> CandidateDraft {
        CandidateDraft::new(
            SemanticCandidateFamily::Attribute,
            "canonical_fact_consistency",
            vec![canon_ref(
                "canonical_fact",
                &format!("fact:{index}"),
                format!("v{index}"),
            )],
            vec!["character:test".into()],
            format!("constraint {index}"),
            (0, 5),
            "canonical_fact",
            true,
            false,
            index as i64,
        )
    }

    fn secret_knowledge(learned_chapter: i32) -> crate::sqlite::records::KnowledgeFact {
        crate::sqlite::records::KnowledgeFact {
            id: "knowledge_fact:test".into(),
            project_id: "project:test".into(),
            branch_id: "branch:test".into(),
            character_id: "character:test".into(),
            fact: "the key is silver".into(),
            normalized_fact: "the key is silver".into(),
            source_summary: "test".into(),
            learned_at: Some(crate::sqlite::records::StoredStoryPlacement {
                book_number: 1,
                chapter_number: learned_chapter,
                scene_order: Some(1),
                note: None,
            }),
            confidence: None,
            tags: Vec::new(),
            reader_visible: true,
            source_import_session_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            secret_of_fact_id: Some("canonical_fact:secret".into()),
        }
    }

    #[test]
    fn secret_holder_timing_changes_membership_and_canon_version() {
        let early = secret_knowledge(1);
        let late = secret_knowledge(10);
        let before_reveal = secret_holders_at(std::slice::from_ref(&late), (1, 1, 1));
        assert!(
            !before_reveal
                .get("canonical_fact:secret")
                .is_some_and(|holders| holders.contains("character:test"))
        );
        let after_reveal = secret_holders_at(&[late], (1, 10, 1));
        assert!(
            after_reveal
                .get("canonical_fact:secret")
                .is_some_and(|holders| holders.contains("character:test"))
        );
        let early_refs = secret_knowledge_refs(&[early], "canonical_fact:secret");
        let late_refs = secret_knowledge_refs(&[secret_knowledge(10)], "canonical_fact:secret");
        assert_ne!(early_refs, late_refs);
    }

    #[test]
    fn family_and_global_caps_record_omissions() {
        let scene = scene("Mara crossed the yard.");
        let drafts = (0..20).map(draft).collect();
        let (candidates, omitted) = rank_and_cap(&scene, drafts);
        assert_eq!(
            candidates.len(),
            SemanticCandidateFamily::Attribute.default_cap()
        );
        assert_eq!(omitted.get("attribute"), Some(&14));
        assert!(candidates.iter().all(|candidate| {
            candidate.excerpt.len() <= scene.full_text.len()
                && candidate.excerpt_start_byte < candidate.excerpt_end_byte
        }));
    }

    #[test]
    fn excerpt_is_bounded_and_keeps_the_hint_inside_it() {
        let text = format!("{}THE HINT{}", "a".repeat(900), "b".repeat(900));
        let scene = scene(&text);
        let start = text.find("THE HINT").unwrap();
        let end = start + "THE HINT".len();
        let candidate = candidate_from_draft(
            &scene,
            &CandidateDraft::new(
                SemanticCandidateFamily::WorldRule,
                "world_rule_semantic_drift",
                vec![canon_ref("world_rule", "rule:1", "v1".into())],
                Vec::new(),
                "rule".into(),
                (start, end),
                "world_rule_scan",
                false,
                true,
                0,
            ),
        )
        .unwrap();
        assert!(candidate.excerpt.chars().count() <= MAX_EXCERPT_CHARS);
        assert!(candidate.evidence_hints.iter().all(|hint| {
            hint.start_byte >= candidate.excerpt_start_byte
                && hint.end_byte <= candidate.excerpt_end_byte
                && hint.exact_quote == "THE HINT"
        }));
    }

    #[test]
    fn canon_version_changes_when_a_referenced_row_changes() {
        let one = SemanticCandidateV1 {
            schema_version: SEMANTIC_CONTRACT_VERSION.into(),
            candidate_id: "c1".into(),
            family: SemanticCandidateFamily::Attribute,
            check_type: "canonical_fact_consistency".into(),
            canon_refs: vec![canon_ref("canonical_fact", "fact:1", "v1".into())],
            subject_ids: vec!["character:1".into()],
            constraint: "x".into(),
            excerpt: "x".into(),
            excerpt_start_byte: 0,
            excerpt_end_byte: 1,
            evidence_hints: Vec::new(),
        };
        let mut two = one.clone();
        two.canon_refs[0].version = "v2".into();
        assert_ne!(canon_version_for(&[one]), canon_version_for(&[two]));
    }
}
