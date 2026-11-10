//! Compatibility projections from `CanonStateViewV1` + visible `CanonEventV1`s,
//! shadow-read parity comparator, and per-project cutover readiness gate (P2 PR D).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use spindle_core::canon::*;
use spindle_core::models::StoryPlacement;
use spindle_core::subject::{Subject, SubjectTable};
use spindle_core::subject_snapshot::SubjectSnapshot;

use super::*;

fn extract_scalar_json_records<T: DeserializeOwned>(
    view: &CanonStateViewV1,
    key_prefix_or_exact: &str,
    exact: bool,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    for entity in view.entities.values() {
        for (attr_key, attr_state) in &entity.attributes {
            let matches = if exact {
                attr_key == key_prefix_or_exact
            } else {
                attr_key.starts_with(key_prefix_or_exact)
            };
            if !matches {
                continue;
            }
            if let Some(ClaimScalar::Text { value }) = &attr_state.scalar_value {
                let record: T = serde_json::from_str(value)?;
                out.push(record);
            }
        }
    }
    Ok(out)
}

fn branch_depth(view: &CanonStateViewV1, branch_id: &str) -> usize {
    view.lineage
        .iter()
        .position(|b| b == branch_id)
        .unwrap_or(usize::MAX)
}

/// Reconstructs active `CanonicalFact` rows on a branch from `CanonStateViewV1`,
/// matching `Repository::list_active_canonical_facts_by_project_and_branch`.
pub fn project_canonical_facts_from_canon(view: &CanonStateViewV1) -> Result<Vec<CanonicalFact>> {
    let mut facts: Vec<CanonicalFact> =
        extract_scalar_json_records(view, "legacy.canonical_fact.", false)?;
    facts.retain(|f| f.superseded_by.is_none());
    facts.sort_by(|a, b| {
        (a.book_number, a.chapter_number, &a.created_at, &a.id).cmp(&(
            b.book_number,
            b.chapter_number,
            &b.created_at,
            &b.id,
        ))
    });
    Ok(facts)
}

/// Reconstructs `CharacterState` snapshots on a branch from `CanonStateViewV1`,
/// matching `Repository::list_character_states_by_project_and_branch`.
pub fn project_character_states_from_canon(view: &CanonStateViewV1) -> Result<Vec<CharacterState>> {
    let mut states: Vec<CharacterState> =
        extract_scalar_json_records(view, "legacy.character_state.snapshot.", false)?;
    states.sort_by(|a, b| {
        (
            &a.character_id,
            a.book_number,
            a.chapter_number,
            a.scene_order,
        )
            .cmp(&(
                &b.character_id,
                b.book_number,
                b.chapter_number,
                b.scene_order,
            ))
    });
    Ok(states)
}

/// Reconstructs the latest `CharacterState` for `character_id` at or before
/// `(book_number, chapter_number, scene_order)` from `CanonStateViewV1`,
/// matching `Repository::get_character_state_at_for_branch`.
pub fn project_character_state_at_from_canon(
    view: &CanonStateViewV1,
    character_id: &str,
    book_number: i32,
    chapter_number: i32,
    scene_order: i32,
) -> Result<Option<CharacterState>> {
    let states = project_character_states_from_canon(view)?;
    Ok(states
        .into_iter()
        .filter(|s| {
            s.character_id == character_id
                && (s.book_number, s.chapter_number, s.scene_order)
                    <= (book_number, chapter_number, scene_order)
        })
        .max_by_key(|s| {
            (
                s.book_number,
                s.chapter_number,
                s.scene_order,
                s.created_at,
                s.id.clone(),
            )
        }))
}

/// Reconstructs `WorldState` rows on a branch from `CanonStateViewV1`,
/// matching `Repository::list_world_states_by_project_and_branch`.
pub fn project_world_states_from_canon(view: &CanonStateViewV1) -> Result<Vec<WorldState>> {
    let mut states: Vec<WorldState> =
        extract_scalar_json_records(view, "legacy.world_state", true)?;
    states.sort_by(|a, b| {
        (branch_depth(view, &a.branch_id), &a.location_id, &a.id).cmp(&(
            branch_depth(view, &b.branch_id),
            &b.location_id,
            &b.id,
        ))
    });
    Ok(states)
}

/// Reconstructs `WorldRule` rows on a branch from `CanonStateViewV1`,
/// matching `Repository::list_world_rules_by_project_and_branch`.
pub fn project_world_rules_from_canon(view: &CanonStateViewV1) -> Result<Vec<WorldRule>> {
    let mut rules: Vec<WorldRule> = extract_scalar_json_records(view, "legacy.world_rule", true)?;
    rules.sort_by(|a, b| {
        (
            branch_depth(view, &a.branch_id),
            &a.rule_type,
            &a.rule_name,
            &a.id,
        )
            .cmp(&(
                branch_depth(view, &b.branch_id),
                &b.rule_type,
                &b.rule_name,
                &b.id,
            ))
    });
    Ok(rules)
}

/// Reconstructs `KnowledgeFact` rows on a branch from `CanonStateViewV1`,
/// matching `Repository::list_knowledge_facts_by_project_and_branch`.
pub fn project_knowledge_facts_from_canon(view: &CanonStateViewV1) -> Result<Vec<KnowledgeFact>> {
    let mut facts: Vec<KnowledgeFact> =
        extract_scalar_json_records(view, "legacy.knowledge_fact.", false)?;
    facts.sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
    Ok(facts)
}

/// Reconstructs `Knows` edge rows on a branch from `CanonStateViewV1`.
pub fn project_knows_from_canon(view: &CanonStateViewV1) -> Result<Vec<Knows>> {
    let mut edges: Vec<Knows> = extract_scalar_json_records(view, "legacy.knows.", false)?;
    edges.sort_by(|a, b| (&a.in_id, &a.out_id).cmp(&(&b.in_id, &b.out_id)));
    Ok(edges)
}

/// Reconstructs secret-circle memberships `(character_id, learned_at)` for a
/// secret `canonical_fact` from `CanonStateViewV1`, matching
/// `Repository::secret_circle_members`.
pub fn project_secret_circle_members_from_canon(
    view: &CanonStateViewV1,
    fact_id: &str,
) -> Result<Vec<(String, Option<StoredStoryPlacement>)>> {
    let facts = project_knowledge_facts_from_canon(view)?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for row in facts {
        if row.secret_of_fact_id.as_deref() == Some(fact_id)
            && seen.insert(row.character_id.clone())
        {
            out.push((row.character_id, row.learned_at));
        }
    }
    Ok(out)
}

/// Reconstructs `FutureKnowledge` rows on a branch from the visible displaced-time
/// (`TemporalScope::Future`) events in `lineage_events`. Because `TemporalScope::Future`
/// events never fold into `view.knowledge`, this reads only `view.visible_event_ids`.
pub fn project_future_knowledge_from_canon(
    view: &CanonStateViewV1,
    lineage_events: &[CanonEventV1],
) -> Result<Vec<FutureKnowledge>> {
    let visible_ids: BTreeSet<&str> = view.visible_event_ids.iter().map(String::as_str).collect();
    let mut by_natural_key: BTreeMap<String, FutureKnowledge> = BTreeMap::new();
    for ev in lineage_events {
        if !visible_ids.contains(ev.event_id.as_str()) {
            continue;
        }
        if let CanonEventPayloadV1::Assertion {
            assertion:
                ClaimAssertion::Attribute {
                    key,
                    value: ClaimScalar::Text { value },
                    ..
                },
            temporal_scope: TemporalScope::Future,
            ..
        } = &ev.payload
            && key.starts_with("legacy.future_knowledge.")
        {
            let fk: FutureKnowledge = serde_json::from_str(value)?;
            let nat_key = format!("{}|{}", fk.character_id, fk.knowledge_summary);
            by_natural_key.insert(nat_key, fk);
        }
    }
    let mut out: Vec<FutureKnowledge> = by_natural_key.into_values().collect();
    out.sort_by(|a, b| {
        (
            branch_depth(view, &a.branch_id),
            &a.character_id,
            &a.created_at,
            &a.id,
        )
            .cmp(&(
                branch_depth(view, &b.branch_id),
                &b.character_id,
                &b.created_at,
                &b.id,
            ))
    });
    Ok(out)
}

/// Reconstructs `StoredQuantityState` rows on a branch from `CanonStateViewV1`,
/// matching `Repository::list_quantity_states_by_project_and_branch`.
pub fn project_quantity_states_from_canon(
    view: &CanonStateViewV1,
) -> Result<Vec<crate::sqlite::records::StoredQuantityState>> {
    let mut rows: Vec<crate::sqlite::records::StoredQuantityState> =
        extract_scalar_json_records(view, "legacy.quantity_state.", false)?;
    rows.sort_by(|a, b| {
        (
            &a.subject_table,
            &a.subject_id,
            &a.measure,
            a.book_number,
            a.chapter_number,
            a.scene_order,
            &a.created_at,
        )
            .cmp(&(
                &b.subject_table,
                &b.subject_id,
                &b.measure,
                b.book_number,
                b.chapter_number,
                b.scene_order,
                &b.created_at,
            ))
    });
    Ok(rows)
}

/// Reconstructs `SystemOverlay` rows on a branch from `CanonStateViewV1`,
/// matching `Repository::list_system_overlays_by_project_and_branch`.
pub fn project_system_overlays_from_canon(view: &CanonStateViewV1) -> Result<Vec<SystemOverlay>> {
    let mut rows: Vec<SystemOverlay> =
        extract_scalar_json_records(view, "legacy.system_overlay", true)?;
    rows.sort_by(|a, b| {
        (branch_depth(view, &a.branch_id), &a.normalized_name, &a.id).cmp(&(
            branch_depth(view, &b.branch_id),
            &b.normalized_name,
            &b.id,
        ))
    });
    Ok(rows)
}

/// Reconstructs `TimelineEvent` rows on a branch from `CanonStateViewV1`,
/// matching `Repository::list_timeline_events_by_project_and_branch`.
pub fn project_timeline_events_from_canon(view: &CanonStateViewV1) -> Result<Vec<TimelineEvent>> {
    let mut rows: Vec<TimelineEvent> =
        extract_scalar_json_records(view, "legacy.timeline_event", true)?;
    rows.sort_by(|a, b| {
        (branch_depth(view, &a.branch_id), &a.title, &a.id).cmp(&(
            branch_depth(view, &b.branch_id),
            &b.title,
            &b.id,
        ))
    });
    Ok(rows)
}

/// Reconstructs `TemporalIntervention` rows on a branch from `CanonStateViewV1`,
/// matching `Repository::list_temporal_interventions_by_project_and_branch`.
pub fn project_temporal_interventions_from_canon(
    view: &CanonStateViewV1,
) -> Result<Vec<TemporalIntervention>> {
    let mut rows: Vec<TemporalIntervention> =
        extract_scalar_json_records(view, "legacy.temporal_intervention", true)?;
    rows.sort_by(|a, b| {
        (branch_depth(view, &a.branch_id), &a.title, &a.id).cmp(&(
            branch_depth(view, &b.branch_id),
            &b.title,
            &b.id,
        ))
    });
    Ok(rows)
}

/// Reconstructs `StoredSceneClock` rows from `CanonStateViewV1`.
pub fn project_scene_clocks_from_canon(
    view: &CanonStateViewV1,
    branch_id: &str,
) -> Result<Vec<crate::sqlite::records::StoredSceneClock>> {
    let mut rows: Vec<crate::sqlite::records::StoredSceneClock> =
        extract_scalar_json_records(view, "legacy.scene_clock", true)?;
    rows.retain(|r| r.branch_id == branch_id);
    rows.sort_by(|a, b| a.scene_id.cmp(&b.scene_id));
    Ok(rows)
}

/// Reconstructs `StoredTimelineEventClock` rows from `CanonStateViewV1`.
pub fn project_timeline_event_clocks_from_canon(
    view: &CanonStateViewV1,
    branch_id: &str,
) -> Result<Vec<crate::sqlite::records::StoredTimelineEventClock>> {
    let mut rows: Vec<crate::sqlite::records::StoredTimelineEventClock> =
        extract_scalar_json_records(view, "legacy.timeline_event_clock", true)?;
    rows.retain(|r| r.branch_id == branch_id);
    rows.sort_by(|a, b| a.timeline_event_id.cmp(&b.timeline_event_id));
    Ok(rows)
}

/// Reconstructs `NarrativePromise` rows on a branch from `CanonStateViewV1`,
/// matching `Repository::list_narrative_promises_by_project_and_branch`.
pub fn project_narrative_promises_from_canon(
    view: &CanonStateViewV1,
) -> Result<Vec<NarrativePromise>> {
    let mut rows: Vec<NarrativePromise> =
        extract_scalar_json_records(view, "legacy.narrative_promise", true)?;
    rows.sort_by(|a, b| {
        (branch_depth(view, &a.branch_id), &a.created_at, &a.id).cmp(&(
            branch_depth(view, &b.branch_id),
            &b.created_at,
            &b.id,
        ))
    });
    Ok(rows)
}

/// Reconstructs active (non-archived and open in `view.open_threads`) `NarrativePromise`
/// rows on a branch from `CanonStateViewV1`.
pub fn project_active_narrative_promises_from_canon(
    view: &CanonStateViewV1,
) -> Result<Vec<NarrativePromise>> {
    let mut rows = project_narrative_promises_from_canon(view)?;
    rows.retain(|r| r.archived_at.is_none() && view.open_threads.contains_key(&r.id));
    Ok(rows)
}

/// Reconstructs `RelatesTo` rows on a branch from `CanonStateViewV1`, overlaying
/// the pure reducer's `view.relationships` (`Observation + sum(Delta)`) `trust`
/// and `tension` totals so shadow comparisons verify the reducer's relationship math.
pub fn project_relationships_from_canon(view: &CanonStateViewV1) -> Result<Vec<RelatesTo>> {
    let mut by_pair: BTreeMap<(String, String), RelatesTo> = BTreeMap::new();
    for entity in view.entities.values() {
        for (attr_key, attr_state) in &entity.attributes {
            if !attr_key.starts_with("legacy.relates_to.") {
                continue;
            }
            if let Some(ClaimScalar::Text { value }) = &attr_state.scalar_value {
                let mut rel: RelatesTo = serde_json::from_str(value)?;
                let pair = relationship_pair_key(&rel.in_id, &rel.out_id);
                if let Some(reduced) = view
                    .relationship(&rel.in_id, &rel.out_id)
                    .or_else(|| view.relationship(&rel.out_id, &rel.in_id))
                {
                    if let Some(t) = reduced.resolved_trust {
                        rel.trust = t;
                    }
                    if let Some(t) = reduced.resolved_tension {
                        rel.tension = t;
                    }
                }
                by_pair.insert(pair, rel);
            }
        }
    }
    let mut out: Vec<RelatesTo> = by_pair.into_values().collect();
    out.sort_by(|a, b| {
        (
            branch_depth(view, &a.branch_id),
            &a.relationship_type,
            &a.in_id,
            &a.out_id,
        )
            .cmp(&(
                branch_depth(view, &b.branch_id),
                &b.relationship_type,
                &b.in_id,
                &b.out_id,
            ))
    });
    Ok(out)
}

fn list_knows_by_project_and_branch_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
) -> rusqlite::Result<Vec<Knows>> {
    let chain = resolve_branch_lineage_in_conn(conn, Some(project_id), branch_id)?;
    let sql = format!(
        "SELECT {KNOWS_COLUMNS} FROM knows \
         WHERE project_id = ?1 AND branch_id = ?2 \
         ORDER BY created_at ASC, in_id ASC, out_id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for candidate_branch_id in &chain {
        let rows = stmt
            .query_map(rusqlite::params![project_id, candidate_branch_id], |r| {
                Knows::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for row in rows {
            if seen.insert((row.in_id.clone(), row.out_id.clone())) {
                out.push(row);
            }
        }
    }
    out.sort_by(|a, b| (&a.in_id, &a.out_id).cmp(&(&b.in_id, &b.out_id)));
    Ok(out)
}

fn list_timeline_event_clocks_by_branch_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
) -> rusqlite::Result<Vec<crate::sqlite::records::StoredTimelineEventClock>> {
    let sql = format!(
        "SELECT {} FROM timeline_event_clock \
         WHERE project_id = ?1 AND branch_id = ?2 \
         ORDER BY timeline_event_id ASC",
        crate::sqlite::records::TIMELINE_EVENT_CLOCK_COLUMNS
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params![project_id, branch_id], |r| {
            crate::sqlite::records::StoredTimelineEventClock::try_from(r)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn compare_keyed_json_maps(
    store_or_view: &str,
    branch_id: &str,
    legacy_map: BTreeMap<String, (Option<String>, Value)>,
    projected_map: BTreeMap<String, (Option<String>, Value)>,
    differences: &mut Vec<CanonShadowParityDifferenceV1>,
) {
    let all_keys: BTreeSet<String> = legacy_map
        .keys()
        .chain(projected_map.keys())
        .cloned()
        .collect();
    for key in all_keys {
        match (legacy_map.get(&key), projected_map.get(&key)) {
            (Some((subj, leg_val)), Some((_, proj_val))) => {
                if leg_val != proj_val {
                    differences.push(CanonShadowParityDifferenceV1 {
                        store_or_view: store_or_view.to_string(),
                        branch_id: branch_id.to_string(),
                        subject_id: subj.clone(),
                        natural_key: key.clone(),
                        legacy_value: leg_val.clone(),
                        projected_value: proj_val.clone(),
                        detail: format!(
                            "{store_or_view} mismatch on branch {branch_id} for key '{key}'"
                        ),
                    });
                }
            }
            (Some((subj, leg_val)), None) => {
                differences.push(CanonShadowParityDifferenceV1 {
                    store_or_view: store_or_view.to_string(),
                    branch_id: branch_id.to_string(),
                    subject_id: subj.clone(),
                    natural_key: key.clone(),
                    legacy_value: leg_val.clone(),
                    projected_value: Value::Null,
                    detail: format!(
                        "{store_or_view} row '{key}' present in legacy read on branch {branch_id} but missing from canon projection"
                    ),
                });
            }
            (None, Some((subj, proj_val))) => {
                differences.push(CanonShadowParityDifferenceV1 {
                    store_or_view: store_or_view.to_string(),
                    branch_id: branch_id.to_string(),
                    subject_id: subj.clone(),
                    natural_key: key.clone(),
                    legacy_value: Value::Null,
                    projected_value: proj_val.clone(),
                    detail: format!(
                        "{store_or_view} row '{key}' present in canon projection on branch {branch_id} but missing from legacy read"
                    ),
                });
            }
            (None, None) => {}
        }
    }
}

impl Repository {
    /// Load a `SnapshotBatchContext` whose canon-backed collections (`world_rules`,
    /// `overlays`, `promises`, `relationships`, `events`, `canonical_facts`,
    /// `knowledge_facts`, `character_states`, `world_states`) are populated
    /// exclusively from the branch's canon compatibility projections.
    async fn load_canon_projected_snapshot_batch_context(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<SnapshotBatchContext> {
        let project = self.get_project(project_id).await?;
        let books = self.list_books_by_project(project_id).await?;
        let chapters = self.list_chapters_by_project(project_id).await?;
        let characters = self
            .list_characters_by_project_and_branch(project_id, branch_id)
            .await?;
        let locations = self
            .list_locations_by_project_and_branch(project_id, branch_id)
            .await?;
        let factions = self
            .list_factions_by_project_and_branch(project_id, branch_id)
            .await?;
        let religions = self
            .list_religions_by_project_and_branch(project_id, branch_id)
            .await?;
        let economies = self
            .list_economies_by_project_and_branch(project_id, branch_id)
            .await?;
        let plot_lines = self
            .list_plot_lines_by_project_and_branch(project_id, branch_id)
            .await?;
        let conflicts = self
            .list_conflicts_by_project_and_branch(project_id, branch_id)
            .await?;
        let themes = self
            .list_themes_by_project_and_branch(project_id, branch_id)
            .await?;
        let motifs = self
            .list_motifs_by_project_and_branch(project_id, branch_id)
            .await?;
        let arcs = self
            .list_character_arcs_by_project_and_branch(project_id, branch_id)
            .await?;
        let terms = self
            .list_terms_by_project_and_branch(project_id, branch_id)
            .await?;
        let scenes = self
            .list_scenes_by_project_and_branch(project_id, branch_id)
            .await?;
        let voice_profiles = self
            .list_voice_profiles_by_project_and_branch(project_id, branch_id)
            .await?;

        let view = self
            .canon_state_at(
                project_id,
                branch_id,
                CanonCursorV1::unbounded(),
                CanonTier::Ratified,
                None,
            )
            .await?;

        let world_rules = project_world_rules_from_canon(&view)?;
        let overlays = project_system_overlays_from_canon(&view)?;
        let promises = project_narrative_promises_from_canon(&view)?;
        let relationships = project_relationships_from_canon(&view)?;
        let events = project_timeline_events_from_canon(&view)?;
        let canonical_facts = project_canonical_facts_from_canon(&view)?;
        let knowledge_facts = project_knowledge_facts_from_canon(&view)?;
        let character_states = project_character_states_from_canon(&view)?;
        let world_states = project_world_states_from_canon(&view)?;

        Ok(SnapshotBatchContext {
            project,
            books,
            chapters,
            world_rules,
            characters,
            locations,
            factions,
            religions,
            economies,
            plot_lines,
            conflicts,
            themes,
            motifs,
            overlays,
            promises,
            arcs,
            terms,
            relationships,
            events,
            scenes,
            canonical_facts,
            knowledge_facts,
            character_states,
            voice_profiles,
            world_states,
        })
    }

    /// Assemble a `SubjectSnapshot` from the branch's canon compatibility projections
    /// + static entity metadata.
    pub async fn get_canon_projected_subject_snapshot(
        &self,
        project_id: &str,
        branch_id: &str,
        subject: &Subject,
        placement: Option<&StoryPlacement>,
    ) -> Result<SubjectSnapshot> {
        let batch = self
            .load_canon_projected_snapshot_batch_context(project_id, branch_id)
            .await?;
        let effective_placement = placement.cloned().unwrap_or(StoryPlacement {
            book_number: i32::MAX,
            chapter_number: i32::MAX,
            scene_order: Some(i32::MAX),
            note: None,
        });
        assemble_subject_snapshot_from_batch(&batch, subject, &effective_placement)
    }

    /// Compare all legacy canon/state reads and `SubjectSnapshot` projections
    /// against the compatibility projections over `CanonStateViewV1` + visible
    /// `CanonEventV1`s on `branch_id`.
    pub async fn compare_canon_compatibility_shadow_reads(
        &self,
        project_id: &str,
        branch_id: &str,
        placement: Option<StoryPlacement>,
    ) -> Result<CanonCompatibilityShadowReportV1> {
        let view = self
            .canon_state_at(
                project_id,
                branch_id,
                CanonCursorV1::unbounded(),
                CanonTier::Ratified,
                None,
            )
            .await?;
        let lineage_events = self
            .list_canon_events_for_lineage(project_id, branch_id)
            .await?;

        let mut differences = Vec::new();
        let mut checked_store_count = 0usize;

        // 1. canonical_fact
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_active_canonical_facts_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_canonical_facts_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    let key = format!(
                        "{}|{}|{}|{}",
                        r.subject_table,
                        r.subject_id.as_deref().unwrap_or(""),
                        r.predicate,
                        r.id
                    );
                    (
                        key,
                        (
                            r.subject_id.clone(),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    let key = format!(
                        "{}|{}|{}|{}",
                        r.subject_table,
                        r.subject_id.as_deref().unwrap_or(""),
                        r.predicate,
                        r.id
                    );
                    (
                        key,
                        (
                            r.subject_id.clone(),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "canonical_fact",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 2. character_state
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_character_states_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_character_states_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    let key = format!(
                        "{}|{}|{}|{}",
                        r.character_id, r.book_number, r.chapter_number, r.scene_order
                    );
                    (
                        key,
                        (
                            Some(r.character_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    let key = format!(
                        "{}|{}|{}|{}",
                        r.character_id, r.book_number, r.chapter_number, r.scene_order
                    );
                    (
                        key,
                        (
                            Some(r.character_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "character_state",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 3. world_state
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_world_states_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_world_states_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    (
                        r.location_id.clone(),
                        (
                            Some(r.location_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    (
                        r.location_id.clone(),
                        (
                            Some(r.location_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "world_state",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 4. world_rule
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_world_rules_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_world_rules_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    let key = format!("{}|{}", r.rule_type, r.rule_name);
                    (
                        key,
                        (
                            Some(r.id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    let key = format!("{}|{}", r.rule_type, r.rule_name);
                    (
                        key,
                        (
                            Some(r.id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps("world_rule", branch_id, leg_map, proj_map, &mut differences);
        }

        // 5. knowledge_fact + secret_circle_members
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_knowledge_facts_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_knowledge_facts_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    let key = format!("{}|{}", r.character_id, r.normalized_fact);
                    (
                        key,
                        (
                            Some(r.character_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    let key = format!("{}|{}", r.character_id, r.normalized_fact);
                    (
                        key,
                        (
                            Some(r.character_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "knowledge_fact",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 6. knows
        {
            checked_store_count += 1;
            let pid = project_id.to_string();
            let bid = branch_id.to_string();
            let legacy_rows = self
                .inner
                .pool
                .read(move |conn| list_knows_by_project_and_branch_in_conn(conn, &pid, &bid))
                .await?;
            let proj_rows = project_knows_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    let key = format!("{}|{}", r.in_id, r.out_id);
                    (
                        key,
                        (
                            Some(r.in_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    let key = format!("{}|{}", r.in_id, r.out_id);
                    (
                        key,
                        (
                            Some(r.in_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps("knows", branch_id, leg_map, proj_map, &mut differences);
        }

        // 7. future_knowledge (plus check that displaced-time propositions never
        //    fold into ordinary unconditional `view.knowledge`!)
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_future_knowledge_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_future_knowledge_from_canon(&view, &lineage_events)?;
            for fk in &legacy_rows {
                let prop_id = format!("proposition:future:{}", fk.id);
                if view.knowledge_status(&fk.character_id, &prop_id) == KnowledgeStatus::Known {
                    differences.push(CanonShadowParityDifferenceV1 {
                        store_or_view: "future_knowledge".to_string(),
                        branch_id: branch_id.to_string(),
                        subject_id: Some(fk.character_id.clone()),
                        natural_key: prop_id,
                        legacy_value: json!({ "ordinary_knowledge": false }),
                        projected_value: json!({ "ordinary_knowledge": true }),
                        detail: format!(
                            "future_knowledge {} leaked into ordinary unconditional view.knowledge",
                            fk.id
                        ),
                    });
                }
            }
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    let key = format!("{}|{}", r.character_id, r.knowledge_summary);
                    (
                        key,
                        (
                            Some(r.character_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    let key = format!("{}|{}", r.character_id, r.knowledge_summary);
                    (
                        key,
                        (
                            Some(r.character_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "future_knowledge",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 8. quantity_state
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_quantity_states_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_quantity_states_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    let key = format!(
                        "{}|{}|{}|{}|{}|{}|{}",
                        r.subject_table,
                        r.subject_id,
                        r.measure,
                        r.book_number,
                        r.chapter_number,
                        r.scene_order,
                        r.id
                    );
                    (
                        key,
                        (
                            Some(r.subject_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    let key = format!(
                        "{}|{}|{}|{}|{}|{}|{}",
                        r.subject_table,
                        r.subject_id,
                        r.measure,
                        r.book_number,
                        r.chapter_number,
                        r.scene_order,
                        r.id
                    );
                    (
                        key,
                        (
                            Some(r.subject_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "quantity_state",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 9. system_overlay
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_system_overlays_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_system_overlays_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    (
                        r.normalized_name.clone(),
                        (
                            Some(r.id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    (
                        r.normalized_name.clone(),
                        (
                            Some(r.id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "system_overlay",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 10. timeline_event
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_timeline_events_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_timeline_events_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    (
                        r.title.clone(),
                        (
                            Some(r.id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    (
                        r.title.clone(),
                        (
                            Some(r.id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "timeline_event",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 11. temporal_intervention
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_temporal_interventions_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_temporal_interventions_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    (
                        r.title.clone(),
                        (
                            Some(r.id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    (
                        r.title.clone(),
                        (
                            Some(r.id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "temporal_intervention",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 12. scene_clock
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_scene_clocks_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_scene_clocks_from_canon(&view, branch_id)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    (
                        r.scene_id.clone(),
                        (
                            Some(r.scene_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    (
                        r.scene_id.clone(),
                        (
                            Some(r.scene_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "scene_clock",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 13. timeline_event_clock
        {
            checked_store_count += 1;
            let pid = project_id.to_string();
            let bid = branch_id.to_string();
            let legacy_rows = self
                .inner
                .pool
                .read(move |conn| list_timeline_event_clocks_by_branch_in_conn(conn, &pid, &bid))
                .await?;
            let proj_rows = project_timeline_event_clocks_from_canon(&view, branch_id)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    (
                        r.timeline_event_id.clone(),
                        (
                            Some(r.timeline_event_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    (
                        r.timeline_event_id.clone(),
                        (
                            Some(r.timeline_event_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "timeline_event_clock",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 14. narrative_promise
        {
            checked_store_count += 1;
            let legacy_rows = self
                .list_narrative_promises_by_project_and_branch(project_id, branch_id)
                .await?;
            let proj_rows = project_narrative_promises_from_canon(&view)?;
            for r in &legacy_rows {
                if r.archived_at.is_some() && view.open_threads.contains_key(&r.id) {
                    differences.push(CanonShadowParityDifferenceV1 {
                        store_or_view: "narrative_promise".to_string(),
                        branch_id: branch_id.to_string(),
                        subject_id: Some(r.id.clone()),
                        natural_key: format!("{}|{}", r.promise_type, r.description),
                        legacy_value: json!({ "open_thread": false }),
                        projected_value: json!({ "open_thread": true }),
                        detail: format!(
                            "archived narrative_promise {} remained open in view.open_threads",
                            r.id
                        ),
                    });
                }
            }
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    let key = format!("{}|{}", r.promise_type, r.description);
                    (
                        key,
                        (
                            Some(r.id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    let key = format!("{}|{}", r.promise_type, r.description);
                    (
                        key,
                        (
                            Some(r.id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps(
                "narrative_promise",
                branch_id,
                leg_map,
                proj_map,
                &mut differences,
            );
        }

        // 15. relates_to (including relationship_scene_delta totals!)
        {
            checked_store_count += 1;
            let legacy_rows = self.list_relationships_by_branch(branch_id).await?;
            let proj_rows = project_relationships_from_canon(&view)?;
            let leg_map = legacy_rows
                .into_iter()
                .map(|r| {
                    let pair = relationship_pair_key(&r.in_id, &r.out_id);
                    let key = format!("{}|{}", pair.0, pair.1);
                    (
                        key,
                        (
                            Some(r.in_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            let proj_map = proj_rows
                .into_iter()
                .map(|r| {
                    let pair = relationship_pair_key(&r.in_id, &r.out_id);
                    let key = format!("{}|{}", pair.0, pair.1);
                    (
                        key,
                        (
                            Some(r.in_id.clone()),
                            serde_json::to_value(&r).unwrap_or(Value::Null),
                        ),
                    )
                })
                .collect();
            compare_keyed_json_maps("relates_to", branch_id, leg_map, proj_map, &mut differences);
        }

        // 16. subject_snapshot (for all subjects on the branch)
        let legacy_batch = self
            .load_snapshot_batch_context(project_id, branch_id)
            .await?;
        let projected_batch = self
            .load_canon_projected_snapshot_batch_context(project_id, branch_id)
            .await?;

        let mut subjects: Vec<Subject> = vec![Subject::project()];
        for c in &legacy_batch.characters {
            subjects.push(Subject::new(SubjectTable::Character, c.id.clone())?);
        }
        for l in &legacy_batch.locations {
            subjects.push(Subject::new(SubjectTable::Location, l.id.clone())?);
        }
        for f in &legacy_batch.factions {
            subjects.push(Subject::new(SubjectTable::Faction, f.id.clone())?);
        }
        for r in &legacy_batch.religions {
            subjects.push(Subject::new(SubjectTable::Religion, r.id.clone())?);
        }
        for e in &legacy_batch.economies {
            subjects.push(Subject::new(SubjectTable::Economy, e.id.clone())?);
        }
        for p in &legacy_batch.plot_lines {
            subjects.push(Subject::new(SubjectTable::PlotLine, p.id.clone())?);
        }
        for c in &legacy_batch.conflicts {
            subjects.push(Subject::new(SubjectTable::Conflict, c.id.clone())?);
        }
        for t in &legacy_batch.themes {
            subjects.push(Subject::new(SubjectTable::Theme, t.id.clone())?);
        }
        for m in &legacy_batch.motifs {
            subjects.push(Subject::new(SubjectTable::Motif, m.id.clone())?);
        }
        for o in &legacy_batch.overlays {
            subjects.push(Subject::new(SubjectTable::SystemOverlay, o.id.clone())?);
        }
        for np in &legacy_batch.promises {
            subjects.push(Subject::new(SubjectTable::NarrativePromise, np.id.clone())?);
        }
        for a in &legacy_batch.arcs {
            subjects.push(Subject::new(SubjectTable::CharacterArc, a.id.clone())?);
        }
        for wr in &legacy_batch.world_rules {
            subjects.push(Subject::new(SubjectTable::WorldRule, wr.id.clone())?);
        }
        for te in &legacy_batch.events {
            subjects.push(Subject::new(SubjectTable::TimelineEvent, te.id.clone())?);
        }
        for t in &legacy_batch.terms {
            subjects.push(Subject::new(SubjectTable::Term, t.id.clone())?);
        }

        let checked_subject_snapshot_count = subjects.len();
        checked_store_count += 1;

        let effective_placement = placement.clone().unwrap_or(StoryPlacement {
            book_number: i32::MAX,
            chapter_number: i32::MAX,
            scene_order: Some(i32::MAX),
            note: None,
        });

        for subject in &subjects {
            let leg_snap =
                assemble_subject_snapshot_from_batch(&legacy_batch, subject, &effective_placement)?;
            let proj_snap = assemble_subject_snapshot_from_batch(
                &projected_batch,
                subject,
                &effective_placement,
            )?;
            let leg_val = serde_json::to_value(&leg_snap)?;
            let proj_val = serde_json::to_value(&proj_snap)?;
            if leg_val != proj_val {
                let subj_key = format!(
                    "{}:{}",
                    subject.table().as_str(),
                    subject.id().unwrap_or("project")
                );
                differences.push(CanonShadowParityDifferenceV1 {
                    store_or_view: "subject_snapshot".to_string(),
                    branch_id: branch_id.to_string(),
                    subject_id: subject.id().map(str::to_string),
                    natural_key: subj_key.clone(),
                    legacy_value: leg_val,
                    projected_value: proj_val,
                    detail: format!(
                        "SubjectSnapshot mismatch on branch {branch_id} for {subj_key}"
                    ),
                });
            }
        }

        Ok(CanonCompatibilityShadowReportV1 {
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            placement,
            parity_clean: differences.is_empty(),
            checked_store_count,
            checked_subject_snapshot_count,
            differences,
        })
    }

    /// Evaluate per-project cutover readiness (Section 2 Stage B -> C gate).
    /// Blocks cutover for `project_id` if any legacy row is unmapped, any
    /// `canon_source_map` row is `Quarantined`, or any branch has a shadow-read
    /// parity difference. Other projects are unaffected.
    pub async fn evaluate_project_canon_cutover_readiness(
        &self,
        project_id: &str,
    ) -> Result<ProjectCutoverReadinessV1> {
        self.evaluate_project_canon_cutover_readiness_with_version(
            project_id,
            CANON_BACKFILL_MIGRATION_VERSION_V1,
        )
        .await
    }

    pub async fn evaluate_project_canon_cutover_readiness_with_version(
        &self,
        project_id: &str,
        migration_version: u32,
    ) -> Result<ProjectCutoverReadinessV1> {
        let pid = project_id.to_string();
        let (branches, unmapped_count, quarantined_entries) = self
            .inner
            .pool
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    let branches = super::list_branches_topological_in_conn(conn, &pid)?;
                    let mut unmapped = 0usize;
                    for branch in &branches {
                        let legacy = super::BranchLegacyRows::load(conn, &pid, &branch.id)?;
                        let snapshots = legacy.current_snapshots_by_key()?;
                        for ((source_table, pk), raw_snap) in snapshots {
                            let mapped = super::load_source_map_entry_in_tx(
                                conn,
                                &pid,
                                &source_table,
                                &pk,
                                migration_version,
                            )?;
                            match mapped {
                                Some(entry)
                                    if entry.branch_id == branch.id
                                        && entry.raw_snapshot_json == raw_snap => {}
                                _ => {
                                    unmapped += 1;
                                }
                            }
                        }
                    }
                    let mut stmt = conn.prepare(
                        "SELECT source_table, legacy_primary_key, quarantine_reason \
                         FROM canon_source_map \
                         WHERE project_id = ?1 AND migration_version = ?2 AND status = 'quarantined' \
                         ORDER BY source_table ASC, legacy_primary_key ASC",
                    )?;
                    let quarantined = stmt
                        .query_map(rusqlite::params![&pid, migration_version], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, Option<String>>(2)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok((branches, unmapped, quarantined))
                })())
            })
            .await??;

        let mut branch_reports = Vec::with_capacity(branches.len());
        let mut parity_clean = true;
        let mut blocking_reasons = Vec::new();

        if unmapped_count > 0 {
            blocking_reasons.push(format!(
                "{unmapped_count} legacy row(s) are unmapped or out-of-date in canon_source_map"
            ));
        }

        let quarantined_count = quarantined_entries.len();
        for (tbl, pk, reason) in &quarantined_entries {
            blocking_reasons.push(format!(
                "quarantined row in {tbl} ({pk}): {}",
                reason.as_deref().unwrap_or("unspecified quarantine reason")
            ));
        }

        for branch in &branches {
            let report = self
                .compare_canon_compatibility_shadow_reads(project_id, &branch.id, None)
                .await?;
            if !report.parity_clean {
                parity_clean = false;
                for diff in &report.differences {
                    blocking_reasons.push(diff.detail.clone());
                }
            }
            branch_reports.push(report);
        }

        let cutover_allowed = parity_clean && quarantined_count == 0 && unmapped_count == 0;

        Ok(ProjectCutoverReadinessV1 {
            project_id: project_id.to_string(),
            migration_version,
            parity_clean,
            quarantined_count,
            unmapped_legacy_row_count: unmapped_count,
            cutover_allowed,
            blocking_reasons,
            branch_reports,
        })
    }
}
