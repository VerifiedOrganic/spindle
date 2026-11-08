//! SQLite persistence for append-only canon claims, decisions, typed events,
//! source mappings, checkpoints, and rebuildable projections (P2 PR C).

use super::*;
use spindle_core::canon::*;
use std::collections::{BTreeMap, BTreeSet};

fn enum_to_sql_str<T: serde::Serialize>(value: T) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("expected string-serialized enum variant"))
}

fn check_branch_in_project(
    conn: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM bible_branch WHERE id = ?1 AND project_id = ?2)",
        rusqlite::params![branch_id, project_id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(
        exists,
        "branch {branch_id} does not belong to project {project_id}"
    );
    Ok(())
}

pub(crate) fn load_events_for_branches_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
    branches: &[String],
) -> Result<Vec<CanonEventV1>> {
    if branches.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: Vec<String> = (0..branches.len()).map(|i| format!("?{}", i + 2)).collect();
    let sql = format!(
        "SELECT append_seq, decision_seq, data FROM canon_event \
         WHERE project_id = ?1 AND branch_id IN ({}) \
         ORDER BY append_seq ASC",
        placeholders.join(", ")
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(branches.len() + 1);
    params.push(&project_id);
    for b in branches {
        params.push(b);
    }
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for (append_seq, decision_seq, data) in rows {
        let mut ev: CanonEventV1 =
            serde_json::from_str(&data).context("deserializing canon_event.data")?;
        ev.append_seq = append_seq;
        ev.decision_seq = decision_seq;
        out.push(ev);
    }
    Ok(out)
}

fn load_decision_by_id_in_conn(
    conn: &rusqlite::Connection,
    decision_id: &str,
) -> Result<Option<CanonDecisionV1>> {
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT decision_seq, data FROM canon_decision WHERE id = ?1",
            [decision_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional_inner()?;
    match row {
        Some((seq, data)) => {
            let mut dec: CanonDecisionV1 =
                serde_json::from_str(&data).context("deserializing canon_decision.data")?;
            dec.decision_seq = seq;
            Ok(Some(dec))
        }
        None => Ok(None),
    }
}

fn load_decision_by_idempotency_in_conn(
    conn: &rusqlite::Connection,
    idempotency_key: &str,
) -> Result<Option<CanonDecisionV1>> {
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT decision_seq, data FROM canon_decision WHERE idempotency_key = ?1",
            [idempotency_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional_inner()?;
    match row {
        Some((seq, data)) => {
            let mut dec: CanonDecisionV1 =
                serde_json::from_str(&data).context("deserializing canon_decision.data")?;
            dec.decision_seq = seq;
            Ok(Some(dec))
        }
        None => Ok(None),
    }
}

fn load_events_by_decision_in_conn(
    conn: &rusqlite::Connection,
    decision_id: &str,
) -> Result<Vec<CanonEventV1>> {
    let mut stmt = conn.prepare(
        "SELECT append_seq, decision_seq, data FROM canon_event \
         WHERE decision_id = ?1 ORDER BY append_seq ASC",
    )?;
    let rows = stmt
        .query_map([decision_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (append_seq, decision_seq, data) in rows {
        let mut ev: CanonEventV1 =
            serde_json::from_str(&data).context("deserializing canon_event.data")?;
        ev.append_seq = append_seq;
        ev.decision_seq = decision_seq;
        out.push(ev);
    }
    Ok(out)
}

fn load_event_by_idempotency_in_conn(
    conn: &rusqlite::Connection,
    idempotency_key: &str,
) -> Result<Option<CanonEventV1>> {
    let row: Option<(i64, i64, String)> = conn
        .query_row(
            "SELECT append_seq, decision_seq, data FROM canon_event WHERE idempotency_key = ?1",
            [idempotency_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional_inner()?;
    match row {
        Some((append_seq, decision_seq, data)) => {
            let mut ev: CanonEventV1 =
                serde_json::from_str(&data).context("deserializing canon_event.data")?;
            ev.append_seq = append_seq;
            ev.decision_seq = decision_seq;
            Ok(Some(ev))
        }
        None => Ok(None),
    }
}

pub(crate) fn materialize_projection_view_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
    view: &CanonStateViewV1,
    now_micros: i64,
) -> Result<()> {
    let tier_str = enum_to_sql_str(view.tier)?;
    conn.execute(
        "DELETE FROM canon_projection_state \
         WHERE project_id = ?1 AND branch_id = ?2 AND tier = ?3",
        rusqlite::params![project_id, branch_id, &tier_str],
    )?;

    let mut stmt = conn.prepare(
        "INSERT INTO canon_projection_state (
            project_id, branch_id, tier, projection_kind, stream_key,
            subject_id, decision_watermark, event_watermark, reducer_version,
            source_event_ids, data, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;

    let mut insert_row = |kind: CanonProjectionKind,
                          stream_key: &str,
                          subject_id: &str,
                          source_event_ids: &[String],
                          reducer_version: u32,
                          data_json: String|
     -> Result<()> {
        let kind_str = enum_to_sql_str(kind)?;
        let source_ids_json = serde_json::to_string(source_event_ids)?;
        stmt.execute(rusqlite::params![
            project_id,
            branch_id,
            &tier_str,
            &kind_str,
            stream_key,
            subject_id,
            view.decision_watermark,
            view.event_watermark,
            reducer_version,
            &source_ids_json,
            &data_json,
            now_micros,
        ])?;
        Ok(())
    };

    for (subject_id, entity_state) in &view.entities {
        insert_row(
            CanonProjectionKind::EntityState,
            subject_id,
            subject_id,
            &entity_state.source_event_ids,
            entity_state.reducer_version,
            serde_json::to_string(entity_state)?,
        )?;
    }
    for (slot_key, rel) in &view.relationships {
        insert_row(
            CanonProjectionKind::Relationship,
            slot_key,
            &rel.subject_id,
            &rel.source_event_ids,
            rel.reducer_version,
            serde_json::to_string(rel)?,
        )?;
    }
    for (slot_key, qty) in &view.quantities {
        insert_row(
            CanonProjectionKind::Quantity,
            slot_key,
            &qty.subject_id,
            &qty.source_event_ids,
            qty.reducer_version,
            serde_json::to_string(qty)?,
        )?;
    }
    for (prop_id, k) in &view.knowledge {
        insert_row(
            CanonProjectionKind::Knowledge,
            prop_id,
            prop_id,
            &k.source_event_ids,
            k.reducer_version,
            serde_json::to_string(k)?,
        )?;
    }
    for (rule_id, rule) in &view.rules {
        insert_row(
            CanonProjectionKind::RuleConstraint,
            rule_id,
            rule_id,
            &rule.source_event_ids,
            rule.reducer_version,
            serde_json::to_string(rule)?,
        )?;
    }
    for (slot_key, clock) in &view.timeline.scene_clocks {
        let subj = clock.scene_id.as_deref().unwrap_or(slot_key.as_str());
        insert_row(
            CanonProjectionKind::TimelineClock,
            slot_key,
            subj,
            &clock.source_event_ids,
            clock.reducer_version,
            serde_json::to_string(clock)?,
        )?;
    }
    for (thread_id, thread) in &view.open_threads {
        insert_row(
            CanonProjectionKind::OpenThread,
            thread_id,
            thread_id,
            &thread.source_event_ids,
            thread.reducer_version,
            serde_json::to_string(thread)?,
        )?;
    }

    Ok(())
}

pub(crate) fn append_canon_decision_in_tx(
    tx: &rusqlite::Connection,
    mut input: AppendCanonDecisionInputV1,
    now_micros: i64,
) -> Result<AppendCanonDecisionOutputV1> {
    check_branch_in_project(tx, &input.project_id, &input.branch_id)?;
    let effective_branch_id = if input.resolve_active_descendant_branch {
        resolve_effective_scene_branch_in_conn(tx, &input.project_id, &input.branch_id)?
    } else {
        input.branch_id.clone()
    };
    input.branch_id = effective_branch_id.clone();

    anyhow::ensure!(
        !input.source_revision_id.trim().is_empty(),
        "source_revision_id must not be empty"
    );
    anyhow::ensure!(
        !input.actor.trim().is_empty(),
        "decision actor must not be empty"
    );
    anyhow::ensure!(
        !input.reason.trim().is_empty(),
        "decision reason must not be empty"
    );
    anyhow::ensure!(
        !input.events.is_empty(),
        "canon decision requires at least one event draft"
    );

    // 1. Compute fingerprints for the user-supplied drafts first so we can check
    //    whether this exact decision batch is a duplicate retry.
    let mut raw_fingerprints = Vec::with_capacity(input.events.len());
    for draft in &input.events {
        raw_fingerprints.push(semantic_event_fingerprint(
            &draft.subject,
            &draft.payload,
            draft.effective_position,
            draft.within_scene_order,
            &draft.in_world_time,
        ));
    }
    let raw_batch_fp = batch_semantic_fingerprint(&raw_fingerprints);
    let raw_decision_idem = compute_idempotency_key(
        &effective_branch_id,
        &input.source_revision_id,
        &raw_batch_fp,
        input.action,
    );

    if let Some(existing_decision) = load_decision_by_idempotency_in_conn(tx, &raw_decision_idem)? {
        let existing_events = load_events_by_decision_in_conn(tx, &existing_decision.decision_id)?;
        return Ok(AppendCanonDecisionOutputV1 {
            decision: existing_decision,
            events: existing_events,
            deduplicated: true,
        });
    }

    // Also check if every individual draft in `input.events` already exists in
    // `canon_event` under `(effective_branch_id, source_revision_id, fingerprint, action)`.
    let mut existing_events_for_all = Vec::new();
    let mut all_exist = true;
    for fp in &raw_fingerprints {
        let ev_idem = compute_idempotency_key(
            &effective_branch_id,
            &input.source_revision_id,
            fp,
            input.action,
        );
        if let Some(ev) = load_event_by_idempotency_in_conn(tx, &ev_idem)? {
            existing_events_for_all.push(ev);
        } else {
            all_exist = false;
            break;
        }
    }
    if all_exist && !existing_events_for_all.is_empty() {
        let first_dec_id = &existing_events_for_all[0].decision_id;
        if let Some(existing_decision) = load_decision_by_id_in_conn(tx, first_dec_id)? {
            return Ok(AppendCanonDecisionOutputV1 {
                decision: existing_decision,
                events: existing_events_for_all,
                deduplicated: true,
            });
        }
    }

    // 2. Load current lineage events for `effective_branch_id`.
    let lineage =
        resolve_branch_lineage_in_conn(tx, Some(&input.project_id), &effective_branch_id)?;
    let mut lineage_events = load_events_for_branches_in_conn(tx, &input.project_id, &lineage)?;

    // 3. If `replace_prior_scene_revisions` is enabled for `target_scene_id`,
    //    identify any active prior revisions/events for that scene on the lineage
    //    and prepend a `SourceInvalidation` event so recommits replace rather than
    //    accumulate prior delta/collection contributions.
    let mut drafts = input.events.clone();
    if input.replace_prior_scene_revisions
        && let Some(scene_id) = &input.target_scene_id
    {
        let current_view = state_at(
            &effective_branch_id,
            &lineage,
            &lineage_events,
            &CanonCursorV1::unbounded(),
            input.tier,
            None,
        )?;
        let visible_set: BTreeSet<&str> = current_view
            .visible_event_ids
            .iter()
            .map(String::as_str)
            .collect();
        let mut invalidated_revs = BTreeSet::new();
        let mut invalidated_ev_ids = Vec::new();
        for ev in &lineage_events {
            if visible_set.contains(ev.event_id.as_str())
                && ev.scene_id.as_deref() == Some(scene_id.as_str())
                && ev.source_revision_id != input.source_revision_id
            {
                invalidated_revs.insert(ev.source_revision_id.clone());
                invalidated_ev_ids.push(ev.event_id.clone());
            }
        }
        if !invalidated_revs.is_empty() || !invalidated_ev_ids.is_empty() {
            let rev_list: Vec<String> = invalidated_revs.into_iter().collect();
            for old_rev in &rev_list {
                tx.execute(
                    "UPDATE canon_claim SET status = 'source_invalidated' \
                     WHERE project_id = ?1 AND branch_id = ?2 AND scene_id = ?3 AND revision_id = ?4",
                    rusqlite::params![&input.project_id, &effective_branch_id, scene_id, old_rev],
                )?;
            }
            let invalidation_draft = CanonEventDraftV1 {
                subject: ClaimEntity::Resolved {
                    id: format!("scene:{scene_id}"),
                },
                payload: CanonEventPayloadV1::SourceInvalidation {
                    invalidation: SourceInvalidationV1 {
                        scene_id: Some(scene_id.clone()),
                        invalidated_revision_ids: rev_list,
                        invalidated_event_ids: invalidated_ev_ids,
                        replacement_revision_id: Some(input.source_revision_id.clone()),
                    },
                },
                effective_position: drafts.first().and_then(|d| d.effective_position),
                within_scene_order: Some(0),
                in_world_time: None,
                scene_id: Some(scene_id.clone()),
                source_claim_ids: Vec::new(),
                evidence_sha256: exact_text_sha256(&input.source_revision_id),
                provenance: CanonProvenanceKind::SystemRestoration,
                supersedes_event_id: None,
                withdraws_event_id: None,
                legacy_record_scope: None,
                merge_provenance: None,
            };
            drafts.insert(0, invalidation_draft);
        }
    }

    // 4. Materialize candidate `CanonEventV1` records (excluding any individual
    //    draft that was already committed for this revision/action).
    let decision_id = mint_id_local("canon_decision");
    let next_watermark_probe_seq: i64 = tx.query_row(
        "SELECT COALESCE(MAX(decision_seq), 0) + 1 FROM canon_decision",
        [],
        |r| r.get(0),
    )?;
    let next_event_probe_seq: i64 = tx.query_row(
        "SELECT COALESCE(MAX(append_seq), 0) + 1 FROM canon_event",
        [],
        |r| r.get(0),
    )?;

    let mut candidate_events: Vec<CanonEventV1> = Vec::with_capacity(drafts.len());
    let mut reused_events: Vec<CanonEventV1> = Vec::new();

    for (idx, draft) in drafts.into_iter().enumerate() {
        let sem_fp = semantic_event_fingerprint(
            &draft.subject,
            &draft.payload,
            draft.effective_position,
            draft.within_scene_order,
            &draft.in_world_time,
        );
        let ev_idem = compute_idempotency_key(
            &effective_branch_id,
            &input.source_revision_id,
            &sem_fp,
            input.action,
        );
        if let Some(existing_ev) = load_event_by_idempotency_in_conn(tx, &ev_idem)? {
            reused_events.push(existing_ev);
            continue;
        }
        let (predicate, operation) = draft.payload.predicate_and_operation();
        let predicate_family = draft.payload.family();
        candidate_events.push(CanonEventV1 {
            event_id: mint_id_local("canon_event"),
            schema_version: CANON_EVENT_SCHEMA_VERSION,
            project_id: input.project_id.clone(),
            branch_id: effective_branch_id.clone(),
            decision_id: decision_id.clone(),
            decision_seq: next_watermark_probe_seq,
            append_seq: next_event_probe_seq + idx as i64,
            tier: input.tier,
            policy_action: input.action,
            subject: draft.subject,
            predicate_family,
            predicate: predicate.to_string(),
            operation: operation.to_string(),
            payload: draft.payload,
            effective_position: draft.effective_position,
            within_scene_order: draft.within_scene_order,
            in_world_time: draft.in_world_time,
            scene_id: draft.scene_id,
            source_revision_id: input.source_revision_id.clone(),
            source_claim_ids: draft.source_claim_ids,
            semantic_fingerprint: sem_fp,
            idempotency_key: ev_idem,
            evidence_sha256: draft.evidence_sha256,
            provenance: draft.provenance,
            supersedes_event_id: draft.supersedes_event_id,
            withdraws_event_id: draft.withdraws_event_id,
            legacy_record_scope: draft.legacy_record_scope,
            merge_provenance: draft.merge_provenance,
            created_at_micros: now_micros,
        });
    }

    if candidate_events.is_empty() && !reused_events.is_empty() {
        let first_dec_id = &reused_events[0].decision_id;
        if let Some(existing_decision) = load_decision_by_id_in_conn(tx, first_dec_id)? {
            return Ok(AppendCanonDecisionOutputV1 {
                decision: existing_decision,
                events: reused_events,
                deduplicated: true,
            });
        }
    }

    // 5. Validate candidate batch preconditions in exact reducer order against
    //    existing lineage history (including within-scene step bounds, prepended
    //    `SourceInvalidation` control events, and intervening history for
    //    multi-position batches).
    let prior_watermark = lineage_events
        .iter()
        .map(|e| e.decision_seq)
        .max()
        .unwrap_or(0);
    validate_event_batch_in_history(
        &effective_branch_id,
        &lineage,
        &lineage_events,
        &input.actor,
        input.action,
        input.tier,
        &candidate_events,
    )?;

    // 6. Persist `canon_decision` and each `canon_event` atomically.
    let action_str = enum_to_sql_str(input.action)?;
    let policy_version = if input.policy_version.trim().is_empty() {
        CANON_POLICY_VERSION_V1.to_string()
    } else {
        input.policy_version.clone()
    };

    let mut decision = CanonDecisionV1 {
        decision_id: decision_id.clone(),
        decision_seq: 0,
        project_id: input.project_id.clone(),
        branch_id: effective_branch_id.clone(),
        action: input.action,
        actor: input.actor.clone(),
        policy_version: policy_version.clone(),
        source_revision_id: input.source_revision_id.clone(),
        semantic_fingerprint: raw_batch_fp.clone(),
        idempotency_key: raw_decision_idem.clone(),
        projection_watermark: prior_watermark,
        previous_decision_id: input.previous_decision_id.clone(),
        target_scene_id: input.target_scene_id.clone(),
        confidence_inputs: input.confidence_inputs.clone(),
        precondition_summary: "validated".into(),
        reason: input.reason.clone(),
        event_ids: candidate_events
            .iter()
            .map(|e| e.event_id.clone())
            .collect(),
        created_at_micros: now_micros,
    };

    tx.execute(
        "INSERT INTO canon_decision (
            id, project_id, branch_id, action, actor, policy_version,
            source_revision_id, semantic_fingerprint, idempotency_key,
            projection_watermark, previous_decision_id, target_scene_id,
            reason, data, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            &decision.decision_id,
            &decision.project_id,
            &decision.branch_id,
            &action_str,
            &decision.actor,
            &decision.policy_version,
            &decision.source_revision_id,
            &decision.semantic_fingerprint,
            &decision.idempotency_key,
            decision.projection_watermark,
            &decision.previous_decision_id,
            &decision.target_scene_id,
            &decision.reason,
            serde_json::to_string(&decision)?,
            now_micros,
        ],
    )?;
    let decision_seq = tx.last_insert_rowid();
    decision.decision_seq = decision_seq;
    tx.execute(
        "UPDATE canon_decision SET data = ?2 WHERE id = ?1",
        rusqlite::params![&decision.decision_id, serde_json::to_string(&decision)?],
    )?;

    let tier_str = enum_to_sql_str(input.tier)?;
    for event in &mut candidate_events {
        event.decision_seq = decision_seq;
        let family_str = enum_to_sql_str(event.predicate_family)?;
        let prov_str = enum_to_sql_str(event.provenance)?;
        let (book_num, chap_num, scene_ord) = match event.effective_position {
            Some(pos) => (
                Some(pos.book_number),
                Some(pos.chapter_number),
                Some(pos.scene_order),
            ),
            None => (None, None, None),
        };
        let in_world_json = event
            .in_world_time
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let claim_ids_json = serde_json::to_string(&event.source_claim_ids)?;
        let payload_json = serde_json::to_string(&event.payload)?;
        let data_json = serde_json::to_string(event)?;

        tx.execute(
            "INSERT INTO canon_event (
                id, project_id, branch_id, decision_id, decision_seq,
                tier, policy_action, subject_id, predicate_family, predicate,
                operation, book_number, chapter_number, scene_order, within_scene_order,
                in_world_time, scene_id, source_revision_id, source_claim_ids,
                semantic_fingerprint, idempotency_key, evidence_hash, provenance,
                supersedes_event_id, withdraws_event_id, payload, data, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19,
                ?20, ?21, ?22, ?23,
                ?24, ?25, ?26, ?27, ?28
            )",
            rusqlite::params![
                &event.event_id,
                &event.project_id,
                &event.branch_id,
                &event.decision_id,
                decision_seq,
                &tier_str,
                &action_str,
                event.subject_key(),
                &family_str,
                &event.predicate,
                &event.operation,
                book_num,
                chap_num,
                scene_ord,
                event.within_scene_order,
                &in_world_json,
                &event.scene_id,
                &event.source_revision_id,
                &claim_ids_json,
                &event.semantic_fingerprint,
                &event.idempotency_key,
                &event.evidence_sha256,
                &prov_str,
                &event.supersedes_event_id,
                &event.withdraws_event_id,
                &payload_json,
                &data_json,
                now_micros,
            ],
        )?;
        let append_seq = tx.last_insert_rowid();
        event.append_seq = append_seq;
        tx.execute(
            "UPDATE canon_event SET data = ?2 WHERE id = ?1",
            rusqlite::params![&event.event_id, serde_json::to_string(event)?],
        )?;
    }

    // 7. Refresh materialized projections for `effective_branch_id`.
    lineage_events.extend(candidate_events.iter().cloned());
    for refresh_tier in [CanonTier::Ratified, CanonTier::Provisional] {
        let refreshed = state_at(
            &effective_branch_id,
            &lineage,
            &lineage_events,
            &CanonCursorV1::unbounded(),
            refresh_tier,
            None,
        )?;
        materialize_projection_view_in_conn(
            tx,
            &input.project_id,
            &effective_branch_id,
            &refreshed,
            now_micros,
        )?;
    }

    let mut all_returned_events = reused_events;
    all_returned_events.extend(candidate_events);

    Ok(AppendCanonDecisionOutputV1 {
        decision,
        events: all_returned_events,
        deduplicated: false,
    })
}

impl Repository {
    pub async fn record_canon_claims(
        &self,
        project_id: &str,
        branch_id: &str,
        claims: &[ClaimV1],
        status: CanonClaimStatus,
        tier: CanonTier,
        provenance: CanonProvenanceKind,
    ) -> Result<Vec<CanonClaimRecordV1>> {
        let project_id = project_id.to_owned();
        let branch_id = branch_id.to_owned();
        let claims = claims.to_vec();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    check_branch_in_project(&tx, &project_id, &branch_id)?;
                    let status_str = enum_to_sql_str(status)?;
                    let tier_str = enum_to_sql_str(tier)?;
                    let prov_str = enum_to_sql_str(provenance)?;
                    let mut out = Vec::with_capacity(claims.len());

                    for claim in claims {
                        let sem_fp = semantic_claim_fingerprint(&claim);
                        let source_str = enum_to_sql_str(claim.source)?;
                        let existing: Option<String> = tx
                            .query_row(
                                "SELECT data FROM canon_claim \
                                 WHERE project_id = ?1 AND branch_id = ?2 \
                                   AND revision_id = ?3 AND semantic_fingerprint = ?4 \
                                   AND source = ?5 \
                                 LIMIT 1",
                                rusqlite::params![
                                    &project_id,
                                    &branch_id,
                                    &claim.binding.revision_id,
                                    &sem_fp,
                                    &source_str,
                                ],
                                |r| r.get(0),
                            )
                            .optional_inner()?;
                        if let Some(data_json) = existing {
                            out.push(serde_json::from_str(&data_json)?);
                            continue;
                        }

                        let rec = CanonClaimRecordV1 {
                            claim_record_id: mint_id_local("canon_claim"),
                            project_id: project_id.clone(),
                            branch_id: branch_id.clone(),
                            scene_id: claim.binding.scene_id.clone(),
                            revision_id: claim.binding.revision_id.clone(),
                            source: claim.source,
                            semantic_fingerprint: sem_fp.clone(),
                            status,
                            tier,
                            provenance,
                            claim,
                            created_at_micros: now,
                        };
                        let data_json = serde_json::to_string(&rec)?;
                        tx.execute(
                            "INSERT INTO canon_claim (
                                id, project_id, branch_id, scene_id, revision_id,
                                source, semantic_fingerprint, status, tier,
                                provenance, data, created_at
                            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                            rusqlite::params![
                                &rec.claim_record_id,
                                &rec.project_id,
                                &rec.branch_id,
                                &rec.scene_id,
                                &rec.revision_id,
                                &source_str,
                                &rec.semantic_fingerprint,
                                &status_str,
                                &tier_str,
                                &prov_str,
                                &data_json,
                                now,
                            ],
                        )?;
                        out.push(rec);
                    }

                    tx.commit()?;
                    Ok(out)
                })())
            })
            .await?
    }

    pub async fn list_canon_claims(
        &self,
        project_id: &str,
        branch_id: &str,
        scene_id: Option<&str>,
        revision_id: Option<&str>,
    ) -> Result<Vec<CanonClaimRecordV1>> {
        let project_id = project_id.to_owned();
        let branch_id = branch_id.to_owned();
        let scene_id = scene_id.map(str::to_owned);
        let revision_id = revision_id.map(str::to_owned);

        self.inner
            .pool
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    let mut stmt = conn.prepare(
                        "SELECT data FROM canon_claim \
                         WHERE project_id = ?1 AND branch_id = ?2 \
                           AND (?3 IS NULL OR scene_id = ?3) \
                           AND (?4 IS NULL OR revision_id = ?4) \
                         ORDER BY created_at ASC, id ASC",
                    )?;
                    let rows = stmt
                        .query_map(
                            rusqlite::params![&project_id, &branch_id, &scene_id, &revision_id],
                            |r| r.get::<_, String>(0),
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    rows.into_iter()
                        .map(|data| Ok(serde_json::from_str(&data)?))
                        .collect()
                })())
            })
            .await?
    }

    pub async fn append_canon_decision(
        &self,
        input: AppendCanonDecisionInputV1,
    ) -> Result<AppendCanonDecisionOutputV1> {
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    let out = append_canon_decision_in_tx(&tx, input, now)?;
                    tx.commit()?;
                    Ok(out)
                })())
            })
            .await?
    }

    pub async fn canon_state_at(
        &self,
        project_id: &str,
        branch_id: &str,
        cursor: CanonCursorV1,
        tier: CanonTier,
        decision_watermark: Option<i64>,
    ) -> Result<CanonStateViewV1> {
        let project_id = project_id.to_owned();
        let branch_id = branch_id.to_owned();
        self.inner
            .pool
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    check_branch_in_project(conn, &project_id, &branch_id)?;
                    let lineage =
                        resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                    let events = load_events_for_branches_in_conn(conn, &project_id, &lineage)?;
                    Ok(state_at(
                        &branch_id,
                        &lineage,
                        &events,
                        &cursor,
                        tier,
                        decision_watermark,
                    )?)
                })())
            })
            .await?
    }

    pub async fn list_canon_events_for_lineage(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CanonEventV1>> {
        let project_id = project_id.to_owned();
        let branch_id = branch_id.to_owned();
        self.inner
            .pool
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    check_branch_in_project(conn, &project_id, &branch_id)?;
                    let lineage =
                        resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                    load_events_for_branches_in_conn(conn, &project_id, &lineage)
                })())
            })
            .await?
    }

    pub async fn rebuild_canon_projections(
        &self,
        project_id: &str,
        branch_id: &str,
        tier: CanonTier,
    ) -> Result<CanonStateViewV1> {
        let project_id = project_id.to_owned();
        let branch_id = branch_id.to_owned();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    check_branch_in_project(&tx, &project_id, &branch_id)?;
                    let lineage =
                        resolve_branch_lineage_in_conn(&tx, Some(&project_id), &branch_id)?;
                    let events = load_events_for_branches_in_conn(&tx, &project_id, &lineage)?;
                    let view = state_at(
                        &branch_id,
                        &lineage,
                        &events,
                        &CanonCursorV1::unbounded(),
                        tier,
                        None,
                    )?;
                    materialize_projection_view_in_conn(&tx, &project_id, &branch_id, &view, now)?;
                    tx.commit()?;
                    Ok(view)
                })())
            })
            .await?
    }

    pub async fn list_canon_projection_rows(
        &self,
        project_id: &str,
        branch_id: &str,
        tier: CanonTier,
    ) -> Result<Vec<CanonProjectionRowV1>> {
        let project_id = project_id.to_owned();
        let branch_id = branch_id.to_owned();
        let tier_str = enum_to_sql_str(tier)?;
        self.inner
            .pool
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    let mut stmt = conn.prepare(
                        "SELECT projection_kind, stream_key, subject_id, decision_watermark, \
                                event_watermark, reducer_version, source_event_ids, data, updated_at \
                         FROM canon_projection_state \
                         WHERE project_id = ?1 AND branch_id = ?2 AND tier = ?3 \
                         ORDER BY projection_kind ASC, stream_key ASC",
                    )?;
                    let rows = stmt
                        .query_map(rusqlite::params![&project_id, &branch_id, &tier_str], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, i64>(3)?,
                                r.get::<_, i64>(4)?,
                                r.get::<_, u32>(5)?,
                                r.get::<_, String>(6)?,
                                r.get::<_, String>(7)?,
                                r.get::<_, i64>(8)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    let mut out = Vec::with_capacity(rows.len());
                    for (
                        kind_s,
                        stream_key,
                        subject_id,
                        dec_wm,
                        ev_wm,
                        red_ver,
                        src_ids_json,
                        data_json,
                        updated_at,
                    ) in rows
                    {
                        let projection_kind: CanonProjectionKind =
                            serde_json::from_value(Value::String(kind_s))?;
                        let source_event_ids: Vec<String> = serde_json::from_str(&src_ids_json)?;
                        out.push(CanonProjectionRowV1 {
                            project_id: project_id.clone(),
                            branch_id: branch_id.clone(),
                            tier,
                            projection_kind,
                            stream_key,
                            subject_id,
                            decision_watermark: dec_wm,
                            event_watermark: ev_wm,
                            reducer_version: red_ver,
                            source_event_ids,
                            data_json,
                            updated_at_micros: updated_at,
                        });
                    }
                    Ok(out)
                })())
            })
            .await?
    }

    pub async fn create_canon_checkpoint(
        &self,
        project_id: &str,
        branch_id: &str,
        save_point_id: Option<String>,
        tier: CanonTier,
    ) -> Result<CanonSavepointCheckpointV1> {
        let project_id = project_id.to_owned();
        let branch_id = branch_id.to_owned();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    check_branch_in_project(&tx, &project_id, &branch_id)?;
                    let lineage =
                        resolve_branch_lineage_in_conn(&tx, Some(&project_id), &branch_id)?;
                    let events = load_events_for_branches_in_conn(&tx, &project_id, &lineage)?;
                    let view = state_at(
                        &branch_id,
                        &lineage,
                        &events,
                        &CanonCursorV1::unbounded(),
                        tier,
                        None,
                    )?;
                    let mut branch_watermarks = BTreeMap::new();
                    for b in &lineage {
                        let wm = events
                            .iter()
                            .filter(|e| &e.branch_id == b)
                            .map(|e| e.decision_seq)
                            .max()
                            .unwrap_or(0);
                        branch_watermarks.insert(b.clone(), wm);
                    }
                    let checkpoint = CanonSavepointCheckpointV1 {
                        checkpoint_id: mint_id_local("canon_checkpoint"),
                        project_id: project_id.clone(),
                        branch_id: branch_id.clone(),
                        save_point_id,
                        tier,
                        lineage,
                        decision_watermark: view.decision_watermark,
                        event_watermark: view.event_watermark,
                        branch_decision_watermarks: branch_watermarks,
                        reducer_version: CANON_REDUCER_VERSION,
                        schema_version: CANON_EVENT_SCHEMA_VERSION,
                        created_at_micros: now,
                    };
                    let tier_str = enum_to_sql_str(tier)?;
                    tx.execute(
                        "INSERT INTO canon_projection_checkpoint (
                            id, project_id, branch_id, save_point_id, tier,
                            decision_watermark, event_watermark, reducer_version,
                            schema_version, lineage, branch_watermarks, state_snapshot, created_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                        rusqlite::params![
                            &checkpoint.checkpoint_id,
                            &checkpoint.project_id,
                            &checkpoint.branch_id,
                            &checkpoint.save_point_id,
                            &tier_str,
                            checkpoint.decision_watermark,
                            checkpoint.event_watermark,
                            checkpoint.reducer_version,
                            checkpoint.schema_version,
                            serde_json::to_string(&checkpoint.lineage)?,
                            serde_json::to_string(&checkpoint.branch_decision_watermarks)?,
                            serde_json::to_string(&view)?,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(checkpoint)
                })())
            })
            .await?
    }

    pub async fn restore_canon_checkpoint(
        &self,
        project_id: &str,
        branch_id: &str,
        checkpoint_id: &str,
        actor: &str,
        reason: &str,
    ) -> Result<CanonStateViewV1> {
        let project_id = project_id.to_owned();
        let branch_id = branch_id.to_owned();
        let checkpoint_id = checkpoint_id.to_owned();
        let actor = actor.to_owned();
        let reason = reason.to_owned();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    check_branch_in_project(&tx, &project_id, &branch_id)?;
                    let (cp_branch, tier_str, dec_wm, ev_wm, save_point_id): (
                        String,
                        String,
                        i64,
                        i64,
                        Option<String>,
                    ) = tx
                        .query_row(
                            "SELECT branch_id, tier, decision_watermark, event_watermark, save_point_id \
                             FROM canon_projection_checkpoint \
                             WHERE id = ?1 AND project_id = ?2",
                            rusqlite::params![&checkpoint_id, &project_id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                        )
                        .optional_inner()?
                        .ok_or_else(|| anyhow!("canon checkpoint {checkpoint_id} not found"))?;
                    anyhow::ensure!(
                        cp_branch == branch_id,
                        "checkpoint {checkpoint_id} belongs to branch {cp_branch}, not {branch_id}"
                    );
                    let tier: CanonTier = serde_json::from_value(Value::String(tier_str))?;

                    let current_max_dec: i64 = tx.query_row(
                        "SELECT COALESCE(MAX(decision_seq), 0) FROM canon_decision",
                        [],
                        |r| r.get(0),
                    )?;
                    let rev_id = format!("restore:{checkpoint_id}:after:{current_max_dec}");

                    let restore_input = AppendCanonDecisionInputV1 {
                        project_id: project_id.clone(),
                        branch_id: branch_id.clone(),
                        resolve_active_descendant_branch: false,
                        action: CanonPolicyAction::RestoreWatermark,
                        tier,
                        actor,
                        policy_version: CANON_POLICY_VERSION_V1.into(),
                        source_revision_id: rev_id.clone(),
                        target_scene_id: None,
                        replace_prior_scene_revisions: false,
                        previous_decision_id: None,
                        confidence_inputs: Vec::new(),
                        reason,
                        events: vec![CanonEventDraftV1 {
                            subject: ClaimEntity::Resolved {
                                id: format!("branch:{branch_id}"),
                            },
                            payload: CanonEventPayloadV1::WatermarkRestore {
                                restore: WatermarkRestoreV1 {
                                    save_point_id,
                                    target_decision_seq: dec_wm,
                                    target_event_seq: ev_wm,
                                },
                            },
                            effective_position: None,
                            within_scene_order: None,
                            in_world_time: None,
                            scene_id: None,
                            source_claim_ids: Vec::new(),
                            evidence_sha256: exact_text_sha256(&rev_id),
                            provenance: CanonProvenanceKind::SystemRestoration,
                            supersedes_event_id: None,
                            withdraws_event_id: None,
                            legacy_record_scope: None,
                            merge_provenance: None,
                        }],
                    };
                    append_canon_decision_in_tx(&tx, restore_input, now)?;

                    let lineage =
                        resolve_branch_lineage_in_conn(&tx, Some(&project_id), &branch_id)?;
                    let events = load_events_for_branches_in_conn(&tx, &project_id, &lineage)?;
                    let restored_view = state_at(
                        &branch_id,
                        &lineage,
                        &events,
                        &CanonCursorV1::unbounded(),
                        tier,
                        None,
                    )?;
                    tx.commit()?;
                    Ok(restored_view)
                })())
            })
            .await?
    }

    pub async fn merge_canon_branch_events(
        &self,
        project_id: &str,
        common_ancestor_branch_id: &str,
        target_branch_id: &str,
        source_branch_id: &str,
        actor: &str,
        reason: &str,
    ) -> Result<(CanonMergePlanV1, Option<AppendCanonDecisionOutputV1>)> {
        let project_id = project_id.to_owned();
        let common_ancestor_branch_id = common_ancestor_branch_id.to_owned();
        let target_branch_id = target_branch_id.to_owned();
        let source_branch_id = source_branch_id.to_owned();
        let actor = actor.to_owned();
        let reason = reason.to_owned();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    for b in [
                        &common_ancestor_branch_id,
                        &target_branch_id,
                        &source_branch_id,
                    ] {
                        check_branch_in_project(&tx, &project_id, b)?;
                    }
                    let ancestor_lineage = resolve_branch_lineage_in_conn(
                        &tx,
                        Some(&project_id),
                        &common_ancestor_branch_id,
                    )?;
                    let target_lineage =
                        resolve_branch_lineage_in_conn(&tx, Some(&project_id), &target_branch_id)?;
                    let source_lineage =
                        resolve_branch_lineage_in_conn(&tx, Some(&project_id), &source_branch_id)?;

                    let mut union_branches: Vec<String> = ancestor_lineage
                        .iter()
                        .chain(target_lineage.iter())
                        .chain(source_lineage.iter())
                        .cloned()
                        .collect();
                    union_branches.sort();
                    union_branches.dedup();

                    let events =
                        load_events_for_branches_in_conn(&tx, &project_id, &union_branches)?;
                    let plan = plan_canon_branch_merge(
                        &common_ancestor_branch_id,
                        &ancestor_lineage,
                        &target_branch_id,
                        &target_lineage,
                        &source_branch_id,
                        &source_lineage,
                        &events,
                        None,
                    )?;

                    if !plan.conflicts.is_empty() || plan.accepted_source_events.is_empty() {
                        tx.commit()?;
                        return Ok((plan, None));
                    }

                    let max_src_seq = plan
                        .accepted_source_events
                        .iter()
                        .map(|e| e.append_seq)
                        .max()
                        .unwrap_or(0);
                    let merge_revision_id =
                        format!("merge:{source_branch_id}->{target_branch_id}:up_to:{max_src_seq}");
                    let drafts: Vec<CanonEventDraftV1> = plan
                        .accepted_source_events
                        .iter()
                        .map(|src_ev| {
                            let mut payload = src_ev.payload.clone();
                            if let CanonEventPayloadV1::SourceInvalidation { invalidation } =
                                &mut payload
                                && !invalidation.invalidated_revision_ids.is_empty()
                            {
                                for candidate in &events {
                                    if candidate.event_id == src_ev.event_id {
                                        continue;
                                    }
                                    let matches_rev =
                                        invalidation.invalidated_revision_ids.iter().any(|rev| {
                                            candidate.source_revision_id == *rev
                                                || candidate
                                                    .merge_provenance
                                                    .as_ref()
                                                    .and_then(|mp| mp.source_revision_id.as_deref())
                                                    .is_some_and(|src_rev| src_rev == rev)
                                        });
                                    let matches_scene = invalidation
                                        .scene_id
                                        .as_ref()
                                        .is_none_or(|s| candidate.scene_id.as_ref() == Some(s));
                                    if matches_rev && matches_scene {
                                        if !invalidation
                                            .invalidated_event_ids
                                            .contains(&candidate.event_id)
                                        {
                                            invalidation
                                                .invalidated_event_ids
                                                .push(candidate.event_id.clone());
                                        }
                                        if let Some(mp) = &candidate.merge_provenance
                                            && !invalidation
                                                .invalidated_event_ids
                                                .contains(&mp.source_event_id)
                                        {
                                            invalidation
                                                .invalidated_event_ids
                                                .push(mp.source_event_id.clone());
                                        }
                                    }
                                }
                            }
                            CanonEventDraftV1 {
                                subject: src_ev.subject.clone(),
                                payload,
                                effective_position: src_ev.effective_position,
                                within_scene_order: src_ev.within_scene_order,
                                in_world_time: src_ev.in_world_time.clone(),
                                scene_id: src_ev.scene_id.clone(),
                                source_claim_ids: src_ev.source_claim_ids.clone(),
                                evidence_sha256: src_ev.evidence_sha256.clone(),
                                provenance: CanonProvenanceKind::Merged,
                                supersedes_event_id: src_ev.supersedes_event_id.clone(),
                                withdraws_event_id: src_ev.withdraws_event_id.clone(),
                                legacy_record_scope: src_ev.legacy_record_scope.clone(),
                                merge_provenance: Some(MergeEventProvenanceV1 {
                                    source_branch_id: src_ev.branch_id.clone(),
                                    source_event_id: src_ev.event_id.clone(),
                                    source_decision_id: src_ev.decision_id.clone(),
                                    common_ancestor_branch_id: Some(
                                        common_ancestor_branch_id.clone(),
                                    ),
                                    source_revision_id: Some(src_ev.source_revision_id.clone()),
                                }),
                            }
                        })
                        .collect();

                    let output = append_canon_decision_in_tx(
                        &tx,
                        AppendCanonDecisionInputV1 {
                            project_id,
                            branch_id: target_branch_id,
                            resolve_active_descendant_branch: false,
                            action: CanonPolicyAction::MergeAccept,
                            tier: CanonTier::Ratified,
                            actor,
                            policy_version: CANON_POLICY_VERSION_V1.into(),
                            source_revision_id: merge_revision_id,
                            target_scene_id: None,
                            replace_prior_scene_revisions: false,
                            previous_decision_id: None,
                            confidence_inputs: Vec::new(),
                            reason,
                            events: drafts,
                        },
                        now,
                    )?;
                    tx.commit()?;
                    Ok((plan, Some(output)))
                })())
            })
            .await?
    }

    pub async fn upsert_canon_source_map(
        &self,
        entry: CanonSourceMapEntryV1,
    ) -> Result<CanonSourceMapEntryV1> {
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    check_branch_in_project(&tx, &entry.project_id, &entry.branch_id)?;
                    let status_str = enum_to_sql_str(entry.status)?;
                    let prov_str = enum_to_sql_str(entry.provenance)?;
                    let claim_ids_json = serde_json::to_string(&entry.claim_ids)?;
                    let event_ids_json = serde_json::to_string(&entry.event_ids)?;
                    let created_at = if entry.created_at_micros > 0 {
                        entry.created_at_micros
                    } else {
                        now
                    };
                    let mut stored = entry;
                    stored.created_at_micros = created_at;
                    stored.updated_at_micros = now;

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
                            &stored.project_id,
                            &stored.branch_id,
                            &stored.source_table,
                            &stored.legacy_primary_key,
                            stored.migration_version,
                            &status_str,
                            &prov_str,
                            &claim_ids_json,
                            &event_ids_json,
                            &stored.decision_id,
                            &stored.quarantine_reason,
                            &stored.raw_snapshot_json,
                            stored.created_at_micros,
                            stored.updated_at_micros,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(stored)
                })())
            })
            .await?
    }

    pub async fn list_canon_source_maps(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        source_table: Option<&str>,
    ) -> Result<Vec<CanonSourceMapEntryV1>> {
        let project_id = project_id.to_owned();
        let branch_id = branch_id.map(str::to_owned);
        let source_table = source_table.map(str::to_owned);
        self.inner
            .pool
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    let mut stmt = conn.prepare(
                        "SELECT project_id, branch_id, source_table, legacy_primary_key, \
                                migration_version, status, provenance, claim_ids, event_ids, \
                                decision_id, quarantine_reason, raw_snapshot, created_at, updated_at \
                         FROM canon_source_map \
                         WHERE project_id = ?1 \
                           AND (?2 IS NULL OR branch_id = ?2) \
                           AND (?3 IS NULL OR source_table = ?3) \
                         ORDER BY source_table ASC, legacy_primary_key ASC, migration_version ASC",
                    )?;
                    let rows = stmt
                        .query_map(
                            rusqlite::params![&project_id, &branch_id, &source_table],
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
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    let mut out = Vec::with_capacity(rows.len());
                    for (
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
                    ) in rows
                    {
                        out.push(CanonSourceMapEntryV1 {
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
                        });
                    }
                    Ok(out)
                })())
            })
            .await?
    }
}
