//! Legacy-to-canon backfill engine and `canon_source_map` reconciliation (P2 PR D).
//!
//! Maps every legacy store listed in `docs/canon-memory-design.md` Section 2 into
//! append-only `canon_claim`, `canon_decision`, and `canon_event` rows with
//! `CanonProvenanceKind::LegacyUnverified`, or quarantines opaque/unattributable
//! rows in `canon_source_map`.

pub mod shadow;
pub use shadow::*;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spindle_core::canon::*;
use spindle_core::models::{StoryClock, StoryPlacement, normalize_name};

use super::canon_events::{
    append_canon_decision_in_tx, load_events_for_branches_in_conn,
    materialize_projection_view_in_conn,
};
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct StoredRelationshipSceneDeltaRow {
    pub branch_id: String,
    pub in_id: String,
    pub out_id: String,
    pub scene_id: String,
    pub source: String,
    pub book_number: Option<i32>,
    pub chapter_number: Option<i32>,
    pub scene_order: Option<i32>,
    pub trust_delta: i32,
    pub tension_delta: i32,
    pub reason: Option<String>,
    pub updated_at: i64,
}

impl StoredRelationshipSceneDeltaRow {
    pub(crate) fn primary_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.branch_id, self.in_id, self.out_id, self.scene_id, self.source
        )
    }
}

fn enum_to_sql_str<T: serde::Serialize>(value: T) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("expected string-serialized enum variant"))
}

pub(crate) fn upsert_source_map_in_tx(
    tx: &rusqlite::Connection,
    entry: &CanonSourceMapEntryV1,
    now_micros: i64,
) -> Result<()> {
    let status_str = enum_to_sql_str(entry.status)?;
    let prov_str = enum_to_sql_str(entry.provenance)?;
    let claim_ids_json = serde_json::to_string(&entry.claim_ids)?;
    let event_ids_json = serde_json::to_string(&entry.event_ids)?;
    let created_at = if entry.created_at_micros > 0 {
        entry.created_at_micros
    } else {
        now_micros
    };
    tx.execute(
        "INSERT INTO canon_source_map (
            project_id, branch_id, source_table, legacy_primary_key,
            migration_version, status, provenance, claim_ids, event_ids,
            decision_id, quarantine_reason, raw_snapshot, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        ON CONFLICT(project_id, source_table, legacy_primary_key, migration_version)
        DO UPDATE SET
            branch_id = excluded.branch_id,
            status = excluded.status,
            provenance = excluded.provenance,
            claim_ids = excluded.claim_ids,
            event_ids = excluded.event_ids,
            decision_id = excluded.decision_id,
            quarantine_reason = excluded.quarantine_reason,
            raw_snapshot = excluded.raw_snapshot,
            updated_at = excluded.updated_at",
        rusqlite::params![
            &entry.project_id,
            &entry.branch_id,
            &entry.source_table,
            &entry.legacy_primary_key,
            entry.migration_version,
            &status_str,
            &prov_str,
            &claim_ids_json,
            &event_ids_json,
            &entry.decision_id,
            &entry.quarantine_reason,
            &entry.raw_snapshot_json,
            created_at,
            now_micros,
        ],
    )?;
    Ok(())
}

pub(crate) fn load_source_map_entry_in_tx(
    tx: &rusqlite::Connection,
    project_id: &str,
    source_table: &str,
    legacy_primary_key: &str,
    migration_version: u32,
) -> Result<Option<CanonSourceMapEntryV1>> {
    let row = tx
        .query_row(
            "SELECT project_id, branch_id, source_table, legacy_primary_key, \
                    migration_version, status, provenance, claim_ids, event_ids, \
                    decision_id, quarantine_reason, raw_snapshot, created_at, updated_at \
             FROM canon_source_map \
             WHERE project_id = ?1 AND source_table = ?2 \
               AND legacy_primary_key = ?3 AND migration_version = ?4",
            rusqlite::params![
                project_id,
                source_table,
                legacy_primary_key,
                migration_version
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u32>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, Option<String>>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, i64>(12)?,
                    r.get::<_, i64>(13)?,
                ))
            },
        )
        .optional_inner()?;

    let Some((
        pid,
        bid,
        tbl,
        pk,
        mig_ver,
        status_s,
        prov_s,
        claims_s,
        events_s,
        dec_id,
        q_reason,
        raw_snap,
        created_at,
        updated_at,
    )) = row
    else {
        return Ok(None);
    };

    Ok(Some(CanonSourceMapEntryV1 {
        project_id: pid,
        branch_id: bid,
        source_table: tbl,
        legacy_primary_key: pk,
        migration_version: mig_ver,
        status: serde_json::from_value(Value::String(status_s))?,
        provenance: serde_json::from_value(Value::String(prov_s))?,
        claim_ids: serde_json::from_str(&claims_s)?,
        event_ids: serde_json::from_str(&events_s)?,
        decision_id: dec_id,
        quarantine_reason: q_reason,
        raw_snapshot_json: raw_snap,
        created_at_micros: created_at,
        updated_at_micros: updated_at,
    }))
}

pub(crate) fn list_branches_topological_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
) -> Result<Vec<BibleBranch>> {
    let sql = format!(
        "SELECT {BIBLE_BRANCH_COLUMNS} FROM bible_branch \
         WHERE project_id = ?1 ORDER BY created_at ASC, id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut branches = stmt
        .query_map([project_id], |r| BibleBranch::try_from(r))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let parent_map: BTreeMap<String, Option<String>> = branches
        .iter()
        .map(|b| (b.id.clone(), b.parent_branch_id.clone()))
        .collect();

    let depth_of = |id: &str| -> usize {
        let mut depth = 0usize;
        let mut cur = parent_map.get(id).and_then(|p| p.as_deref());
        let mut visited = BTreeSet::new();
        while let Some(parent_id) = cur {
            if !visited.insert(parent_id) {
                break;
            }
            depth += 1;
            cur = parent_map.get(parent_id).and_then(|p| p.as_deref());
        }
        depth
    };

    branches.sort_by(|a, b| {
        depth_of(&a.id)
            .cmp(&depth_of(&b.id))
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(branches)
}

fn load_branch_scene_positions_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
) -> Result<BTreeMap<String, CanonPositionV1>> {
    let chain = resolve_branch_lineage_in_conn(conn, Some(project_id), branch_id)?;
    let sql = format!(
        "SELECT {SCENE_COLUMNS} FROM scene \
         WHERE project_id = ?1 AND branch_id = ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut out = BTreeMap::new();
    let mut seen_coords = BTreeSet::new();
    for candidate_branch_id in &chain {
        let rows = stmt
            .query_map(rusqlite::params![project_id, candidate_branch_id], |r| {
                Scene::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for scene in rows {
            if !seen_coords.insert((scene.book_number, scene.chapter_number, scene.scene_order)) {
                continue;
            }
            if let Some(pos) = coordinates_to_canon_position(
                Some(scene.book_number),
                Some(scene.chapter_number),
                Some(scene.scene_order),
            ) {
                out.entry(scene.id).or_insert(pos);
            }
        }
    }
    Ok(out)
}

#[derive(Default)]
struct BranchLegacyRows {
    canonical_facts: Vec<CanonicalFact>,
    character_states: Vec<CharacterState>,
    world_states: Vec<WorldState>,
    world_rules: Vec<WorldRule>,
    knowledge_facts: Vec<KnowledgeFact>,
    knows_edges: Vec<Knows>,
    future_knowledge: Vec<FutureKnowledge>,
    quantity_states: Vec<crate::sqlite::records::StoredQuantityState>,
    progression_events: Vec<ProgressionEvent>,
    system_overlays: Vec<SystemOverlay>,
    timeline_events: Vec<TimelineEvent>,
    temporal_interventions: Vec<TemporalIntervention>,
    scene_clocks: Vec<crate::sqlite::records::StoredSceneClock>,
    timeline_event_clocks: Vec<crate::sqlite::records::StoredTimelineEventClock>,
    narrative_promises: Vec<NarrativePromise>,
    relationships: Vec<RelatesTo>,
    relationship_deltas: Vec<StoredRelationshipSceneDeltaRow>,
    canon_deltas: Vec<crate::sqlite::records::StoredCanonDelta>,
}

impl BranchLegacyRows {
    fn load(conn: &rusqlite::Connection, project_id: &str, branch_id: &str) -> Result<Self> {
        let query_vec = |sql: &str| -> Result<()> {
            let _ = sql;
            Ok(())
        };
        let _ = query_vec;

        let canonical_facts = {
            let sql = format!(
                "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                CanonicalFact::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let character_states = {
            let sql = format!(
                "SELECT {CHARACTER_STATE_COLUMNS} FROM character_state \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY character_id ASC, book_number ASC, chapter_number ASC, scene_order ASC, created_at ASC, rowid ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                CharacterState::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let world_states = {
            let sql = format!(
                "SELECT {WORLD_STATE_COLUMNS} FROM world_state \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY location_id ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                WorldState::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let world_rules = {
            let sql = format!(
                "SELECT {WORLD_RULE_COLUMNS} FROM world_rule \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                WorldRule::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let knowledge_facts = {
            let sql = format!(
                "SELECT {KNOWLEDGE_FACT_COLUMNS} FROM knowledge_fact \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                KnowledgeFact::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let knows_edges = {
            let sql = format!(
                "SELECT {KNOWS_COLUMNS} FROM knows \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, in_id ASC, out_id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                Knows::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let future_knowledge = {
            let sql = format!(
                "SELECT {FUTURE_KNOWLEDGE_COLUMNS} FROM future_knowledge \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                FutureKnowledge::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let quantity_states = {
            let sql = format!(
                "SELECT {} FROM quantity_state \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY subject_table ASC, subject_id ASC, measure ASC, \
                          book_number ASC, chapter_number ASC, scene_order ASC, created_at ASC, id ASC",
                crate::sqlite::records::QUANTITY_STATE_COLUMNS
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                crate::sqlite::records::StoredQuantityState::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let progression_events = {
            let sql = format!(
                "SELECT {PROGRESSION_EVENT_COLUMNS} FROM progression_event \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                ProgressionEvent::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let system_overlays = {
            let sql = format!(
                "SELECT {SYSTEM_OVERLAY_COLUMNS} FROM system_overlay \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                SystemOverlay::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let timeline_events = {
            let sql = format!(
                "SELECT {TIMELINE_EVENT_COLUMNS} FROM timeline_event \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                TimelineEvent::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let temporal_interventions = {
            let sql = format!(
                "SELECT {TEMPORAL_INTERVENTION_COLUMNS} FROM temporal_intervention \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                TemporalIntervention::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let scene_clocks = {
            let sql = format!(
                "SELECT {} FROM scene_clock \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY scene_id ASC",
                crate::sqlite::records::SCENE_CLOCK_COLUMNS
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                crate::sqlite::records::StoredSceneClock::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let timeline_event_clocks = {
            let sql = format!(
                "SELECT {} FROM timeline_event_clock \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY timeline_event_id ASC",
                crate::sqlite::records::TIMELINE_EVENT_CLOCK_COLUMNS
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                crate::sqlite::records::StoredTimelineEventClock::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let narrative_promises = {
            let sql = format!(
                "SELECT {NARRATIVE_PROMISE_COLUMNS} FROM narrative_promise \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                NarrativePromise::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let relationships = {
            let sql = format!(
                "SELECT {RELATES_TO_COLUMNS} FROM relates_to \
                 WHERE branch_id = ?1 \
                 ORDER BY relationship_type ASC, in_id ASC, out_id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map([branch_id], |r| RelatesTo::try_from(r))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let relationship_deltas = {
            let mut stmt = conn.prepare(
                "SELECT branch_id, in_id, out_id, scene_id, source, \
                        book_number, chapter_number, scene_order, \
                        trust_delta, tension_delta, reason, updated_at \
                 FROM relationship_scene_delta \
                 WHERE branch_id = ?1 \
                 ORDER BY book_number ASC, chapter_number ASC, scene_order ASC, updated_at ASC, scene_id ASC, source ASC",
            )?;
            stmt.query_map([branch_id], |r| {
                Ok(StoredRelationshipSceneDeltaRow {
                    branch_id: r.get(0)?,
                    in_id: r.get(1)?,
                    out_id: r.get(2)?,
                    scene_id: r.get(3)?,
                    source: r.get(4)?,
                    book_number: r.get(5)?,
                    chapter_number: r.get(6)?,
                    scene_order: r.get(7)?,
                    trust_delta: r.get(8)?,
                    tension_delta: r.get(9)?,
                    reason: r.get(10)?,
                    updated_at: r.get(11)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let canon_deltas = {
            let sql = format!(
                "SELECT {} FROM canon_delta \
                 WHERE project_id = ?1 AND branch_id = ?2 \
                 ORDER BY created_at ASC, id ASC",
                crate::sqlite::records::CANON_DELTA_COLUMNS
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                crate::sqlite::records::StoredCanonDelta::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        Ok(Self {
            canonical_facts,
            character_states,
            world_states,
            world_rules,
            knowledge_facts,
            knows_edges,
            future_knowledge,
            quantity_states,
            progression_events,
            system_overlays,
            timeline_events,
            temporal_interventions,
            scene_clocks,
            timeline_event_clocks,
            narrative_promises,
            relationships,
            relationship_deltas,
            canon_deltas,
        })
    }

    fn current_snapshots_by_key(&self) -> Result<BTreeMap<(String, String), String>> {
        let mut map = BTreeMap::new();
        for r in &self.canonical_facts {
            map.insert(
                ("canonical_fact".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.character_states {
            map.insert(
                ("character_state".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.world_states {
            map.insert(
                ("world_state".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.world_rules {
            map.insert(
                ("world_rule".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.knowledge_facts {
            map.insert(
                ("knowledge_fact".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.knows_edges {
            let pk = format!("{}|{}|{}", r.branch_id, r.in_id, r.out_id);
            map.insert(("knows".to_string(), pk), serde_json::to_string(r)?);
        }
        for r in &self.future_knowledge {
            map.insert(
                ("future_knowledge".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.quantity_states {
            map.insert(
                ("quantity_state".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.progression_events {
            map.insert(
                ("progression_event".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.system_overlays {
            map.insert(
                ("system_overlay".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.timeline_events {
            map.insert(
                ("timeline_event".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.temporal_interventions {
            map.insert(
                ("temporal_intervention".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.scene_clocks {
            map.insert(
                ("scene_clock".to_string(), r.scene_id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.timeline_event_clocks {
            map.insert(
                (
                    "timeline_event_clock".to_string(),
                    r.timeline_event_id.clone(),
                ),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.narrative_promises {
            map.insert(
                ("narrative_promise".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.relationships {
            let pk = format!("{}|{}|{}", r.branch_id, r.in_id, r.out_id);
            let matching_deltas: Vec<&StoredRelationshipSceneDeltaRow> = self
                .relationship_deltas
                .iter()
                .filter(|d| {
                    relationship_pair_key(&d.in_id, &d.out_id)
                        == relationship_pair_key(&r.in_id, &r.out_id)
                })
                .collect();
            map.insert(
                ("relates_to".to_string(), pk),
                serde_json::to_string(&(r, matching_deltas))?,
            );
        }
        for r in &self.relationship_deltas {
            map.insert(
                ("relationship_scene_delta".to_string(), r.primary_key()),
                serde_json::to_string(r)?,
            );
        }
        for r in &self.canon_deltas {
            map.insert(
                ("canon_delta".to_string(), r.id.clone()),
                serde_json::to_string(r)?,
            );
        }
        Ok(map)
    }
}

fn reconcile_branch_source_maps_in_tx(
    tx: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
    migration_version: u32,
    current_snapshots: &BTreeMap<(String, String), String>,
    now_micros: i64,
) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT source_table, legacy_primary_key, event_ids, raw_snapshot \
         FROM canon_source_map \
         WHERE project_id = ?1 AND branch_id = ?2 AND migration_version = ?3",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![project_id, branch_id, migration_version],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut stale_event_ids = Vec::new();
    let mut stale_keys = Vec::new();
    for (source_table, pk, events_json, raw_snap) in rows {
        let key = (source_table, pk);
        let unchanged = current_snapshots
            .get(&key)
            .is_some_and(|curr_snap| curr_snap == &raw_snap);
        if !unchanged {
            let ev_ids: Vec<String> = serde_json::from_str(&events_json).unwrap_or_default();
            stale_event_ids.extend(ev_ids);
            stale_keys.push(key);
        }
    }

    if !stale_keys.is_empty() {
        for (source_table, pk) in &stale_keys {
            tx.execute(
                "DELETE FROM canon_source_map \
                 WHERE project_id = ?1 AND branch_id = ?2 AND source_table = ?3 \
                   AND legacy_primary_key = ?4 AND migration_version = ?5",
                rusqlite::params![project_id, branch_id, source_table, pk, migration_version],
            )?;
        }
    }

    stale_event_ids.sort();
    stale_event_ids.dedup();
    if !stale_event_ids.is_empty() {
        let inv_rev = format!("backfill:reconcile:{branch_id}:{now_micros}");
        let draft = CanonEventDraftV1 {
            subject: resolved_entity(format!("branch:{branch_id}")),
            payload: CanonEventPayloadV1::SourceInvalidation {
                invalidation: SourceInvalidationV1 {
                    scene_id: None,
                    invalidated_revision_ids: Vec::new(),
                    invalidated_event_ids: stale_event_ids,
                    replacement_revision_id: None,
                },
            },
            effective_position: None,
            within_scene_order: None,
            in_world_time: None,
            scene_id: None,
            source_claim_ids: Vec::new(),
            evidence_sha256: evidence_sha256(&[]),
            provenance: CanonProvenanceKind::SystemRestoration,
            supersedes_event_id: None,
            withdraws_event_id: None,
            legacy_record_scope: None,
            merge_provenance: None,
        };
        append_canon_decision_in_tx(
            tx,
            AppendCanonDecisionInputV1 {
                project_id: project_id.to_string(),
                branch_id: branch_id.to_string(),
                resolve_active_descendant_branch: false,
                tier: CanonTier::Ratified,
                action: CanonPolicyAction::InvalidateSource,
                actor: "system:canon_backfill".to_string(),
                policy_version: CANON_POLICY_VERSION_V1.to_string(),
                source_revision_id: inv_rev,
                target_scene_id: None,
                replace_prior_scene_revisions: false,
                previous_decision_id: None,
                confidence_inputs: Vec::new(),
                reason: "reconcile stale backfilled canon events after legacy change".to_string(),
                events: vec![draft],
            },
            now_micros,
        )?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_backfill_events_and_source_map_in_tx(
    tx: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
    store: CanonBackfillStoreKind,
    legacy_primary_key: &str,
    migration_version: u32,
    raw_snapshot_json: String,
    source_revision_id: String,
    target_scene_id: Option<String>,
    reason: String,
    drafts: Vec<CanonEventDraftV1>,
    counts: &mut CanonStoreBackfillCountsV1,
    now_micros: i64,
) -> Result<Vec<String>> {
    let mask_count = drafts
        .iter()
        .filter(|d| matches!(d.payload, CanonEventPayloadV1::CoverageMask { .. }))
        .count();
    let assertion_count = drafts.len().saturating_sub(mask_count);

    let out = append_canon_decision_in_tx(
        tx,
        AppendCanonDecisionInputV1 {
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            resolve_active_descendant_branch: false,
            tier: CanonTier::Ratified,
            action: CanonPolicyAction::Ratify,
            actor: "system:canon_backfill".to_string(),
            policy_version: CANON_POLICY_VERSION_V1.to_string(),
            source_revision_id,
            target_scene_id,
            replace_prior_scene_revisions: false,
            previous_decision_id: None,
            confidence_inputs: Vec::new(),
            reason,
            events: drafts,
        },
        now_micros,
    )?;

    let event_ids: Vec<String> = out.events.iter().map(|e| e.event_id.clone()).collect();
    let entry = CanonSourceMapEntryV1 {
        project_id: project_id.to_string(),
        branch_id: branch_id.to_string(),
        source_table: store.as_source_table().to_string(),
        legacy_primary_key: legacy_primary_key.to_string(),
        migration_version,
        status: CanonSourceMapStatus::Mapped,
        provenance: CanonProvenanceKind::LegacyUnverified,
        claim_ids: Vec::new(),
        event_ids: event_ids.clone(),
        decision_id: Some(out.decision.decision_id),
        quarantine_reason: None,
        raw_snapshot_json,
        created_at_micros: now_micros,
        updated_at_micros: now_micros,
    };
    upsert_source_map_in_tx(tx, &entry, now_micros)?;

    counts.mapped_count += 1;
    counts.emitted_event_count += assertion_count;
    counts.emitted_mask_count += mask_count;
    Ok(event_ids)
}

fn check_already_mapped_in_tx(
    tx: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
    store: CanonBackfillStoreKind,
    legacy_primary_key: &str,
    migration_version: u32,
    raw_snapshot_json: &str,
    counts: &mut CanonStoreBackfillCountsV1,
    quarantined_out: &mut Vec<CanonSourceMapEntryV1>,
) -> Result<Option<CanonSourceMapEntryV1>> {
    if let Some(existing) = load_source_map_entry_in_tx(
        tx,
        project_id,
        store.as_source_table(),
        legacy_primary_key,
        migration_version,
    )? && existing.branch_id == branch_id
        && existing.raw_snapshot_json == raw_snapshot_json
    {
        counts.reused_idempotent_count += 1;
        if existing.status == CanonSourceMapStatus::Quarantined {
            quarantined_out.push(existing.clone());
        }
        return Ok(Some(existing));
    }
    Ok(None)
}

fn legacy_whole_record_mask_draft(
    record_kind: &str,
    subject_id: &str,
    natural_key: &str,
    tombstone: bool,
    _reason: &str,
) -> CanonEventDraftV1 {
    CanonEventDraftV1 {
        subject: resolved_entity(subject_id),
        payload: CanonEventPayloadV1::CoverageMask {
            mask: StreamCoverageMaskV1 {
                target: CoverageMaskTargetV1::LegacyWholeRecord {
                    record_kind: record_kind.to_string(),
                    subject_id: subject_id.to_string(),
                    natural_key: natural_key.to_string(),
                },
                start_position: None,
                end_position: None,
                tombstone,
            },
        },
        effective_position: None,
        within_scene_order: None,
        in_world_time: None,
        scene_id: None,
        source_claim_ids: Vec::new(),
        evidence_sha256: evidence_sha256(&[]),
        provenance: CanonProvenanceKind::LegacyUnverified,
        supersedes_event_id: None,
        withdraws_event_id: None,
        legacy_record_scope: None,
        merge_provenance: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn assertion_draft(
    subject_id: &str,
    assertion: ClaimAssertion,
    temporal_scope: TemporalScope,
    effective_position: Option<CanonPositionV1>,
    within_scene_order: Option<u32>,
    in_world_time: Option<StoryClock>,
    scene_id: Option<String>,
    supersedes_event_id: Option<String>,
    record_kind: &str,
    natural_key: &str,
) -> CanonEventDraftV1 {
    CanonEventDraftV1 {
        subject: resolved_entity(subject_id),
        payload: CanonEventPayloadV1::Assertion {
            assertion,
            modality: ClaimModality::Narration,
            temporal_scope,
            speaker: None,
            experiencer: None,
        },
        effective_position,
        within_scene_order,
        in_world_time,
        scene_id,
        source_claim_ids: Vec::new(),
        evidence_sha256: evidence_sha256(&[]),
        provenance: CanonProvenanceKind::LegacyUnverified,
        supersedes_event_id,
        withdraws_event_id: None,
        legacy_record_scope: Some(LegacyRecordScopeV1 {
            record_kind: record_kind.to_string(),
            natural_key: natural_key.to_string(),
        }),
        merge_provenance: None,
    }
}

fn canonical_fact_scalar(fact: &CanonicalFact) -> ClaimScalar {
    if let Some(num) = fact.value_number {
        let rendered = if num.fract() == 0.0 {
            format!("{}", num as i64)
        } else {
            format!("{num}")
        };
        return ClaimScalar::Decimal {
            value: rendered,
            unit: fact.unit.clone(),
        };
    }
    if let Some(Value::Bool(b)) = &fact.value_json {
        return ClaimScalar::Boolean { value: *b };
    }
    let text = fact
        .value_text
        .clone()
        .or_else(|| fact.value_json.as_ref().map(Value::to_string))
        .unwrap_or_default();
    ClaimScalar::Text { value: text }
}

fn scope_to_attribute_nature(scope: &str) -> AttributeNature {
    match scope.trim().to_ascii_lowercase().as_str() {
        "invariant" => AttributeNature::Invariant,
        "evolving" => AttributeNature::Evolving,
        "conditional" => AttributeNature::Conditional,
        _ => AttributeNature::Unknown,
    }
}

#[allow(clippy::too_many_arguments)]
fn backfill_branch_in_tx(
    tx: &rusqlite::Connection,
    project_id: &str,
    branch: &BibleBranch,
    migration_version: u32,
    legacy: &BranchLegacyRows,
    scene_positions: &BTreeMap<String, CanonPositionV1>,
    report: &mut CanonBackfillReportV1,
    now_micros: i64,
) -> Result<()> {
    let branch_id = &branch.id;
    let is_non_root = branch.parent_branch_id.is_some();

    // -------------------------------------------------------------------------
    // 1. canonical_fact
    // -------------------------------------------------------------------------
    {
        let store = CanonBackfillStoreKind::CanonicalFact;
        let mut counts = CanonStoreBackfillCountsV1::default();
        let mut fact_event_ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut newly_mapped_fact_ids: BTreeSet<String> = BTreeSet::new();

        for fact in &legacy.canonical_facts {
            let raw_snap = serde_json::to_string(fact)?;
            if let Some(existing) = check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &fact.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )? {
                fact_event_ids.insert(fact.id.clone(), existing.event_ids);
                continue;
            }

            let natural_key = format!(
                "{}|{}|{}",
                fact.subject_table,
                fact.subject_id.as_deref().unwrap_or(""),
                fact.predicate
            );
            let subject_id = fact
                .subject_id
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| format!("project:{project_id}"));

            let effective_pos = fact
                .scene_id
                .as_ref()
                .and_then(|sid| scene_positions.get(sid).copied())
                .or_else(|| {
                    fact.valid_from
                        .as_ref()
                        .map(|p| p.clone().into_core())
                        .and_then(|p| story_placement_to_canon_position(&p))
                });

            let nature = scope_to_attribute_nature(&fact.scope);
            let pred_event_id = fact
                .supersedes_fact_id
                .as_ref()
                .and_then(|old_id| fact_event_ids.get(old_id))
                .and_then(|ids| ids.first().cloned());
            let pred_meta_event_id = fact
                .supersedes_fact_id
                .as_ref()
                .and_then(|old_id| fact_event_ids.get(old_id))
                .and_then(|ids| ids.get(1).cloned());

            let mut drafts = vec![
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: fact.predicate.clone(),
                        value: canonical_fact_scalar(fact),
                        nature,
                    },
                    TemporalScope::Current,
                    effective_pos,
                    Some(0),
                    None,
                    fact.scene_id.clone(),
                    pred_event_id,
                    "canonical_fact",
                    &natural_key,
                ),
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: format!("legacy.canonical_fact.{}", fact.id),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature,
                    },
                    TemporalScope::Current,
                    None,
                    Some(1),
                    None,
                    fact.scene_id.clone(),
                    pred_meta_event_id,
                    "canonical_fact",
                    &natural_key,
                ),
            ];

            if is_non_root && fact.superseded_by.is_none() {
                drafts.push(legacy_whole_record_mask_draft(
                    "canonical_fact",
                    &subject_id,
                    &natural_key,
                    false,
                    "child branch canonical_fact natural-key override",
                ));
            }

            let ev_ids = emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &fact.id,
                migration_version,
                raw_snap,
                format!("backfill:canonical_fact:{}", fact.id),
                fact.scene_id.clone(),
                format!("backfill canonical_fact {}", fact.id),
                drafts,
                &mut counts,
                now_micros,
            )?;
            fact_event_ids.insert(fact.id.clone(), ev_ids);
            newly_mapped_fact_ids.insert(fact.id.clone());
        }

        // Any newly mapped canonical_fact with `superseded_by IS NOT NULL` has its
        // assertion events suppressed so only active facts survive in `CanonStateViewV1`.
        for fact in &legacy.canonical_facts {
            if fact.superseded_by.is_some()
                && newly_mapped_fact_ids.contains(&fact.id)
                && let Some(ev_ids) = fact_event_ids.get(&fact.id)
                && !ev_ids.is_empty()
            {
                let inv_rev = format!(
                    "backfill:canonical_fact:superseded:{}:{}",
                    fact.id,
                    fact.superseded_by.as_deref().unwrap_or("unknown")
                );
                let draft = CanonEventDraftV1 {
                    subject: resolved_entity(
                        fact.subject_id
                            .clone()
                            .unwrap_or_else(|| format!("project:{project_id}")),
                    ),
                    payload: CanonEventPayloadV1::SourceInvalidation {
                        invalidation: SourceInvalidationV1 {
                            scene_id: fact.scene_id.clone(),
                            invalidated_revision_ids: Vec::new(),
                            invalidated_event_ids: ev_ids.clone(),
                            replacement_revision_id: fact
                                .superseded_by
                                .as_ref()
                                .map(|new_id| format!("backfill:canonical_fact:{new_id}")),
                        },
                    },
                    effective_position: None,
                    within_scene_order: None,
                    in_world_time: None,
                    scene_id: fact.scene_id.clone(),
                    source_claim_ids: Vec::new(),
                    evidence_sha256: evidence_sha256(&[]),
                    provenance: CanonProvenanceKind::LegacyUnverified,
                    supersedes_event_id: None,
                    withdraws_event_id: None,
                    legacy_record_scope: None,
                    merge_provenance: None,
                };
                append_canon_decision_in_tx(
                    tx,
                    AppendCanonDecisionInputV1 {
                        project_id: project_id.to_string(),
                        branch_id: branch_id.to_string(),
                        resolve_active_descendant_branch: false,
                        tier: CanonTier::Ratified,
                        action: CanonPolicyAction::Supersede,
                        actor: "system:canon_backfill".to_string(),
                        policy_version: CANON_POLICY_VERSION_V1.to_string(),
                        source_revision_id: inv_rev,
                        target_scene_id: fact.scene_id.clone(),
                        replace_prior_scene_revisions: false,
                        previous_decision_id: None,
                        confidence_inputs: Vec::new(),
                        reason: format!("supersede canonical_fact {}", fact.id),
                        events: vec![draft],
                    },
                    now_micros,
                )?;
            }
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 2. character_state
    // -------------------------------------------------------------------------
    let mut state_positions_with_status: BTreeSet<(String, Option<CanonPositionV1>)> =
        BTreeSet::new();
    {
        let store = CanonBackfillStoreKind::CharacterState;
        let mut counts = CanonStoreBackfillCountsV1::default();

        let mut by_char: BTreeMap<String, Vec<&CharacterState>> = BTreeMap::new();
        for state in &legacy.character_states {
            by_char
                .entry(state.character_id.clone())
                .or_default()
                .push(state);
        }

        for (character_id, states) in by_char {
            let mut prev_status: Vec<String> = Vec::new();
            let mut prev_goals: BTreeSet<String> = BTreeSet::new();
            let mut prev_notes: BTreeSet<String> = BTreeSet::new();
            let mut prev_pos_meta_event: BTreeMap<(i32, i32, i32), String> = BTreeMap::new();

            for state in states {
                let eff_pos = coordinates_to_canon_position(
                    Some(state.book_number),
                    Some(state.chapter_number),
                    Some(state.scene_order),
                );
                if !state.status.is_empty() {
                    state_positions_with_status.insert((character_id.clone(), eff_pos));
                }

                let raw_snap = serde_json::to_string(state)?;
                if let Some(existing) = check_already_mapped_in_tx(
                    tx,
                    project_id,
                    branch_id,
                    store,
                    &state.id,
                    migration_version,
                    &raw_snap,
                    &mut counts,
                    &mut report.quarantined_entries,
                )? {
                    prev_status = state.status.clone();
                    prev_goals = state.goals.iter().cloned().collect();
                    prev_notes = state.notes.iter().cloned().collect();
                    if let Some(last_ev) = existing.event_ids.last() {
                        prev_pos_meta_event.insert(
                            (state.book_number, state.chapter_number, state.scene_order),
                            last_ev.clone(),
                        );
                    }
                    continue;
                }

                let natural_key = format!(
                    "{}|{}|{}|{}",
                    state.character_id, state.book_number, state.chapter_number, state.scene_order
                );
                let mut assertions =
                    diff_character_status_snapshots(&character_id, &prev_status, &state.status);

                let curr_goals: BTreeSet<String> = state.goals.iter().cloned().collect();
                for removed_goal in prev_goals.difference(&curr_goals) {
                    assertions.push(ClaimAssertion::Attribute {
                        operation: AttributeOperation::Remove,
                        key: "character_state.goals".to_string(),
                        value: ClaimScalar::Text {
                            value: removed_goal.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    });
                }
                for added_goal in curr_goals.difference(&prev_goals) {
                    assertions.push(ClaimAssertion::Attribute {
                        operation: AttributeOperation::Add,
                        key: "character_state.goals".to_string(),
                        value: ClaimScalar::Text {
                            value: added_goal.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    });
                }

                let curr_notes: BTreeSet<String> = state.notes.iter().cloned().collect();
                for removed_note in prev_notes.difference(&curr_notes) {
                    assertions.push(ClaimAssertion::Attribute {
                        operation: AttributeOperation::Remove,
                        key: "character_state.notes".to_string(),
                        value: ClaimScalar::Text {
                            value: removed_note.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    });
                }
                for added_note in curr_notes.difference(&prev_notes) {
                    assertions.push(ClaimAssertion::Attribute {
                        operation: AttributeOperation::Add,
                        key: "character_state.notes".to_string(),
                        value: ClaimScalar::Text {
                            value: added_note.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    });
                }

                for (emotion, level) in &state.emotional_state {
                    let scalar = if let Some(num) = level.as_f64() {
                        ClaimScalar::Decimal {
                            value: num.to_string(),
                            unit: None,
                        }
                    } else if let Some(b) = level.as_bool() {
                        ClaimScalar::Boolean { value: b }
                    } else if let Some(s) = level.as_str() {
                        ClaimScalar::Text {
                            value: s.to_owned(),
                        }
                    } else {
                        ClaimScalar::Text {
                            value: level.to_string(),
                        }
                    };
                    assertions.push(ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: format!("character_state.emotional.{emotion}"),
                        value: scalar,
                        nature: AttributeNature::Evolving,
                    });
                }
                if let Some(summary) = &state.source_summary {
                    assertions.push(ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: "character_state.source_summary".to_string(),
                        value: ClaimScalar::Text {
                            value: summary.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    });
                }

                let pos_key = (state.book_number, state.chapter_number, state.scene_order);
                let supersedes_meta = prev_pos_meta_event.get(&pos_key).cloned();

                let mut drafts = Vec::new();
                for (step_idx, assertion) in assertions.into_iter().enumerate() {
                    drafts.push(assertion_draft(
                        &character_id,
                        assertion,
                        TemporalScope::Current,
                        eff_pos,
                        Some(step_idx as u32),
                        None,
                        state.scene_id.clone(),
                        None,
                        "character_state",
                        &natural_key,
                    ));
                }
                let meta_step = drafts.len() as u32;
                drafts.push(assertion_draft(
                    &character_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: format!(
                            "legacy.character_state.snapshot.{}.{}.{}",
                            state.book_number, state.chapter_number, state.scene_order
                        ),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    TemporalScope::Current,
                    eff_pos,
                    Some(meta_step),
                    None,
                    state.scene_id.clone(),
                    supersedes_meta,
                    "character_state",
                    &natural_key,
                ));

                if is_non_root {
                    drafts.push(legacy_whole_record_mask_draft(
                        "character_state",
                        &character_id,
                        &natural_key,
                        false,
                        "child branch character_state position override",
                    ));
                }

                let ev_ids = emit_backfill_events_and_source_map_in_tx(
                    tx,
                    project_id,
                    branch_id,
                    store,
                    &state.id,
                    migration_version,
                    raw_snap,
                    format!("backfill:character_state:{}", state.id),
                    state.scene_id.clone(),
                    format!("backfill character_state {}", state.id),
                    drafts,
                    &mut counts,
                    now_micros,
                )?;
                let non_mask_ev = ev_ids
                    .iter()
                    .rev()
                    .skip(usize::from(is_non_root))
                    .next()
                    .cloned();
                if let Some(meta_id) = non_mask_ev {
                    prev_pos_meta_event.insert(pos_key, meta_id);
                }

                prev_status = state.status.clone();
                prev_goals = curr_goals;
                prev_notes = curr_notes;
            }
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 3. world_state
    // -------------------------------------------------------------------------
    {
        let store = CanonBackfillStoreKind::WorldState;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for ws in &legacy.world_states {
            let raw_snap = serde_json::to_string(ws)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &ws.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = ws.location_id.clone();
            let subject_id = ws.location_id.clone();
            let mut drafts = Vec::new();
            let mut step = 0u32;

            for (field_key, field_val) in [
                ("world_state.status", ws.status.clone()),
                (
                    "world_state.controlling_faction",
                    ws.controlling_faction.clone(),
                ),
                ("world_state.prosperity", ws.prosperity.clone()),
                ("world_state.stability", ws.stability.clone()),
                ("world_state.threat_level", ws.threat_level.clone()),
            ] {
                if let Some(val) = field_val {
                    drafts.push(assertion_draft(
                        &subject_id,
                        ClaimAssertion::Attribute {
                            operation: AttributeOperation::Set,
                            key: field_key.to_string(),
                            value: ClaimScalar::Text { value: val },
                            nature: AttributeNature::Evolving,
                        },
                        TemporalScope::Current,
                        None,
                        Some(step),
                        None,
                        None,
                        None,
                        "world_state",
                        &natural_key,
                    ));
                    step += 1;
                }
            }
            for detail in &ws.sensory_details {
                drafts.push(assertion_draft(
                    &subject_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Add,
                        key: "world_state.sensory_details".to_string(),
                        value: ClaimScalar::Text {
                            value: detail.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    TemporalScope::Current,
                    None,
                    Some(step),
                    None,
                    None,
                    None,
                    "world_state",
                    &natural_key,
                ));
                step += 1;
            }
            drafts.push(assertion_draft(
                &subject_id,
                ClaimAssertion::Attribute {
                    operation: AttributeOperation::Set,
                    key: "legacy.world_state".to_string(),
                    value: ClaimScalar::Text {
                        value: raw_snap.clone(),
                    },
                    nature: AttributeNature::Evolving,
                },
                TemporalScope::Current,
                None,
                Some(step),
                None,
                None,
                None,
                "world_state",
                &natural_key,
            ));

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "world_state",
                    &subject_id,
                    &natural_key,
                    false,
                    "child branch world_state location override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &ws.id,
                migration_version,
                raw_snap,
                format!("backfill:world_state:{}", ws.id),
                None,
                format!("backfill world_state {}", ws.id),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 4. world_rule
    // -------------------------------------------------------------------------
    {
        let store = CanonBackfillStoreKind::WorldRule;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for rule in &legacy.world_rules {
            let raw_snap = serde_json::to_string(rule)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &rule.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = format!("{}|{}", rule.rule_type, rule.rule_name);
            let subject_id = rule.id.clone();

            let eff_pos: Option<CanonPositionV1> = None;

            let mut drafts = vec![
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Rule {
                        operation: RuleOperation::Define,
                        rule: resolved_entity(&rule.id),
                        anchor: None,
                        preconditions: rule.relevance_tags.clone().unwrap_or_default(),
                        effects: vec![rule.description.clone()],
                    },
                    TemporalScope::Current,
                    eff_pos,
                    Some(0),
                    None,
                    None,
                    None,
                    "world_rule",
                    &natural_key,
                ),
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: "legacy.world_rule".to_string(),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature: AttributeNature::Invariant,
                    },
                    TemporalScope::Current,
                    None,
                    Some(1),
                    None,
                    None,
                    None,
                    "world_rule",
                    &natural_key,
                ),
            ];

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "world_rule",
                    &subject_id,
                    &natural_key,
                    false,
                    "child branch world_rule natural-key override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &rule.id,
                migration_version,
                raw_snap,
                format!("backfill:world_rule:{}", rule.id),
                None,
                format!("backfill world_rule {}", rule.id),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 5. knowledge_fact + 6. knows (deduplicated propositions)
    // -------------------------------------------------------------------------
    let mut fact_id_to_proposition: BTreeMap<String, String> = BTreeMap::new();
    {
        // Load all knowledge_fact rows across the project so `knows.out_id` can
        // resolve inherited knowledge_fact IDs to the same deduplicated proposition!
        let mut stmt = tx.prepare(
            "SELECT id, normalized_fact, secret_of_fact_id \
             FROM knowledge_fact WHERE project_id = ?1",
        )?;
        let rows = stmt
            .query_map([project_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, norm_fact, secret_id) in rows {
            let prop_id = deduplicated_knowledge_proposition_id(&norm_fact, secret_id.as_deref());
            fact_id_to_proposition.insert(id, prop_id);
        }
    }

    {
        let store = CanonBackfillStoreKind::KnowledgeFact;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for kf in &legacy.knowledge_facts {
            let raw_snap = serde_json::to_string(kf)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &kf.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = format!("{}|{}", kf.character_id, kf.normalized_fact);
            let prop_id = deduplicated_knowledge_proposition_id(
                &kf.normalized_fact,
                kf.secret_of_fact_id.as_deref(),
            );
            let eff_pos = kf
                .learned_at
                .as_ref()
                .map(|p| p.clone().into_core())
                .and_then(|p| story_placement_to_canon_position(&p));

            let mut drafts = vec![
                assertion_draft(
                    &kf.character_id,
                    ClaimAssertion::Knowledge {
                        operation: KnowledgeOperation::Learn,
                        proposition: resolved_entity(&prop_id),
                        audience: Vec::new(),
                        source: None,
                    },
                    TemporalScope::Current,
                    eff_pos,
                    Some(0),
                    None,
                    None,
                    None,
                    "knowledge_fact",
                    &natural_key,
                ),
                assertion_draft(
                    &kf.character_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: format!("legacy.knowledge_fact.{}", kf.id),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    TemporalScope::Current,
                    None,
                    Some(1),
                    None,
                    None,
                    None,
                    "knowledge_fact",
                    &natural_key,
                ),
            ];

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "knowledge_fact",
                    &kf.character_id,
                    &natural_key,
                    false,
                    "child branch knowledge_fact natural-key override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &kf.id,
                migration_version,
                raw_snap,
                format!("backfill:knowledge_fact:{}", kf.id),
                None,
                format!("backfill knowledge_fact {}", kf.id),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    {
        let store = CanonBackfillStoreKind::Knows;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for knows in &legacy.knows_edges {
            let pk = format!("{}|{}|{}", knows.branch_id, knows.in_id, knows.out_id);
            let raw_snap = serde_json::to_string(knows)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &pk,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = format!("{}|{}", knows.in_id, knows.out_id);
            let prop_id = fact_id_to_proposition
                .get(&knows.out_id)
                .cloned()
                .unwrap_or_else(|| format!("proposition:fact_id:{}", knows.out_id));
            let eff_pos = knows
                .learned_at
                .as_ref()
                .map(|p| p.clone().into_core())
                .and_then(|p| story_placement_to_canon_position(&p));

            let mut drafts = vec![
                assertion_draft(
                    &knows.in_id,
                    ClaimAssertion::Knowledge {
                        operation: KnowledgeOperation::Learn,
                        proposition: resolved_entity(&prop_id),
                        audience: Vec::new(),
                        source: None,
                    },
                    TemporalScope::Current,
                    eff_pos,
                    Some(0),
                    None,
                    None,
                    None,
                    "knows",
                    &natural_key,
                ),
                assertion_draft(
                    &knows.in_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: format!("legacy.knows.{}", knows.out_id),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    TemporalScope::Current,
                    None,
                    Some(1),
                    None,
                    None,
                    None,
                    "knows",
                    &natural_key,
                ),
            ];

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "knows",
                    &knows.in_id,
                    &natural_key,
                    false,
                    "child branch knows natural-key override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &pk,
                migration_version,
                raw_snap,
                format!("backfill:knows:{pk}"),
                None,
                format!("backfill knows {pk}"),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 7. future_knowledge (displaced-time TemporalScope::Future; never folds
    //    into ordinary unconditional knowledge)
    // -------------------------------------------------------------------------
    {
        let store = CanonBackfillStoreKind::FutureKnowledge;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for fk in &legacy.future_knowledge {
            let raw_snap = serde_json::to_string(fk)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &fk.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = format!("{}|{}", fk.character_id, fk.knowledge_summary);
            let learned_pos = story_placement_to_canon_position(&fk.learned_at.clone().into_core());
            let expires_pos = fk
                .expires_at
                .as_ref()
                .map(|p| p.clone().into_core())
                .and_then(|p| story_placement_to_canon_position(&p));
            let prop_id = format!("proposition:future:{}", fk.id);
            let source_entity =
                resolved_entity(format!("temporal_source:{}", normalize_name(&fk.source)));

            let mut drafts = vec![
                assertion_draft(
                    &fk.character_id,
                    ClaimAssertion::Knowledge {
                        operation: KnowledgeOperation::Learn,
                        proposition: resolved_entity(&prop_id),
                        audience: Vec::new(),
                        source: Some(source_entity.clone()),
                    },
                    TemporalScope::Future,
                    learned_pos,
                    Some(0),
                    None,
                    None,
                    None,
                    "future_knowledge",
                    &natural_key,
                ),
                assertion_draft(
                    &fk.character_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: format!("legacy.future_knowledge.{}", fk.id),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature: AttributeNature::Conditional,
                    },
                    TemporalScope::Future,
                    learned_pos,
                    Some(1),
                    None,
                    None,
                    None,
                    "future_knowledge",
                    &natural_key,
                ),
            ];

            if let Some(exp_pos) = expires_pos {
                drafts.push(assertion_draft(
                    &fk.character_id,
                    ClaimAssertion::Knowledge {
                        operation: KnowledgeOperation::Forget,
                        proposition: resolved_entity(&prop_id),
                        audience: Vec::new(),
                        source: Some(source_entity),
                    },
                    TemporalScope::Future,
                    Some(exp_pos),
                    Some(2),
                    None,
                    None,
                    None,
                    "future_knowledge",
                    &natural_key,
                ));
            }

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "future_knowledge",
                    &fk.character_id,
                    &natural_key,
                    false,
                    "child branch future_knowledge natural-key override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &fk.id,
                migration_version,
                raw_snap,
                format!("backfill:future_knowledge:{}", fk.id),
                None,
                format!("backfill future_knowledge {}", fk.id),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 8. quantity_state (absolute observations, never invented deltas)
    // -------------------------------------------------------------------------
    let mut quantity_state_by_subject_pos: BTreeMap<
        (String, String, Option<CanonPositionV1>),
        Vec<String>,
    > = BTreeMap::new();
    {
        let store = CanonBackfillStoreKind::QuantityState;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for qs in &legacy.quantity_states {
            let eff_pos = coordinates_to_canon_position(
                Some(qs.book_number),
                Some(qs.chapter_number),
                Some(qs.scene_order),
            );
            let raw_snap = serde_json::to_string(qs)?;
            if let Some(existing) = check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &qs.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )? {
                quantity_state_by_subject_pos.insert(
                    (qs.subject_id.clone(), qs.measure.clone(), eff_pos),
                    existing.event_ids,
                );
                continue;
            }

            if qs.state.amount.is_none()
                && qs
                    .state
                    .band
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(str::is_empty)
            {
                let entry = CanonSourceMapEntryV1 {
                    project_id: project_id.to_string(),
                    branch_id: branch_id.to_string(),
                    source_table: store.as_source_table().to_string(),
                    legacy_primary_key: qs.id.clone(),
                    migration_version,
                    status: CanonSourceMapStatus::Quarantined,
                    provenance: CanonProvenanceKind::LegacyUnverified,
                    claim_ids: Vec::new(),
                    event_ids: Vec::new(),
                    decision_id: None,
                    quarantine_reason: Some(
                        "quantity_state has neither numeric amount nor band".to_string(),
                    ),
                    raw_snapshot_json: raw_snap,
                    created_at_micros: now_micros,
                    updated_at_micros: now_micros,
                };
                upsert_source_map_in_tx(tx, &entry, now_micros)?;
                counts.quarantined_count += 1;
                report.quarantined_entries.push(entry);
                continue;
            }

            let natural_key = format!(
                "{}|{}|{}|{}|{}|{}",
                qs.subject_table,
                qs.subject_id,
                qs.measure,
                qs.book_number,
                qs.chapter_number,
                qs.scene_order
            );
            let mut drafts = Vec::new();
            let mut step = 0u32;

            if let Some(amount) = qs.state.amount {
                let amount_str = if amount.fract() == 0.0 {
                    format!("{}", amount as i64)
                } else {
                    format!("{amount}")
                };
                drafts.push(assertion_draft(
                    &qs.subject_id,
                    ClaimAssertion::Quantity {
                        operation: QuantityOperation::Set,
                        measure: qs.measure.clone(),
                        amount: amount_str,
                        unit: qs.state.unit.clone().unwrap_or_else(|| "unit".to_string()),
                        transaction_id: None,
                    },
                    TemporalScope::Current,
                    eff_pos,
                    Some(step),
                    None,
                    qs.scene_id.clone(),
                    None,
                    "quantity_state",
                    &natural_key,
                ));
                step += 1;
            }

            drafts.push(assertion_draft(
                &qs.subject_id,
                ClaimAssertion::Attribute {
                    operation: AttributeOperation::Set,
                    key: format!("legacy.quantity_state.{}", qs.id),
                    value: ClaimScalar::Text {
                        value: raw_snap.clone(),
                    },
                    nature: AttributeNature::Evolving,
                },
                TemporalScope::Current,
                None,
                Some(step),
                None,
                qs.scene_id.clone(),
                None,
                "quantity_state",
                &natural_key,
            ));

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "quantity_state",
                    &qs.subject_id,
                    &natural_key,
                    false,
                    "child branch quantity_state natural-key override",
                ));
            }

            let ev_ids = emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &qs.id,
                migration_version,
                raw_snap,
                format!("backfill:quantity_state:{}", qs.id),
                qs.scene_id.clone(),
                format!("backfill quantity_state {}", qs.id),
                drafts,
                &mut counts,
                now_micros,
            )?;
            quantity_state_by_subject_pos
                .insert((qs.subject_id.clone(), qs.measure.clone(), eff_pos), ev_ids);
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 9. progression_event (attributable vs duplicate_skipped vs quarantined)
    // -------------------------------------------------------------------------
    {
        let store = CanonBackfillStoreKind::ProgressionEvent;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for pe in &legacy.progression_events {
            let raw_snap = serde_json::to_string(pe)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &pe.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let eff_pos = pe
                .placement
                .as_ref()
                .map(|p| p.clone().into_core())
                .and_then(|p| story_placement_to_canon_position(&p));
            let delta_str = serde_json::to_string(&pe.delta_json)?;

            match classify_progression_delta_json(&pe.subject_id, &pe.kind, &delta_str) {
                ProgressionDeltaClassification::Quarantined { reason } => {
                    let entry = CanonSourceMapEntryV1 {
                        project_id: project_id.to_string(),
                        branch_id: branch_id.to_string(),
                        source_table: store.as_source_table().to_string(),
                        legacy_primary_key: pe.id.clone(),
                        migration_version,
                        status: CanonSourceMapStatus::Quarantined,
                        provenance: CanonProvenanceKind::LegacyUnverified,
                        claim_ids: Vec::new(),
                        event_ids: Vec::new(),
                        decision_id: None,
                        quarantine_reason: Some(reason),
                        raw_snapshot_json: raw_snap,
                        created_at_micros: now_micros,
                        updated_at_micros: now_micros,
                    };
                    upsert_source_map_in_tx(tx, &entry, now_micros)?;
                    counts.quarantined_count += 1;
                    report.quarantined_entries.push(entry);
                }
                ProgressionDeltaClassification::Attributable {
                    assertions,
                    quantity_measures,
                    touches_character_status,
                } => {
                    // Check if this progression_event duplicates a `quantity_state` or
                    // `character_state` row at the same subject and position on this branch.
                    let mut duplicate_event_ids = Vec::new();
                    for measure in &quantity_measures {
                        if let Some(ev_ids) = quantity_state_by_subject_pos.get(&(
                            pe.subject_id.clone(),
                            measure.clone(),
                            eff_pos,
                        )) {
                            duplicate_event_ids.extend(ev_ids.iter().cloned());
                        }
                    }
                    let duplicates_char_state = touches_character_status
                        && state_positions_with_status.contains(&(pe.subject_id.clone(), eff_pos));

                    if !duplicate_event_ids.is_empty() || duplicates_char_state {
                        let entry = CanonSourceMapEntryV1 {
                            project_id: project_id.to_string(),
                            branch_id: branch_id.to_string(),
                            source_table: store.as_source_table().to_string(),
                            legacy_primary_key: pe.id.clone(),
                            migration_version,
                            status: CanonSourceMapStatus::DuplicateSkipped,
                            provenance: CanonProvenanceKind::LegacyUnverified,
                            claim_ids: Vec::new(),
                            event_ids: duplicate_event_ids,
                            decision_id: None,
                            quarantine_reason: None,
                            raw_snapshot_json: raw_snap,
                            created_at_micros: now_micros,
                            updated_at_micros: now_micros,
                        };
                        upsert_source_map_in_tx(tx, &entry, now_micros)?;
                        counts.duplicate_skipped_count += 1;
                    } else {
                        let natural_key = format!("{}|{}", pe.subject_id, pe.id);
                        let drafts: Vec<CanonEventDraftV1> = assertions
                            .into_iter()
                            .enumerate()
                            .map(|(idx, assertion)| {
                                assertion_draft(
                                    &pe.subject_id,
                                    assertion,
                                    TemporalScope::Current,
                                    eff_pos,
                                    Some(idx as u32),
                                    None,
                                    None,
                                    None,
                                    "progression_event",
                                    &natural_key,
                                )
                            })
                            .collect();

                        emit_backfill_events_and_source_map_in_tx(
                            tx,
                            project_id,
                            branch_id,
                            store,
                            &pe.id,
                            migration_version,
                            raw_snap,
                            format!("backfill:progression_event:{}", pe.id),
                            None,
                            format!("backfill progression_event {}", pe.id),
                            drafts,
                            &mut counts,
                            now_micros,
                        )?;
                    }
                }
            }
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 10. system_overlay
    // -------------------------------------------------------------------------
    {
        let store = CanonBackfillStoreKind::SystemOverlay;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for so in &legacy.system_overlays {
            let raw_snap = serde_json::to_string(so)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &so.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = so.normalized_name.clone();
            let subject_id = so.id.clone();

            let mut drafts = vec![
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: "system_overlay.visibility".to_string(),
                        value: ClaimScalar::Text {
                            value: so.visibility.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    TemporalScope::Current,
                    None,
                    Some(0),
                    None,
                    None,
                    None,
                    "system_overlay",
                    &natural_key,
                ),
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: "legacy.system_overlay".to_string(),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    TemporalScope::Current,
                    None,
                    Some(1),
                    None,
                    None,
                    None,
                    "system_overlay",
                    &natural_key,
                ),
            ];

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "system_overlay",
                    &subject_id,
                    &natural_key,
                    false,
                    "child branch system_overlay natural-key override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &so.id,
                migration_version,
                raw_snap,
                format!("backfill:system_overlay:{}", so.id),
                None,
                format!("backfill system_overlay {}", so.id),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 11. timeline_event + 12. temporal_intervention
    // -------------------------------------------------------------------------
    let event_clocks_by_id: BTreeMap<String, StoryClock> = legacy
        .timeline_event_clocks
        .iter()
        .map(|c| (c.timeline_event_id.clone(), c.clock.clone()))
        .collect();

    {
        let store = CanonBackfillStoreKind::TimelineEvent;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for te in &legacy.timeline_events {
            let raw_snap = serde_json::to_string(te)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &te.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = te.title.clone();
            let subject_id = te.id.clone();

            let eff_pos = story_placement_to_canon_position(&te.placement.clone().into_core());
            let clock = event_clocks_by_id.get(&te.id).cloned();

            let mut drafts = vec![
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Rule {
                        operation: RuleOperation::Anchor,
                        rule: resolved_entity(&te.id),
                        anchor: None,
                        preconditions: te.related_entity_ids.clone(),
                        effects: vec![te.summary.clone()],
                    },
                    TemporalScope::Current,
                    eff_pos,
                    Some(0),
                    clock.clone(),
                    None,
                    None,
                    "timeline_event",
                    &natural_key,
                ),
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: "legacy.timeline_event".to_string(),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature: AttributeNature::Invariant,
                    },
                    TemporalScope::Current,
                    None,
                    Some(1),
                    clock,
                    None,
                    None,
                    "timeline_event",
                    &natural_key,
                ),
            ];

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "timeline_event",
                    &subject_id,
                    &natural_key,
                    false,
                    "child branch timeline_event natural-key override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &te.id,
                migration_version,
                raw_snap,
                format!("backfill:timeline_event:{}", te.id),
                None,
                format!("backfill timeline_event {}", te.id),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    {
        let store = CanonBackfillStoreKind::TemporalIntervention;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for ti in &legacy.temporal_interventions {
            let raw_snap = serde_json::to_string(ti)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &ti.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = ti.title.clone();
            let subject_id = ti.id.clone();

            let eff_pos: Option<CanonPositionV1> = None;
            let anchor = ti
                .target_event_id
                .as_ref()
                .or(ti.source_event_id.as_ref())
                .map(|eid| resolved_entity(eid.clone()));

            let mut drafts = vec![
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Rule {
                        operation: RuleOperation::Intervene,
                        rule: resolved_entity(&ti.id),
                        anchor,
                        preconditions: ti.source_event_id.iter().cloned().collect(),
                        effects: ti.consequences.clone(),
                    },
                    TemporalScope::Current,
                    eff_pos,
                    Some(0),
                    None,
                    None,
                    None,
                    "temporal_intervention",
                    &natural_key,
                ),
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: "legacy.temporal_intervention".to_string(),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    TemporalScope::Current,
                    None,
                    Some(1),
                    None,
                    None,
                    None,
                    "temporal_intervention",
                    &natural_key,
                ),
            ];

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "temporal_intervention",
                    &subject_id,
                    &natural_key,
                    false,
                    "child branch temporal_intervention natural-key override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &ti.id,
                migration_version,
                raw_snap,
                format!("backfill:temporal_intervention:{}", ti.id),
                None,
                format!("backfill temporal_intervention {}", ti.id),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 13. scene_clock + 14. timeline_event_clock
    // -------------------------------------------------------------------------
    {
        let store = CanonBackfillStoreKind::SceneClock;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for sc in &legacy.scene_clocks {
            let raw_snap = serde_json::to_string(sc)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &sc.scene_id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = sc.scene_id.clone();
            let subject_id = sc.scene_id.clone();
            let eff_pos = scene_positions.get(&sc.scene_id).copied();

            let mut drafts = vec![
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::SceneClock {
                        clock: sc.clock.clone(),
                    },
                    TemporalScope::Current,
                    eff_pos,
                    Some(0),
                    Some(sc.clock.clone()),
                    Some(sc.scene_id.clone()),
                    None,
                    "scene_clock",
                    &natural_key,
                ),
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: "legacy.scene_clock".to_string(),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    TemporalScope::Current,
                    None,
                    Some(1),
                    Some(sc.clock.clone()),
                    Some(sc.scene_id.clone()),
                    None,
                    "scene_clock",
                    &natural_key,
                ),
            ];

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "scene_clock",
                    &subject_id,
                    &natural_key,
                    false,
                    "child branch scene_clock override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &sc.scene_id,
                migration_version,
                raw_snap,
                format!("backfill:scene_clock:{}", sc.scene_id),
                Some(sc.scene_id.clone()),
                format!("backfill scene_clock {}", sc.scene_id),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    {
        let store = CanonBackfillStoreKind::TimelineEventClock;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for tec in &legacy.timeline_event_clocks {
            let raw_snap = serde_json::to_string(tec)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &tec.timeline_event_id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = tec.timeline_event_id.clone();
            let subject_id = tec.timeline_event_id.clone();

            let mut drafts = vec![
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::SceneClock {
                        clock: tec.clock.clone(),
                    },
                    TemporalScope::Current,
                    None,
                    Some(0),
                    Some(tec.clock.clone()),
                    None,
                    None,
                    "timeline_event_clock",
                    &natural_key,
                ),
                assertion_draft(
                    &subject_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: "legacy.timeline_event_clock".to_string(),
                        value: ClaimScalar::Text {
                            value: raw_snap.clone(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    TemporalScope::Current,
                    None,
                    Some(1),
                    Some(tec.clock.clone()),
                    None,
                    None,
                    "timeline_event_clock",
                    &natural_key,
                ),
            ];

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "timeline_event_clock",
                    &subject_id,
                    &natural_key,
                    false,
                    "child branch timeline_event_clock override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &tec.timeline_event_id,
                migration_version,
                raw_snap,
                format!("backfill:timeline_event_clock:{}", tec.timeline_event_id),
                None,
                format!("backfill timeline_event_clock {}", tec.timeline_event_id),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 15. narrative_promise
    // -------------------------------------------------------------------------
    {
        let store = CanonBackfillStoreKind::NarrativePromise;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for np in &legacy.narrative_promises {
            let raw_snap = serde_json::to_string(np)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &np.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let natural_key = format!("{}|{}", np.promise_type, np.description);
            let subject_id = np.id.clone();

            if np.archived_at.is_some() {
                let mut drafts = vec![
                    assertion_draft(
                        &subject_id,
                        ClaimAssertion::Attribute {
                            operation: AttributeOperation::Set,
                            key: "legacy.narrative_promise".to_string(),
                            value: ClaimScalar::Text {
                                value: raw_snap.clone(),
                            },
                            nature: AttributeNature::Evolving,
                        },
                        TemporalScope::Current,
                        None,
                        Some(0),
                        None,
                        None,
                        None,
                        "narrative_promise",
                        &natural_key,
                    ),
                    CanonEventDraftV1 {
                        subject: resolved_entity(&subject_id),
                        payload: CanonEventPayloadV1::CoverageMask {
                            mask: StreamCoverageMaskV1 {
                                target: CoverageMaskTargetV1::Stream {
                                    stream: SemanticStreamKeyV1::PromiseThread {
                                        thread_id: np.id.clone(),
                                    },
                                },
                                start_position: None,
                                end_position: None,
                                tombstone: true,
                            },
                        },
                        effective_position: None,
                        within_scene_order: None,
                        in_world_time: None,
                        scene_id: None,
                        source_claim_ids: Vec::new(),
                        evidence_sha256: evidence_sha256(&[]),
                        provenance: CanonProvenanceKind::LegacyUnverified,
                        supersedes_event_id: None,
                        withdraws_event_id: None,
                        legacy_record_scope: None,
                        merge_provenance: None,
                    },
                ];
                if is_non_root {
                    drafts.push(legacy_whole_record_mask_draft(
                        "narrative_promise",
                        &subject_id,
                        &natural_key,
                        false,
                        "child branch archived narrative_promise override",
                    ));
                }
                emit_backfill_events_and_source_map_in_tx(
                    tx,
                    project_id,
                    branch_id,
                    store,
                    &np.id,
                    migration_version,
                    raw_snap,
                    format!("backfill:narrative_promise:tombstone:{}", np.id),
                    None,
                    format!("backfill archived narrative_promise {}", np.id),
                    drafts,
                    &mut counts,
                    now_micros,
                )?;
                continue;
            }

            let planted_pos = story_placement_to_canon_position(&np.planted_at.clone().into_core());
            let mut drafts = vec![assertion_draft(
                &subject_id,
                ClaimAssertion::Promise {
                    operation: PromiseOperation::Plant,
                    thread: resolved_entity(&np.id),
                },
                TemporalScope::Current,
                planted_pos,
                Some(0),
                None,
                None,
                None,
                "narrative_promise",
                &natural_key,
            )];

            let replaced_ids: BTreeSet<&str> = np
                .status_history
                .iter()
                .filter_map(|e| e.replaces_event_id.as_deref())
                .collect();
            let mut step = 1u32;
            let mut emitted_status_transition = false;
            for entry in &np.status_history {
                if replaced_ids.contains(entry.id.as_str()) {
                    continue;
                }
                let op = match entry.status.trim().to_ascii_lowercase().as_str() {
                    "planted" => PromiseOperation::Plant,
                    "reinforced" | "advanced" | "in_progress" => PromiseOperation::Reinforce,
                    "paid_off" | "resolved" | "fulfilled" => PromiseOperation::Payoff,
                    "abandoned" | "dropped" => PromiseOperation::Abandon,
                    _ => PromiseOperation::Reinforce,
                };
                if op == PromiseOperation::Plant && step == 1 {
                    continue;
                }
                let transition_pos = entry
                    .at
                    .as_ref()
                    .and_then(story_placement_to_canon_position)
                    .or(planted_pos);
                drafts.push(assertion_draft(
                    &subject_id,
                    ClaimAssertion::Promise {
                        operation: op,
                        thread: resolved_entity(&np.id),
                    },
                    TemporalScope::Current,
                    transition_pos,
                    Some(step),
                    None,
                    None,
                    None,
                    "narrative_promise",
                    &natural_key,
                ));
                emitted_status_transition = true;
                step += 1;
            }

            if !emitted_status_transition {
                let terminal_op = match np.status.trim().to_ascii_lowercase().as_str() {
                    "reinforced" | "advanced" | "in_progress" => Some(PromiseOperation::Reinforce),
                    "paid_off" | "resolved" | "fulfilled" => Some(PromiseOperation::Payoff),
                    "abandoned" | "dropped" => Some(PromiseOperation::Abandon),
                    _ => None,
                };
                if let Some(op) = terminal_op {
                    let payoff_pos = np
                        .planned_payoff
                        .as_ref()
                        .map(|p| p.clone().into_core())
                        .and_then(|p| story_placement_to_canon_position(&p))
                        .or(planted_pos);
                    drafts.push(assertion_draft(
                        &subject_id,
                        ClaimAssertion::Promise {
                            operation: op,
                            thread: resolved_entity(&np.id),
                        },
                        TemporalScope::Current,
                        payoff_pos,
                        Some(step),
                        None,
                        None,
                        None,
                        "narrative_promise",
                        &natural_key,
                    ));
                    step += 1;
                }
            }

            drafts.push(assertion_draft(
                &subject_id,
                ClaimAssertion::Attribute {
                    operation: AttributeOperation::Set,
                    key: "legacy.narrative_promise".to_string(),
                    value: ClaimScalar::Text {
                        value: raw_snap.clone(),
                    },
                    nature: AttributeNature::Evolving,
                },
                TemporalScope::Current,
                None,
                Some(step),
                None,
                None,
                None,
                "narrative_promise",
                &natural_key,
            ));

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "narrative_promise",
                    &subject_id,
                    &natural_key,
                    false,
                    "child branch narrative_promise natural-key override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &np.id,
                migration_version,
                raw_snap,
                format!("backfill:narrative_promise:{}", np.id),
                None,
                format!("backfill narrative_promise {}", np.id),
                drafts,
                &mut counts,
                now_micros,
            )?;
        }

        report.record_store_counts(store, &counts);
    }

    // -------------------------------------------------------------------------
    // 16. relates_to + 17. relationship_scene_delta
    // -------------------------------------------------------------------------
    {
        let rel_store = CanonBackfillStoreKind::Relationship;
        let delta_store = CanonBackfillStoreKind::RelationshipSceneDelta;
        let mut rel_counts = CanonStoreBackfillCountsV1::default();
        let mut delta_counts = CanonStoreBackfillCountsV1::default();

        // Group `relationship_scene_delta` rows by canonical pair and scene key
        // (matching `tx.update_relationship`: per pair + scene position or scene_id,
        // the row with the greatest `updated_at` is the active delta; older rows for
        // that same pair+scene were replaced and must be marked `DuplicateSkipped`).
        let mut deltas_by_pair: BTreeMap<(String, String), Vec<&StoredRelationshipSceneDeltaRow>> =
            BTreeMap::new();
        for d in &legacy.relationship_deltas {
            let pair = relationship_pair_key(&d.in_id, &d.out_id);
            deltas_by_pair.entry(pair).or_default().push(d);
        }

        let mut active_deltas_by_pair: BTreeMap<
            (String, String),
            Vec<&StoredRelationshipSceneDeltaRow>,
        > = BTreeMap::new();
        let mut superseded_deltas_by_pair: BTreeMap<
            (String, String),
            Vec<(&StoredRelationshipSceneDeltaRow, String)>,
        > = BTreeMap::new();

        for (pair, rows) in &deltas_by_pair {
            let mut by_scene_slot: BTreeMap<String, Vec<&StoredRelationshipSceneDeltaRow>> =
                BTreeMap::new();
            for row in rows {
                let slot_key = match (row.book_number, row.chapter_number, row.scene_order) {
                    (Some(b), Some(c), Some(s)) if b >= 1 && c >= 1 && s >= 1 => {
                        format!("pos:{b}:{c}:{s}")
                    }
                    _ => format!("scene:{}", row.scene_id),
                };
                by_scene_slot.entry(slot_key).or_default().push(*row);
            }
            for (_slot, mut slot_rows) in by_scene_slot {
                slot_rows.sort_by(|a, b| {
                    b.updated_at
                        .cmp(&a.updated_at)
                        .then_with(|| a.source.cmp(&b.source))
                });
                let winner = slot_rows[0];
                let winner_pk = winner.primary_key();
                active_deltas_by_pair
                    .entry(pair.clone())
                    .or_default()
                    .push(winner);
                for loser in slot_rows.into_iter().skip(1) {
                    superseded_deltas_by_pair
                        .entry(pair.clone())
                        .or_default()
                        .push((loser, winner_pk.clone()));
                }
            }
        }

        let mut rel_orientation_by_pair: BTreeMap<(String, String), (String, String)> =
            BTreeMap::new();

        for rel in &legacy.relationships {
            let pair = relationship_pair_key(&rel.in_id, &rel.out_id);
            rel_orientation_by_pair.insert(pair.clone(), (rel.in_id.clone(), rel.out_id.clone()));
            let pk = format!("{}|{}|{}", rel.branch_id, rel.in_id, rel.out_id);
            let matching_deltas: Vec<&StoredRelationshipSceneDeltaRow> = legacy
                .relationship_deltas
                .iter()
                .filter(|d| relationship_pair_key(&d.in_id, &d.out_id) == pair)
                .collect();
            let raw_snap = serde_json::to_string(&(rel, matching_deltas))?;

            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                rel_store,
                &pk,
                migration_version,
                &raw_snap,
                &mut rel_counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            let active_deltas = active_deltas_by_pair
                .get(&pair)
                .cloned()
                .unwrap_or_default();
            let sum_trust_delta: i32 = active_deltas.iter().map(|d| d.trust_delta).sum();
            let sum_tension_delta: i32 = active_deltas.iter().map(|d| d.tension_delta).sum();
            let opening_trust = rel.trust - sum_trust_delta;
            let opening_tension = rel.tension - sum_tension_delta;

            let natural_key = format!("{}|{}", pair.0, pair.1);
            let raw_rel_only = serde_json::to_string(rel)?;

            let mut drafts = vec![
                assertion_draft(
                    &rel.in_id,
                    ClaimAssertion::Relationship {
                        operation: RelationshipOperation::Observation,
                        other: resolved_entity(&rel.out_id),
                        trust: opening_trust,
                        tension: opening_tension,
                    },
                    TemporalScope::Current,
                    None,
                    Some(0),
                    None,
                    rel.last_scene_id.clone(),
                    None,
                    "relates_to",
                    &natural_key,
                ),
                assertion_draft(
                    &rel.in_id,
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: format!("legacy.relates_to.{natural_key}"),
                        value: ClaimScalar::Text {
                            value: raw_rel_only,
                        },
                        nature: AttributeNature::Evolving,
                    },
                    TemporalScope::Current,
                    None,
                    Some(1),
                    None,
                    rel.last_scene_id.clone(),
                    None,
                    "relates_to",
                    &natural_key,
                ),
            ];

            if is_non_root {
                drafts.push(legacy_whole_record_mask_draft(
                    "relates_to",
                    &rel.in_id,
                    &natural_key,
                    false,
                    "child branch relates_to natural-key override",
                ));
            }

            emit_backfill_events_and_source_map_in_tx(
                tx,
                project_id,
                branch_id,
                rel_store,
                &pk,
                migration_version,
                raw_snap,
                format!("backfill:relates_to:{pk}"),
                rel.last_scene_id.clone(),
                format!("backfill relates_to {pk}"),
                drafts,
                &mut rel_counts,
                now_micros,
            )?;
        }

        // Now backfill active `relationship_scene_delta` rows and mark superseded ones
        // `DuplicateSkipped`.
        let mut active_delta_event_ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (pair, mut active_rows) in active_deltas_by_pair {
            active_rows.sort_by(|a, b| {
                (a.book_number, a.chapter_number, a.scene_order, a.updated_at).cmp(&(
                    b.book_number,
                    b.chapter_number,
                    b.scene_order,
                    b.updated_at,
                ))
            });
            let (subj_id, other_id) = rel_orientation_by_pair
                .get(&pair)
                .cloned()
                .unwrap_or_else(|| (pair.0.clone(), pair.1.clone()));
            let natural_key = format!("{}|{}", pair.0, pair.1);

            for (step_idx, d) in active_rows.into_iter().enumerate() {
                let pk = d.primary_key();
                let raw_snap = serde_json::to_string(d)?;
                if let Some(existing) = check_already_mapped_in_tx(
                    tx,
                    project_id,
                    branch_id,
                    delta_store,
                    &pk,
                    migration_version,
                    &raw_snap,
                    &mut delta_counts,
                    &mut report.quarantined_entries,
                )? {
                    active_delta_event_ids.insert(pk, existing.event_ids);
                    continue;
                }

                let eff_pos =
                    coordinates_to_canon_position(d.book_number, d.chapter_number, d.scene_order)
                        .or_else(|| scene_positions.get(&d.scene_id).copied());

                let drafts = vec![
                    assertion_draft(
                        &subj_id,
                        ClaimAssertion::Relationship {
                            operation: RelationshipOperation::Delta,
                            other: resolved_entity(&other_id),
                            trust: d.trust_delta,
                            tension: d.tension_delta,
                        },
                        TemporalScope::Current,
                        eff_pos,
                        Some((step_idx as u32) + 10),
                        None,
                        Some(d.scene_id.clone()),
                        None,
                        "relates_to",
                        &natural_key,
                    ),
                    assertion_draft(
                        &subj_id,
                        ClaimAssertion::Attribute {
                            operation: AttributeOperation::Set,
                            key: format!("legacy.relationship_scene_delta.{pk}"),
                            value: ClaimScalar::Text {
                                value: raw_snap.clone(),
                            },
                            nature: AttributeNature::Evolving,
                        },
                        TemporalScope::Current,
                        eff_pos,
                        Some((step_idx as u32) + 1000),
                        None,
                        Some(d.scene_id.clone()),
                        None,
                        "relates_to",
                        &natural_key,
                    ),
                ];

                let ev_ids = emit_backfill_events_and_source_map_in_tx(
                    tx,
                    project_id,
                    branch_id,
                    delta_store,
                    &pk,
                    migration_version,
                    raw_snap,
                    format!("backfill:relationship_scene_delta:{pk}"),
                    Some(d.scene_id.clone()),
                    format!("backfill relationship_scene_delta {pk}"),
                    drafts,
                    &mut delta_counts,
                    now_micros,
                )?;
                active_delta_event_ids.insert(pk, ev_ids);
            }
        }

        for (_pair, losers) in superseded_deltas_by_pair {
            for (loser, winner_pk) in losers {
                let pk = loser.primary_key();
                let raw_snap = serde_json::to_string(loser)?;
                if check_already_mapped_in_tx(
                    tx,
                    project_id,
                    branch_id,
                    delta_store,
                    &pk,
                    migration_version,
                    &raw_snap,
                    &mut delta_counts,
                    &mut report.quarantined_entries,
                )?
                .is_some()
                {
                    continue;
                }
                let linked_ev_ids = active_delta_event_ids
                    .get(&winner_pk)
                    .cloned()
                    .unwrap_or_default();
                let entry = CanonSourceMapEntryV1 {
                    project_id: project_id.to_string(),
                    branch_id: branch_id.to_string(),
                    source_table: delta_store.as_source_table().to_string(),
                    legacy_primary_key: pk,
                    migration_version,
                    status: CanonSourceMapStatus::DuplicateSkipped,
                    provenance: CanonProvenanceKind::LegacyUnverified,
                    claim_ids: Vec::new(),
                    event_ids: linked_ev_ids,
                    decision_id: None,
                    quarantine_reason: None,
                    raw_snapshot_json: raw_snap,
                    created_at_micros: now_micros,
                    updated_at_micros: now_micros,
                };
                upsert_source_map_in_tx(tx, &entry, now_micros)?;
                delta_counts.duplicate_skipped_count += 1;
            }
        }

        report.record_store_counts(rel_store, &rel_counts);
        report.record_store_counts(delta_store, &delta_counts);
    }

    // -------------------------------------------------------------------------
    // 18. canon_delta (claims + decisions; applied rows link without reapplying)
    // -------------------------------------------------------------------------
    {
        let store = CanonBackfillStoreKind::CanonDelta;
        let mut counts = CanonStoreBackfillCountsV1::default();

        for cd in &legacy.canon_deltas {
            let raw_snap = serde_json::to_string(cd)?;
            if check_already_mapped_in_tx(
                tx,
                project_id,
                branch_id,
                store,
                &cd.id,
                migration_version,
                &raw_snap,
                &mut counts,
                &mut report.quarantined_entries,
            )?
            .is_some()
            {
                continue;
            }

            if !cd.payload.is_object() {
                let entry = CanonSourceMapEntryV1 {
                    project_id: project_id.to_string(),
                    branch_id: branch_id.to_string(),
                    source_table: store.as_source_table().to_string(),
                    legacy_primary_key: cd.id.clone(),
                    migration_version,
                    status: CanonSourceMapStatus::Quarantined,
                    provenance: CanonProvenanceKind::LegacyUnverified,
                    claim_ids: Vec::new(),
                    event_ids: Vec::new(),
                    decision_id: None,
                    quarantine_reason: Some("canon_delta payload is not a JSON object".to_string()),
                    raw_snapshot_json: raw_snap,
                    created_at_micros: now_micros,
                    updated_at_micros: now_micros,
                };
                upsert_source_map_in_tx(tx, &entry, now_micros)?;
                counts.quarantined_count += 1;
                report.quarantined_entries.push(entry);
                continue;
            }

            let (claim_status, claim_tier, decision_action, source_map_status) =
                match cd.status.as_str() {
                    "staged" => (
                        CanonClaimStatus::Provisional,
                        CanonTier::Provisional,
                        CanonPolicyAction::StageProvisional,
                        CanonSourceMapStatus::Mapped,
                    ),
                    "rejected" => (
                        CanonClaimStatus::Rejected,
                        CanonTier::Provisional,
                        CanonPolicyAction::Reject,
                        CanonSourceMapStatus::Mapped,
                    ),
                    "superseded" => (
                        CanonClaimStatus::SourceInvalidated,
                        CanonTier::Provisional,
                        CanonPolicyAction::InvalidateSource,
                        CanonSourceMapStatus::Mapped,
                    ),
                    "applied" => (
                        CanonClaimStatus::Ratified,
                        CanonTier::Ratified,
                        CanonPolicyAction::Ratify,
                        CanonSourceMapStatus::DuplicateSkipped,
                    ),
                    other => {
                        let entry = CanonSourceMapEntryV1 {
                            project_id: project_id.to_string(),
                            branch_id: branch_id.to_string(),
                            source_table: store.as_source_table().to_string(),
                            legacy_primary_key: cd.id.clone(),
                            migration_version,
                            status: CanonSourceMapStatus::Quarantined,
                            provenance: CanonProvenanceKind::LegacyUnverified,
                            claim_ids: Vec::new(),
                            event_ids: Vec::new(),
                            decision_id: None,
                            quarantine_reason: Some(format!(
                                "unrecognized canon_delta status '{other}'"
                            )),
                            raw_snapshot_json: raw_snap,
                            created_at_micros: now_micros,
                            updated_at_micros: now_micros,
                        };
                        upsert_source_map_in_tx(tx, &entry, now_micros)?;
                        counts.quarantined_count += 1;
                        report.quarantined_entries.push(entry);
                        continue;
                    }
                };

            let scene_pos = scene_positions.get(&cd.scene_id).copied();
            let placement = match scene_pos {
                Some(pos) => StoryPlacement {
                    book_number: pos.book_number as i32,
                    chapter_number: pos.chapter_number as i32,
                    scene_order: Some(pos.scene_order as i32),
                    note: None,
                },
                None => StoryPlacement {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: None,
                    note: None,
                },
            };
            let subject_id = cd
                .target_id
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| format!("scene:{}", cd.scene_id));
            let confidence = match cd.confidence.as_str() {
                "high" => Some(0.9),
                "medium" => Some(0.65),
                "low" => Some(0.35),
                _ => None,
            };

            let claim_v1 = ClaimV1 {
                binding: ClaimBindingV1 {
                    schema_version: CLAIM_SCHEMA_VERSION,
                    project_id: project_id.to_string(),
                    branch_id: branch_id.to_string(),
                    scene_id: Some(cd.scene_id.clone()),
                    revision_id: format!("backfill:canon_delta:{}", cd.id),
                    accepted_text_sha256: "0".repeat(64),
                    position: placement,
                    extraction_run_id: cd
                        .authoring_run_id
                        .clone()
                        .unwrap_or_else(|| format!("backfill:{}", cd.id)),
                    model: "legacy_canon_delta".to_string(),
                    agent_id: None,
                    model_call_id: None,
                    prompt_version: EXTRACT_PROMPT_VERSION.to_string(),
                },
                source: ClaimSource::Independent,
                proposed: ProposedClaimV1 {
                    claim_id: cd.id.clone(),
                    subject: resolved_entity(&subject_id),
                    assertion: ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: format!("canon_delta.{}", cd.delta_class),
                        value: ClaimScalar::Text {
                            value: cd.payload.to_string(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    within_scene_order: None,
                    in_world_time: None,
                    modality: ClaimModality::Narration,
                    temporal_scope: TemporalScope::Current,
                    speaker: None,
                    experiencer: None,
                    confidence,
                    evidence: Vec::new(),
                },
            };

            let sem_fp = semantic_claim_fingerprint(&claim_v1);
            let claim_record_id = mint_id_local("canon_claim");
            let claim_rec = CanonClaimRecordV1 {
                claim_record_id: claim_record_id.clone(),
                project_id: project_id.to_string(),
                branch_id: branch_id.to_string(),
                scene_id: Some(cd.scene_id.clone()),
                revision_id: claim_v1.binding.revision_id.clone(),
                source: ClaimSource::Independent,
                semantic_fingerprint: sem_fp.clone(),
                status: claim_status,
                tier: claim_tier,
                provenance: CanonProvenanceKind::LegacyUnverified,
                claim: claim_v1,
                created_at_micros: now_micros,
            };
            let claim_status_str = enum_to_sql_str(claim_status)?;
            let claim_tier_str = enum_to_sql_str(claim_tier)?;
            let prov_str = enum_to_sql_str(CanonProvenanceKind::LegacyUnverified)?;
            let claim_json = serde_json::to_string(&claim_rec)?;
            tx.execute(
                "INSERT INTO canon_claim (
                    id, project_id, branch_id, scene_id, revision_id,
                    source, semantic_fingerprint, status, tier,
                    provenance, data, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, 'independent', ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    &claim_record_id,
                    project_id,
                    branch_id,
                    &cd.scene_id,
                    &claim_rec.revision_id,
                    &sem_fp,
                    &claim_status_str,
                    &claim_tier_str,
                    &prov_str,
                    &claim_json,
                    now_micros,
                ],
            )?;

            // If `cd.status == "applied"`, find any already-mapped target events on
            // this branch for the same scene or target entity so we link them without
            // reapplying the delta!
            let linked_event_ids: Vec<String> = if cd.status == "applied" {
                let mut stmt = tx.prepare(
                    "SELECT id FROM canon_event \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND (scene_id = ?3 OR (?4 IS NOT NULL AND subject_id = ?4)) \
                     ORDER BY append_seq ASC",
                )?;
                stmt.query_map(
                    rusqlite::params![project_id, branch_id, &cd.scene_id, &cd.target_id],
                    |r| r.get::<_, String>(0),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                Vec::new()
            };

            // Record the canon_decision row (without appending duplicate truth events
            // for already-applied deltas!).
            let decision_id = mint_id_local("canon_decision");
            let action_str = enum_to_sql_str(decision_action)?;
            let idem_key = compute_idempotency_key(
                branch_id,
                &claim_rec.revision_id,
                &sem_fp,
                decision_action,
            );
            let mut dec_rec = CanonDecisionV1 {
                decision_id: decision_id.clone(),
                decision_seq: 0,
                project_id: project_id.to_string(),
                branch_id: branch_id.to_string(),
                action: decision_action,
                actor: cd
                    .decided_by
                    .clone()
                    .unwrap_or_else(|| "system:canon_backfill".to_string()),
                policy_version: CANON_POLICY_VERSION_V1.to_string(),
                source_revision_id: claim_rec.revision_id.clone(),
                semantic_fingerprint: sem_fp.clone(),
                idempotency_key: idem_key.clone(),
                projection_watermark: 0,
                previous_decision_id: None,
                target_scene_id: Some(cd.scene_id.clone()),
                confidence_inputs: confidence.into_iter().collect(),
                precondition_summary: "legacy_backfill".to_string(),
                reason: format!("backfill canon_delta {} ({})", cd.id, cd.status),
                event_ids: linked_event_ids.clone(),
                created_at_micros: now_micros,
            };
            let placeholder_dec = serde_json::to_string(&dec_rec)?;
            tx.execute(
                "INSERT INTO canon_decision (
                    id, project_id, branch_id, action, actor, policy_version,
                    source_revision_id, semantic_fingerprint, idempotency_key,
                    projection_watermark, previous_decision_id, target_scene_id,
                    reason, data, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, NULL, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    &decision_id,
                    project_id,
                    branch_id,
                    &action_str,
                    &dec_rec.actor,
                    &dec_rec.policy_version,
                    &dec_rec.source_revision_id,
                    &sem_fp,
                    &idem_key,
                    &cd.scene_id,
                    &dec_rec.reason,
                    &placeholder_dec,
                    now_micros,
                ],
            )?;
            let dec_seq = tx.last_insert_rowid();
            dec_rec.decision_seq = dec_seq;
            let final_dec_json = serde_json::to_string(&dec_rec)?;
            tx.execute(
                "UPDATE canon_decision SET data = ?1 WHERE decision_seq = ?2",
                rusqlite::params![&final_dec_json, dec_seq],
            )?;

            let entry = CanonSourceMapEntryV1 {
                project_id: project_id.to_string(),
                branch_id: branch_id.to_string(),
                source_table: store.as_source_table().to_string(),
                legacy_primary_key: cd.id.clone(),
                migration_version,
                status: source_map_status,
                provenance: CanonProvenanceKind::LegacyUnverified,
                claim_ids: vec![claim_record_id],
                event_ids: linked_event_ids,
                decision_id: Some(decision_id),
                quarantine_reason: None,
                raw_snapshot_json: raw_snap,
                created_at_micros: now_micros,
                updated_at_micros: now_micros,
            };
            upsert_source_map_in_tx(tx, &entry, now_micros)?;

            counts.emitted_claim_count += 1;
            if source_map_status == CanonSourceMapStatus::DuplicateSkipped {
                counts.duplicate_skipped_count += 1;
            } else {
                counts.mapped_count += 1;
            }
        }

        report.record_store_counts(store, &counts);
    }

    // Materialize the branch's ratified projection state after backfilling all stores.
    let lineage = resolve_branch_lineage_in_conn(tx, Some(project_id), branch_id)?;
    let lineage_events = load_events_for_branches_in_conn(tx, project_id, &lineage)?;
    let view = state_at(
        branch_id,
        &lineage,
        &lineage_events,
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )?;
    materialize_projection_view_in_conn(tx, project_id, branch_id, &view, now_micros)?;

    Ok(())
}

pub(crate) fn backfill_project_canon_in_tx(
    tx: &rusqlite::Connection,
    project_id: &str,
    migration_version: u32,
    now_micros: i64,
) -> Result<CanonBackfillReportV1> {
    let project_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM project WHERE id = ?1)",
        [project_id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(project_exists, "project {project_id} not found");

    let branches = list_branches_topological_in_conn(tx, project_id)?;
    let mut legacy_by_branch = BTreeMap::new();
    for branch in &branches {
        let rows = BranchLegacyRows::load(tx, project_id, &branch.id)?;
        let snapshots = rows.current_snapshots_by_key()?;
        reconcile_branch_source_maps_in_tx(
            tx,
            project_id,
            &branch.id,
            migration_version,
            &snapshots,
            now_micros,
        )?;
        legacy_by_branch.insert(branch.id.clone(), rows);
    }

    let mut report = CanonBackfillReportV1::new(project_id, migration_version);
    for branch in &branches {
        let scene_positions = load_branch_scene_positions_in_conn(tx, project_id, &branch.id)?;
        let legacy = legacy_by_branch
            .get(&branch.id)
            .expect("branch legacy rows loaded");
        backfill_branch_in_tx(
            tx,
            project_id,
            branch,
            migration_version,
            legacy,
            &scene_positions,
            &mut report,
            now_micros,
        )?;
    }

    Ok(report)
}

impl Repository {
    /// Backfill all legacy canon/state stores across all branches of `project_id`
    /// into append-only `canon_claim`, `canon_decision`, `canon_event`, and
    /// `canon_source_map` rows using `CANON_BACKFILL_MIGRATION_VERSION_V1`.
    pub async fn backfill_project_canon(&self, project_id: &str) -> Result<CanonBackfillReportV1> {
        self.backfill_project_canon_with_version(project_id, CANON_BACKFILL_MIGRATION_VERSION_V1)
            .await
    }

    /// Backfill all legacy canon/state stores across all branches of `project_id`
    /// using an explicit `migration_version`. Reruns with the same version are
    /// idempotent.
    pub async fn backfill_project_canon_with_version(
        &self,
        project_id: &str,
        migration_version: u32,
    ) -> Result<CanonBackfillReportV1> {
        let project_id = project_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    let report =
                        backfill_project_canon_in_tx(&tx, &project_id, migration_version, now)?;
                    tx.commit()?;
                    Ok(report)
                })())
            })
            .await?
    }
}
