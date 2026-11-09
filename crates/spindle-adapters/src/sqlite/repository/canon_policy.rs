//! SQLite orchestration for calibrated canon policy decisions, audited
//! ratification, and revocation (P2 PR E).

use super::canon_backfill::mark_legacy_rows_mapped_in_conn;
use super::canon_cutover::canon_cutover_mode_in_conn;
use super::canon_events::{
    append_canon_policy_decision_in_tx, enum_to_sql_str, load_events_by_decision_in_conn,
    load_events_for_branches_in_conn,
};
use super::*;
use spindle_core::canon::*;
use std::collections::{BTreeMap, BTreeSet};

fn policy_event_draft(
    claim: &ClaimV1,
    provenance: CanonProvenanceKind,
    supersedes_event_id: Option<String>,
) -> CanonEventDraftV1 {
    CanonEventDraftV1 {
        subject: claim.proposed.subject.clone(),
        payload: CanonEventPayloadV1::Assertion {
            assertion: claim.proposed.assertion.clone(),
            modality: claim.proposed.modality,
            temporal_scope: claim.proposed.temporal_scope,
            speaker: claim.proposed.speaker.clone(),
            experiencer: claim.proposed.experiencer.clone(),
        },
        effective_position: story_placement_to_canon_position(&claim.binding.position),
        within_scene_order: claim.proposed.within_scene_order,
        in_world_time: claim.proposed.in_world_time.clone(),
        scene_id: claim.binding.scene_id.clone(),
        source_claim_ids: vec![claim.proposed.claim_id.clone()],
        evidence_sha256: evidence_sha256(&claim.proposed.evidence),
        provenance,
        supersedes_event_id,
        withdraws_event_id: None,
        legacy_record_scope: None,
        merge_provenance: None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn upsert_canon_claim_in_conn(
    tx: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
    claim: &ClaimV1,
    status: CanonClaimStatus,
    tier: CanonTier,
    provenance: CanonProvenanceKind,
    now_micros: i64,
) -> Result<CanonClaimRecordV1> {
    let sem_fp = semantic_claim_fingerprint(claim);
    let source_str = enum_to_sql_str(claim.source)?;
    let status_str = enum_to_sql_str(status)?;
    let tier_str = enum_to_sql_str(tier)?;
    let prov_str = enum_to_sql_str(provenance)?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT data FROM canon_claim \
             WHERE project_id = ?1 AND branch_id = ?2 \
               AND revision_id = ?3 AND semantic_fingerprint = ?4 \
               AND source = ?5 \
             LIMIT 1",
            rusqlite::params![
                project_id,
                branch_id,
                &claim.binding.revision_id,
                &sem_fp,
                &source_str,
            ],
            |row| row.get(0),
        )
        .optional_inner()?;
    if let Some(data_json) = existing {
        let mut record: CanonClaimRecordV1 = serde_json::from_str(&data_json)?;
        record.status = status;
        record.tier = tier;
        record.provenance = provenance;
        record.claim = claim.clone();
        let updated_json = serde_json::to_string(&record)?;
        tx.execute(
            "UPDATE canon_claim SET status = ?2, tier = ?3, provenance = ?4, data = ?5 \
             WHERE id = ?1",
            rusqlite::params![
                &record.claim_record_id,
                &status_str,
                &tier_str,
                &prov_str,
                &updated_json,
            ],
        )?;
        return Ok(record);
    }
    let record = CanonClaimRecordV1 {
        claim_record_id: mint_id_local("canon_claim"),
        project_id: project_id.to_string(),
        branch_id: branch_id.to_string(),
        scene_id: claim.binding.scene_id.clone(),
        revision_id: claim.binding.revision_id.clone(),
        source: claim.source,
        semantic_fingerprint: sem_fp,
        status,
        tier,
        provenance,
        claim: claim.clone(),
        created_at_micros: now_micros,
    };
    let data_json = serde_json::to_string(&record)?;
    tx.execute(
        "INSERT INTO canon_claim (
            id, project_id, branch_id, scene_id, revision_id,
            source, semantic_fingerprint, status, tier,
            provenance, data, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            &record.claim_record_id,
            &record.project_id,
            &record.branch_id,
            &record.scene_id,
            &record.revision_id,
            &source_str,
            &record.semantic_fingerprint,
            &status_str,
            &tier_str,
            &prov_str,
            &data_json,
            now_micros,
        ],
    )?;
    Ok(record)
}

fn load_canon_claims_for_branch_in_conn(
    tx: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
) -> Result<Vec<CanonClaimRecordV1>> {
    let mut stmt = tx.prepare(
        "SELECT data FROM canon_claim \
         WHERE project_id = ?1 AND branch_id = ?2 ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map(rusqlite::params![project_id, branch_id], |row| {
        row.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(serde_json::from_str(&row?)?);
    }
    Ok(out)
}

fn load_decision_in_conn(tx: &rusqlite::Connection, decision_id: &str) -> Result<CanonDecisionV1> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM canon_decision WHERE id = ?1",
            [decision_id],
            |row| row.get(0),
        )
        .optional_inner()?;
    data.map(|json| serde_json::from_str(&json))
        .transpose()?
        .ok_or_else(|| anyhow!("canon decision not found"))
}

fn withdraw_draft(event: &CanonEventV1) -> CanonEventDraftV1 {
    CanonEventDraftV1 {
        subject: event.subject.clone(),
        payload: event.payload.clone(),
        effective_position: event.effective_position,
        within_scene_order: event.within_scene_order,
        in_world_time: event.in_world_time.clone(),
        scene_id: event.scene_id.clone(),
        source_claim_ids: event.source_claim_ids.clone(),
        evidence_sha256: event.evidence_sha256.clone(),
        provenance: CanonProvenanceKind::SystemRestoration,
        supersedes_event_id: None,
        withdraws_event_id: Some(event.event_id.clone()),
        legacy_record_scope: event.legacy_record_scope.clone(),
        merge_provenance: None,
    }
}

pub(crate) fn apply_canon_policy_input_in_tx(
    conn: &rusqlite::Connection,
    input: RecordCanonPolicyInputV1,
    now_micros: i64,
) -> Result<RecordCanonPolicyOutputV1> {
    let mut evaluations = Vec::with_capacity(input.requests.len());
    let mut claims = Vec::with_capacity(input.requests.len());
    let mut decisions = Vec::with_capacity(input.requests.len());
    for (index, request) in input.requests.iter().enumerate() {
        // Only the first claim is compared against the caller-supplied tip.
        // Later claims in this transaction cannot race an external writer, and
        // a deduplicated earlier claim must not become a stale tip for them.
        let previous_decision_id = if index == 0 {
            input.previous_decision_id.clone()
        } else {
            None
        };
        let secret_linked = request.secret_linked
            || match &request.claim.proposed.assertion {
                ClaimAssertion::Knowledge { proposition, .. } => input
                    .context
                    .secret_linked_propositions
                    .contains(&entity_key(proposition)),
                _ => false,
            };
        let evaluation = evaluate_canon_policy(
            &request.claim,
            CanonPolicyProvenanceV1::from_provenance(request.provenance),
            secret_linked,
            &input.context,
        )?;
        let record = upsert_canon_claim_in_conn(
            conn,
            &input.project_id,
            &input.branch_id,
            &request.claim,
            evaluation.disposition.claim_status(),
            evaluation.tier,
            request.provenance,
            now_micros,
        )?;
        let events = match evaluation.disposition {
            CanonPolicyDispositionV1::StageProvisional
            | CanonPolicyDispositionV1::AutoRatify
            | CanonPolicyDispositionV1::Ratify => {
                vec![policy_event_draft(
                    &request.claim,
                    request.provenance,
                    request.supersedes_event_id.clone(),
                )]
            }
            CanonPolicyDispositionV1::Reject
            | CanonPolicyDispositionV1::Supersede
            | CanonPolicyDispositionV1::Withdraw
            | CanonPolicyDispositionV1::InvalidateSource => Vec::new(),
        };
        let decision_input = AppendCanonDecisionInputV1 {
            project_id: input.project_id.clone(),
            branch_id: input.branch_id.clone(),
            resolve_active_descendant_branch: false,
            action: evaluation.action,
            tier: evaluation.tier,
            actor: evaluation.actor.clone(),
            policy_version: evaluation.policy_version.clone(),
            source_revision_id: request.claim.binding.revision_id.clone(),
            target_scene_id: input.target_scene_id.clone(),
            replace_prior_scene_revisions: input.replace_prior_scene_revisions && index == 0,
            previous_decision_id: previous_decision_id.clone(),
            confidence_inputs: evaluation.confidence_inputs.clone(),
            reason: evaluation.reason.clone(),
            events,
        };
        let output = append_canon_policy_decision_in_tx(
            conn,
            decision_input,
            CanonDecisionAuditV1::from(&evaluation),
            now_micros,
        )?;
        evaluations.push(evaluation);
        claims.push(record);
        decisions.push(output);
    }
    Ok(RecordCanonPolicyOutputV1 {
        evaluations,
        claims,
        decisions,
    })
}

/// Build a legacy/operator-declared claim for the dual-write bridge. It has no
/// prose evidence (legacy paths do not preserve one), so only the explicit
/// operator path can ratify it; high-impact classes still stage.
#[allow(clippy::too_many_arguments)]
pub(crate) fn legacy_claim(
    project_id: &str,
    branch_id: &str,
    source_revision_id: &str,
    scene_id: Option<&str>,
    placement: &spindle_core::models::StoryPlacement,
    subject: ClaimEntity,
    assertion: ClaimAssertion,
    within_scene_order: Option<u32>,
    index: usize,
) -> ClaimV1 {
    ClaimV1 {
        binding: ClaimBindingV1 {
            schema_version: CLAIM_SCHEMA_VERSION,
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            scene_id: scene_id.map(str::to_owned),
            revision_id: source_revision_id.to_string(),
            accepted_text_sha256: exact_text_sha256(source_revision_id),
            position: placement.clone(),
            extraction_run_id: format!("legacy-dual-write:{source_revision_id}"),
            model: "legacy-write".into(),
            agent_id: None,
            model_call_id: None,
            prompt_version: "legacy-write-v1".into(),
        },
        source: ClaimSource::Drafter,
        proposed: ProposedClaimV1 {
            claim_id: format!("legacy:{source_revision_id}:{index}"),
            subject,
            assertion,
            within_scene_order,
            in_world_time: None,
            modality: ClaimModality::Narration,
            temporal_scope: TemporalScope::Current,
            speaker: None,
            experiencer: None,
            confidence: Some(1.0),
            evidence: Vec::new(),
        },
    }
}

/// Build a canonical compatibility-snapshot attribute for a legacy row. The
/// projection reads these exact JSON records, so every mirrored store must emit
/// one in addition to its typed assertions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn legacy_snapshot_claim<T: serde::Serialize>(
    project_id: &str,
    branch_id: &str,
    source_revision_id: &str,
    scene_id: Option<&str>,
    placement: &spindle_core::models::StoryPlacement,
    subject_id: &str,
    attribute_key: String,
    record: &T,
    within_scene_order: Option<u32>,
    index: usize,
) -> Result<ClaimV1> {
    Ok(legacy_claim(
        project_id,
        branch_id,
        source_revision_id,
        scene_id,
        placement,
        ClaimEntity::Resolved {
            id: subject_id.to_string(),
        },
        ClaimAssertion::Attribute {
            operation: AttributeOperation::Set,
            key: attribute_key,
            value: ClaimScalar::Text {
                value: serde_json::to_string(record)?,
            },
            nature: AttributeNature::Evolving,
        },
        within_scene_order,
        index,
    ))
}

/// Mirror authoritative legacy writes into canon inside the same transaction.
/// Enabled only in dual-write/shadow-read/canon-read modes. Ordinary writes are
/// authored (`ManualAuthor`) and may ratify; v1 high-impact classes stay
/// provisional. Returns `None` when cutover has not started.
pub(crate) fn mirror_legacy_claims_in_tx(
    conn: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
    source_revision_id: &str,
    target_scene_id: Option<&str>,
    claims: Vec<ClaimV1>,
) -> Result<Option<RecordCanonPolicyOutputV1>> {
    if !canon_cutover_mode_in_conn(conn, project_id)?.mirrors_writes() {
        return Ok(None);
    }
    if claims.is_empty() {
        return Ok(None);
    }
    let lineage = resolve_branch_lineage_in_conn(conn, Some(project_id), branch_id)?;
    let events = load_events_for_branches_in_conn(conn, project_id, &lineage)?;
    let ratified = state_at(
        branch_id,
        &lineage,
        &events,
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )?;
    let existing_promise_thread_ids = ratified.open_threads.keys().cloned().collect();
    let context = CanonPolicyContextV1 {
        accepted_revisions: BTreeMap::new(),
        ratified_state: Some(ratified),
        package_agreed_fingerprints: BTreeSet::new(),
        secret_linked_propositions: BTreeSet::new(),
        existing_promise_thread_ids,
        calibrations: Vec::new(),
        config: CanonPolicyConfigV1::default(),
    };
    let mut requests = Vec::with_capacity(claims.len());
    for mut claim in claims {
        let stream_key = claim_semantic_stream_key(&claim);
        let new_payload = CanonEventPayloadV1::Assertion {
            assertion: claim.proposed.assertion.clone(),
            modality: claim.proposed.modality,
            temporal_scope: claim.proposed.temporal_scope,
            speaker: claim.proposed.speaker.clone(),
            experiencer: claim.proposed.experiencer.clone(),
        };
        let new_fingerprint = semantic_event_fingerprint(
            &claim.proposed.subject,
            &new_payload,
            story_placement_to_canon_position(&claim.binding.position),
            claim.proposed.within_scene_order,
            &claim.proposed.in_world_time,
        );
        let is_scene_delta = matches!(
            claim.proposed.assertion,
            ClaimAssertion::Relationship {
                operation: RelationshipOperation::Delta,
                ..
            }
        );
        let prior = stream_key.as_deref().and_then(|stream_key| {
            events
                .iter()
                .filter(|event| {
                    let provenance_matches = event.provenance == CanonProvenanceKind::ManualAuthor
                        || (is_scene_delta
                            && event.provenance == CanonProvenanceKind::LegacyUnverified);
                    // The backfilled opening Observation establishes the
                    // balance; only a backfilled scene Delta is replaceable.
                    let stream_shape_matches = !is_scene_delta
                        || matches!(
                            event.payload,
                            CanonEventPayloadV1::Assertion {
                                assertion: ClaimAssertion::Relationship {
                                    operation: RelationshipOperation::Delta,
                                    ..
                                },
                                ..
                            }
                        );
                    provenance_matches
                        && stream_shape_matches
                        && event.scene_id == claim.binding.scene_id
                        && event_semantic_stream_key(event).as_deref() == Some(stream_key)
                })
                .max_by_key(|event| event.append_seq)
        });
        let mut supersedes_event_id = None;
        if let Some(prior) = prior {
            // An exact retry is already reflected in canon; skip it. A changed
            // (or reverted) value must not deduplicate against an older
            // decision, so bind this mirror to the event it replaces.
            if prior.semantic_fingerprint == new_fingerprint {
                continue;
            }
            claim.binding.revision_id = format!(
                "{}#supersedes:{}",
                claim.binding.revision_id, prior.event_id
            );
            claim.binding.accepted_text_sha256 = exact_text_sha256(&claim.binding.revision_id);
            supersedes_event_id = Some(prior.event_id.clone());
        }
        requests.push(CanonPolicyClaimRequestV1 {
            claim,
            provenance: CanonProvenanceKind::ManualAuthor,
            secret_linked: false,
            supersedes_event_id,
        });
    }
    if requests.is_empty() {
        return Ok(None);
    }
    let now_micros = timestamp_to_micros(chrono::Utc::now());
    apply_canon_policy_input_in_tx(
        conn,
        RecordCanonPolicyInputV1 {
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            source_revision_id: source_revision_id.to_string(),
            target_scene_id: target_scene_id.map(str::to_owned),
            previous_decision_id: None,
            replace_prior_scene_revisions: false,
            requests,
            context,
        },
        now_micros,
    )
    .map(Some)
}

impl CanonTransaction<'_> {
    pub fn canon_cutover_mode(&self, project_id: &str) -> Result<CanonCutoverModeV1> {
        canon_cutover_mode_in_conn(&self.tx, project_id)
    }

    pub fn mirror_canon_claims(
        &self,
        project_id: &str,
        branch_id: &str,
        source_revision_id: &str,
        target_scene_id: Option<&str>,
        claims: Vec<ClaimV1>,
    ) -> Result<Option<RecordCanonPolicyOutputV1>> {
        mirror_legacy_claims_in_tx(
            &self.tx,
            project_id,
            branch_id,
            source_revision_id,
            target_scene_id,
            claims,
        )
    }

    /// Mark the just-mirrored legacy rows as mapped with their exact snapshots so
    /// a later backfill rerun skips them instead of duplicating events.
    pub fn mark_canon_source_rows_mapped(
        &self,
        project_id: &str,
        branch_id: &str,
        keys: Vec<(String, String)>,
    ) -> Result<()> {
        mark_legacy_rows_mapped_in_conn(
            &self.tx,
            project_id,
            branch_id,
            &keys,
            timestamp_to_micros(chrono::Utc::now()),
        )
    }
}

impl Repository {
    /// Evaluate claims with the pure policy and persist the decision, events and
    /// claim status atomically. A stale `previous_decision_id` fails the whole
    /// batch; retries of the same batch deduplicate instead of accumulating.
    pub async fn record_canon_policy_decisions(
        &self,
        input: RecordCanonPolicyInputV1,
    ) -> Result<RecordCanonPolicyOutputV1> {
        self.with_transaction(move |tx| {
            let now_micros = timestamp_to_micros(chrono::Utc::now());
            apply_canon_policy_input_in_tx(&tx.tx, input, now_micros)
        })
        .await
    }

    /// Revoke a decision batch: append `Withdraw` events for every target and
    /// dependent event, invalidate affected claim rows, and return the derived
    /// summary keys that must be recomputed by later memory layers. Original
    /// events and audit decisions are never deleted or rewritten.
    pub async fn revoke_canon_decision_batch(
        &self,
        input: RevokeCanonDecisionInputV1,
    ) -> Result<RevokeCanonDecisionOutputV1> {
        self.with_transaction(move |tx| {
            let now_micros = timestamp_to_micros(chrono::Utc::now());
            let target_decision = load_decision_in_conn(&tx.tx, &input.decision_id)?;
            let target_events = load_events_by_decision_in_conn(&tx.tx, &input.decision_id)?;
            anyhow::ensure!(
                !target_events.is_empty(),
                "canon decision has no events to revoke"
            );
            let lineage =
                resolve_branch_lineage_in_conn(&tx.tx, Some(&input.project_id), &input.branch_id)?;
            let all_events = load_events_for_branches_in_conn(&tx.tx, &input.project_id, &lineage)?;
            let claims =
                load_canon_claims_for_branch_in_conn(&tx.tx, &input.project_id, &input.branch_id)?;
            let mut plan = plan_canon_batch_revocation(
                &target_events,
                &all_events,
                &claims,
                &input.branch_id,
                &target_decision.source_revision_id,
            );
            let dependent: Vec<CanonEventV1> = all_events
                .iter()
                .filter(|event| {
                    plan.invalidated_dependent_event_ids
                        .contains(&event.event_id)
                })
                .cloned()
                .collect();
            for event in &dependent {
                plan.withdraw_event_drafts.push(withdraw_draft(event));
            }
            let audit = CanonDecisionAuditV1 {
                policy_version: CANON_POLICY_VERSION_V1.to_string(),
                semantic_fingerprint: plan.semantic_fingerprint.clone(),
                idempotency_key: compute_idempotency_key(
                    &input.branch_id,
                    &target_decision.source_revision_id,
                    &plan.semantic_fingerprint,
                    CanonPolicyAction::Withdraw,
                ),
                confidence_inputs: vec![1.0],
                precondition_summary: format!(
                    "revoked_decision={};withdraw_events={};dependent_events={}",
                    input.decision_id,
                    target_events.len(),
                    dependent.len()
                ),
            };
            let decision = append_canon_policy_decision_in_tx(
                &tx.tx,
                AppendCanonDecisionInputV1 {
                    project_id: input.project_id.clone(),
                    branch_id: input.branch_id.clone(),
                    resolve_active_descendant_branch: false,
                    action: CanonPolicyAction::Withdraw,
                    tier: CanonTier::Ratified,
                    actor: input.actor.clone(),
                    policy_version: CANON_POLICY_VERSION_V1.to_string(),
                    source_revision_id: target_decision.source_revision_id.clone(),
                    target_scene_id: target_decision.target_scene_id.clone(),
                    replace_prior_scene_revisions: false,
                    previous_decision_id: None,
                    confidence_inputs: vec![1.0],
                    reason: input.reason.clone(),
                    events: plan.withdraw_event_drafts.clone(),
                },
                audit,
                now_micros,
            )?;
            // Invalidate the audited claim rows; dependency on the same claims
            // is captured by the plan's dependent event ids.
            let mut invalidated_claim_ids = plan.invalidated_claim_ids.clone();
            for claim in &claims {
                if invalidated_claim_ids.contains(&claim.claim_record_id) {
                    continue;
                }
                let shares_target_claim = target_events.iter().any(|event| {
                    event
                        .source_claim_ids
                        .iter()
                        .any(|id| id == &claim.claim.proposed.claim_id)
                });
                let shares_dependent_claim = dependent.iter().any(|event| {
                    event
                        .source_claim_ids
                        .iter()
                        .any(|id| id == &claim.claim.proposed.claim_id)
                });
                if shares_target_claim || shares_dependent_claim {
                    invalidated_claim_ids.push(claim.claim_record_id.clone());
                }
            }
            let status_str = enum_to_sql_str(CanonClaimStatus::Withdrawn)?;
            for claim_id in &invalidated_claim_ids {
                if let Some(claim) = claims
                    .iter()
                    .find(|claim| &claim.claim_record_id == claim_id)
                {
                    let mut updated = claim.clone();
                    updated.status = CanonClaimStatus::Withdrawn;
                    tx.tx.execute(
                        "UPDATE canon_claim SET status = ?2, data = ?3 WHERE id = ?1",
                        rusqlite::params![
                            claim_id,
                            &status_str,
                            &serde_json::to_string(&updated)?,
                        ],
                    )?;
                }
            }
            Ok(RevokeCanonDecisionOutputV1 {
                invalidated_claim_ids,
                invalidated_event_ids: plan
                    .withdraw_event_drafts
                    .iter()
                    .filter_map(|draft| draft.withdraws_event_id.clone())
                    .collect(),
                invalidated_summary_keys: plan.invalidated_summary_keys.clone(),
                plan,
                decision,
            })
        })
        .await
    }
}
