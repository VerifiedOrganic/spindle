use super::*;
use crate::sqlite::repository::resolve_planning_operation;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

fn digest(value: &impl serde::Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn references(receipts: &[PlanningOperationReceipt]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for r in receipts {
        if r.outcome == PlanningOperationOutcome::Applied {
            out.extend(r.returned_ids.clone());
        }
    }
    out
}
impl SqliteSpindleService {
    pub async fn planning_apply_status(
        &self,
        id: &str,
        scope: Option<PlanningScope>,
    ) -> Result<PlanningApplyReport> {
        let proposals = self.list_planning_proposals(id, scope.clone()).await?;
        let receipts = self.repository.planning_receipts(id).await?;
        let prior = self.repository.latest_planning_attempt(id).await?;
        let mut operations = Vec::new();
        for p in &proposals {
            for index in 0..p.artifact.operations.len() {
                operations.push(
                    receipts
                        .iter()
                        .find(|r| r.proposal_id == p.id && r.operation_index == index)
                        .cloned()
                        .unwrap_or(PlanningOperationReceipt {
                            proposal_id: p.id.clone(),
                            operation_index: index,
                            input_hash: String::new(),
                            returned_ids: BTreeMap::new(),
                            actor: String::new(),
                            decision_note: String::new(),
                            result: None,
                            prior_state: Value::Null,
                            outcome: PlanningOperationOutcome::NotReached,
                            error: None,
                            recorded_at: 0,
                        }),
                );
            }
        }
        let complete = !proposals.is_empty()
            && proposals
                .iter()
                .all(|p| p.status == PlanningProposalStatus::Applied);
        Ok(PlanningApplyReport {
            attempt_id: prior
                .as_ref()
                .map(|r| r.attempt_id.clone())
                .unwrap_or_default(),
            complete,
            proposals,
            operations,
            error: prior.and_then(|r| r.error),
            pending_search_refreshes: self.repository.planning_embedding_count(scope).await?,
        })
    }

    async fn selected_planning(
        &self,
        input: &DecidePlanningInput,
    ) -> Result<Vec<PlanningProposal>> {
        anyhow::ensure!(
            input.decision == PlanningDecision::Accept,
            "apply requires accept decision"
        );
        anyhow::ensure!(!input.expected_hashes.is_empty(), "empty planning decision");
        anyhow::ensure!(
            !input.actor.trim().is_empty() && input.actor.len() <= 256,
            "human or delegated policy decision actor required"
        );
        anyhow::ensure!(
            !input.note.trim().is_empty() && input.note.len() <= 8000,
            "decision note required (max 8000 bytes)"
        );
        let s = self.get_planning_session(&input.session_id).await?;
        anyhow::ensure!(s.scope == input.scope, "planning scope mismatch");
        let all = self.list_planning_proposals(&s.id, s.scope.clone()).await?;
        let is_policy_actor = input.actor.trim_start().starts_with("policy:");
        if is_policy_actor {
            anyhow::ensure!(
                s.scope.is_some(),
                "human decision actor required before project root attachment"
            );
            anyhow::ensure!(
                !s.projection_stale,
                "human decision actor required while planning projection is stale"
            );
            let unaccepted_root = all.iter().any(|p| {
                p.status == PlanningProposalStatus::Staged
                    && matches!(
                        p.artifact.level,
                        PlanningLevel::Contract
                            | PlanningLevel::CastWorld
                            | PlanningLevel::SeriesArc
                            | PlanningLevel::Book
                    )
            });
            anyhow::ensure!(
                !unaccepted_root,
                "human decision actor required while upstream root proposals remain unaccepted"
            );
        }
        let mut pending = Vec::new();
        for (id, expected) in &input.expected_hashes {
            let p = all
                .iter()
                .find(|p| &p.id == id)
                .context("proposal not in planning session")?;
            anyhow::ensure!(
                &p.payload_hash == expected && digest(&p.artifact)? == *expected,
                "stale planning proposal hash"
            );
            anyhow::ensure!(
                matches!(
                    p.status,
                    PlanningProposalStatus::Staged | PlanningProposalStatus::Applied
                ),
                "planning proposal is no longer active"
            );
            anyhow::ensure!(
                p.artifact.intent_hash == s.intent_hash,
                "stale planning intent"
            );
            anyhow::ensure!(
                !p.findings
                    .iter()
                    .any(|f| f.severity == PlanningFindingSeverity::Blocking),
                "cannot accept planning proposal with blocking validation findings"
            );
            let effective_gate = s
                .policy
                .gates
                .get(&p.artifact.level)
                .copied()
                .unwrap_or_default();
            if is_policy_actor {
                anyhow::ensure!(
                    matches!(
                        p.artifact.level,
                        PlanningLevel::Chapter | PlanningLevel::Scene
                    ),
                    "human decision actor required for root planning levels"
                );
                anyhow::ensure!(
                    effective_gate == PlanningGate::DelegateIfValid,
                    "human decision actor required when delegate_if_valid is not active"
                );
                anyhow::ensure!(
                    s.policy.delegated_actor().as_deref() == Some(input.actor.trim()),
                    "policy actor '{}' does not match active session policy ({:?})",
                    input.actor,
                    s.policy.delegated_actor()
                );
                anyhow::ensure!(
                    p.artifact.author_questions.is_empty(),
                    "delegated policy cannot accept proposal with unresolved author_questions"
                );
                let adv_count = p
                    .findings
                    .iter()
                    .filter(|f| f.severity == PlanningFindingSeverity::Advisory)
                    .count();
                anyhow::ensure!(
                    s.policy
                        .max_advisory_findings
                        .is_none_or(|max_adv| adv_count <= max_adv),
                    "delegated policy cannot accept proposal exceeding max_advisory_findings"
                );
            } else {
                anyhow::ensure!(
                    !matches!(
                        p.artifact.level,
                        PlanningLevel::Contract
                            | PlanningLevel::CastWorld
                            | PlanningLevel::SeriesArc
                            | PlanningLevel::Book
                    ) || effective_gate != PlanningGate::DelegateIfValid,
                    "root planning levels cannot use delegate_if_valid"
                );
            }
            if p.artifact.level == PlanningLevel::Contract {
                anyhow::ensure!(
                    p.artifact.operations.len() == 1,
                    "contract acceptance requires one root operation"
                );
            }
            for (parent_id, parent_hash) in &p.artifact.parents {
                let parent = all
                    .iter()
                    .find(|p| &p.id == parent_id)
                    .context("missing planning parent")?;
                anyhow::ensure!(
                    &parent.payload_hash == parent_hash
                        && matches!(
                            parent.status,
                            PlanningProposalStatus::Staged | PlanningProposalStatus::Applied
                        ),
                    "stale planning parent"
                );
                anyhow::ensure!(
                    parent.status == PlanningProposalStatus::Applied
                        || input.expected_hashes.get(parent_id) == Some(parent_hash),
                    "dependency closure includes an unapproved parent; select it explicitly"
                );
            }
            pending.push(p.clone());
        }
        let mut ordered = Vec::new();
        let mut done: BTreeSet<String> = all
            .iter()
            .filter(|p| p.status == PlanningProposalStatus::Applied)
            .map(|p| p.id.clone())
            .collect();
        while !pending.is_empty() {
            let index = pending
                .iter()
                .position(|p| p.artifact.parents.keys().all(|id| done.contains(id)))
                .context("planning dependency cycle")?;
            let p = pending.remove(index);
            done.insert(p.id.clone());
            ordered.push(p);
        }
        Ok(ordered)
    }

    pub async fn apply_planning(&self, input: DecidePlanningInput) -> Result<PlanningApplyReport> {
        let pool = self.repository.pool().clone();
        // Execute the same service paths in a rollback-only transaction. This
        // validates the full selected closure without leaving domain writes behind.
        let ordered = pool
            .application_transaction(false, || async {
                let ordered = self.selected_planning(&input).await?;
                for p in &ordered {
                    for index in 0..p.artifact.operations.len() {
                        self.apply_planning_operation(p, index, &input.actor, &input.note)
                            .await?;
                    }
                    self.repository
                        .finish_planning_proposal(
                            p.id.clone(),
                            input.actor.clone(),
                            input.note.clone(),
                        )
                        .await?;
                }
                Ok(ordered)
            })
            .await?;
        let attempt_id = format!("planning_apply_attempt:{}", ulid::Ulid::new());
        let selected: BTreeSet<String> = ordered.iter().map(|p| p.id.clone()).collect();
        let mut report = self
            .planning_report_for(&input.session_id, &selected, &attempt_id)
            .await?;
        self.repository
            .store_planning_attempt(
                input.session_id.clone(),
                input.actor.clone(),
                report.clone(),
            )
            .await?;
        let mut expected_scope = input.scope.clone();
        for p in &ordered {
            for index in 0..p.artifact.operations.len() {
                let result = pool
                    .application_transaction(true, || async {
                        let s = self.get_planning_session(&input.session_id).await?;
                        anyhow::ensure!(
                            s.scope == expected_scope,
                            "planning session scope changed during apply"
                        );
                        let current = self.list_planning_proposals(&s.id, s.scope.clone()).await?;
                        let current = current
                            .iter()
                            .find(|item| item.id == p.id)
                            .context("proposal disappeared during apply")?;
                        anyhow::ensure!(
                            current.payload_hash == p.payload_hash
                                && matches!(
                                    current.status,
                                    PlanningProposalStatus::Staged
                                        | PlanningProposalStatus::Applied
                                ),
                            "proposal changed during apply"
                        );
                        self.apply_planning_operation(current, index, &input.actor, &input.note)
                            .await
                    })
                    .await;
                if let Err(error) = result {
                    let current_session = self.get_planning_session(&input.session_id).await?;
                    let known =
                        references(&self.repository.planning_receipts(&input.session_id).await?);
                    let failed_input_hash = match resolve_planning_operation(
                        &p.artifact.operations[index],
                        &current_session.scope,
                        &known,
                    ) {
                        Ok(d) => digest(&d)?,
                        Err(_) => String::new(),
                    };
                    let receipt = PlanningOperationReceipt {
                        proposal_id: p.id.clone(),
                        operation_index: index,
                        input_hash: failed_input_hash,
                        returned_ids: BTreeMap::new(),
                        actor: input.actor.clone(),
                        decision_note: input.note.clone(),
                        result: None,
                        prior_state: Value::Null,
                        outcome: PlanningOperationOutcome::Failed,
                        error: Some(error.to_string()),
                        recorded_at: chrono::Utc::now().timestamp_micros(),
                    };
                    report = pool
                        .application_transaction(true, || async {
                            self.repository
                                .store_planning_receipt(receipt, None, BTreeMap::new())
                                .await?;
                            let mut report = self
                                .planning_report_for(&input.session_id, &selected, &attempt_id)
                                .await?;
                            report.error = Some(error.to_string());
                            self.repository
                                .store_planning_attempt(
                                    input.session_id.clone(),
                                    input.actor.clone(),
                                    report.clone(),
                                )
                                .await?;
                            Ok(report)
                        })
                        .await?;
                    return Ok(report);
                }
                expected_scope = self.get_planning_session(&input.session_id).await?.scope;
            }
            self.repository
                .finish_planning_proposal(p.id.clone(), input.actor.clone(), input.note.clone())
                .await?;
            report = self
                .planning_report_for(&input.session_id, &selected, &attempt_id)
                .await?;
            self.repository
                .store_planning_attempt(
                    input.session_id.clone(),
                    input.actor.clone(),
                    report.clone(),
                )
                .await?;
        }
        if let Some(scope) = &expected_scope {
            report.pending_search_refreshes = self
                .repository
                .flush_planning_embeddings(&scope.project_id)
                .await?;
        }
        report.error = None;
        self.repository
            .store_planning_attempt(input.session_id, input.actor, report.clone())
            .await?;
        Ok(report)
    }

    async fn planning_report_for(
        &self,
        id: &str,
        selected: &BTreeSet<String>,
        attempt: &str,
    ) -> Result<PlanningApplyReport> {
        let s = self.get_planning_session(id).await?;
        let mut report = self.planning_apply_status(id, s.scope).await?;
        report.proposals.retain(|p| selected.contains(&p.id));
        report
            .operations
            .retain(|r| selected.contains(&r.proposal_id));
        report.attempt_id = attempt.to_string();
        report.complete = report
            .proposals
            .iter()
            .all(|p| p.status == PlanningProposalStatus::Applied)
            && report
                .operations
                .iter()
                .all(|r| r.outcome == PlanningOperationOutcome::Applied);
        report.error = None;
        Ok(report)
    }

    async fn apply_planning_operation(
        &self,
        p: &PlanningProposal,
        index: usize,
        actor: &str,
        note: &str,
    ) -> Result<PlanningOperationReceipt> {
        let s = self.get_planning_session(&p.artifact.session_id).await?;
        let proposals = self.list_planning_proposals(&s.id, s.scope.clone()).await?;
        let fresh = proposals
            .iter()
            .find(|candidate| candidate.id == p.id)
            .context("missing current planning proposal")?;
        anyhow::ensure!(
            fresh.payload_hash == p.payload_hash,
            "planning proposal changed during operation"
        );
        let p = fresh;
        let receipts = self.repository.planning_receipts(&s.id).await?;
        let refs = references(&receipts);
        let item = &p.artifact.operations[index];
        let descriptor = resolve_planning_operation(item, &s.scope, &refs)?;
        let input_hash = digest(&descriptor)?;
        if let Some(prior) = receipts.iter().find(|r| {
            r.proposal_id == p.id
                && r.operation_index == index
                && r.outcome == PlanningOperationOutcome::Applied
        }) {
            anyhow::ensure!(
                prior.input_hash == input_hash,
                "planning receipt input hash mismatch"
            );
            return Ok(prior.clone());
        }
        anyhow::ensure!(
            p.status == PlanningProposalStatus::Staged,
            "applied proposal has a missing receipt"
        );
        self.repository
            .planning_scope_guard(
                s.scope.clone(),
                descriptor.1.clone(),
                p.artifact.level,
                matches!(
                    &item.operation,
                    PlanningOperation::UpdateReaderContract(_) | PlanningOperation::CreateBook(_)
                ),
            )
            .await?;
        let state = self
            .repository
            .planning_operation_state(s.scope.clone(), item.clone(), index, refs)
            .await?;
        for (key, value) in &state {
            let expected = p.base_state.get(key);
            let current = digest(value)?;
            anyhow::ensure!(
                expected == Some(&current)
                    || (expected.is_none() && value.as_array().is_some_and(Vec::is_empty)),
                "stale planning base state at {key}; stage a fresh synchronization proposal"
            );
        }
        let before_followups = self
            .repository
            .planning_followup_states(p.id.clone())
            .await?;
        let (output, ids, root) = self
            .dispatch_planning_operation(item, &s.scope, descriptor.1)
            .await?;
        let receipt = PlanningOperationReceipt {
            proposal_id: p.id.clone(),
            operation_index: index,
            input_hash,
            returned_ids: ids,
            actor: actor.into(),
            decision_note: note.into(),
            result: Some(output),
            prior_state: serde_json::to_value(state)?,
            outcome: PlanningOperationOutcome::Applied,
            error: None,
            recorded_at: chrono::Utc::now().timestamp_micros(),
        };
        self.repository
            .store_planning_receipt(receipt.clone(), root, before_followups)
            .await?;
        Ok(receipt)
    }

    async fn dispatch_planning_operation(
        &self,
        item: &PlanningOperationItem,
        scope: &Option<PlanningScope>,
        input: Value,
    ) -> Result<(
        PlanningAppliedOutput,
        BTreeMap<String, String>,
        Option<CreateProjectOutput>,
    )> {
        use PlanningOperation::*;
        let mut ids = BTreeMap::new();
        let mut root = None;
        macro_rules! invoke {
            ($method:ident,$input:ty,$variant:ident,$field:ident) => {{
                let out = self
                    .$method(serde_json::from_value::<$input>(input)?)
                    .await?;
                ids.insert(item.output_ref.clone(), out.$field.clone());
                PlanningAppliedOutput::$variant(Box::new(out))
            }};
        }
        let output = match &item.operation {
            CreateProject(_) => {
                anyhow::ensure!(scope.is_none(), "cannot replace an accepted planning root");
                let value: CreateProjectInput = serde_json::from_value(input)?;
                anyhow::ensure!(
                    !value.name.trim().is_empty()
                        && !value.reader_contract.promise.trim().is_empty(),
                    "project name and reader promise must be nonempty"
                );
                let out = self.create_project(value).await?;
                ids.insert(item.output_ref.clone(), out.project_id.clone());
                ids.insert(format!("{}:book", item.output_ref), out.book_id.clone());
                ids.insert(
                    format!("{}:chapter", item.output_ref),
                    out.chapter_id.clone(),
                );
                ids.insert(format!("{}:branch", item.output_ref), out.branch_id.clone());
                root = Some(out.clone());
                PlanningAppliedOutput::CreateProject(Box::new(out))
            }
            CreateCharacter(_) => invoke!(
                create_character,
                CreateCharacterInput,
                CreateCharacter,
                character_id
            ),
            CreateLocation(_) => invoke!(
                create_location,
                CreateLocationInput,
                CreateLocation,
                location_id
            ),
            CreateWorldRule(_) => invoke!(
                create_world_rule,
                CreateWorldRuleInput,
                CreateWorldRule,
                world_rule_id
            ),
            CreateRelationship(_) => invoke!(
                create_relationship,
                CreateRelationshipInput,
                CreateRelationship,
                relationship_id
            ),
            CreatePlotLine(_) => invoke!(
                create_plot_line,
                CreatePlotLineInput,
                CreatePlotLine,
                plot_line_id
            ),
            CreateConflict(_) => invoke!(
                create_conflict,
                CreateConflictInput,
                CreateConflict,
                conflict_id
            ),
            CreateTheme(_) => invoke!(create_theme, CreateThemeInput, CreateTheme, theme_id),
            CreateMotif(_) => invoke!(create_motif, CreateMotifInput, CreateMotif, motif_id),
            CreateNarrativePromise(_) => invoke!(
                create_narrative_promise,
                CreateNarrativePromiseInput,
                CreateNarrativePromise,
                narrative_promise_id
            ),
            CreateCharacterArc(_) => {
                let out = self
                    .create_character_arc(serde_json::from_value(input)?)
                    .await?;
                ids.insert(item.output_ref.clone(), out.character_arc_id.clone());
                ids.insert(
                    format!("{}:pacing_tracker", item.output_ref),
                    out.pacing_tracker_id.clone(),
                );
                PlanningAppliedOutput::CreateCharacterArc(Box::new(out))
            }
            CreatePacingConfig(_) => invoke!(
                create_pacing_config,
                CreatePacingConfigInput,
                CreatePacingConfig,
                pacing_config_id
            ),
            CreatePacingCurve(_) => invoke!(
                create_pacing_curve,
                CreatePacingCurveInput,
                CreatePacingCurve,
                pacing_curve_id
            ),
            SetArcPacingConstraints(_) => invoke!(
                set_arc_pacing_constraints,
                SetArcPacingConstraintsInput,
                SetArcPacingConstraints,
                pacing_tracker_id
            ),
            CreateBook(_) => invoke!(create_book, CreateBookInput, CreateBook, book_id),
            CreateChapter(_) => invoke!(
                create_chapter,
                CreateChapterInput,
                CreateChapter,
                chapter_id
            ),
            PlanChapter(_) => {
                let plan_input: PlanChapterInput = serde_json::from_value(input)?;
                let book_number = plan_input.book_number;
                let chapter_number = plan_input.chapter_number;
                let prior_rev = if let Some(scope) = scope {
                    let project_id = scope.project_id.clone();
                    let branch_id = scope.branch_id.clone();
                    self.repository
                        .pool()
                        .read(move |conn| {
                            let drafted: bool = conn.query_row(
                                "WITH RECURSIVE lineage(id,parent_branch_id) AS (SELECT id,parent_branch_id FROM bible_branch WHERE id=?1 AND project_id=?2 UNION SELECT b.id,b.parent_branch_id FROM bible_branch b JOIN lineage l ON b.id=l.parent_branch_id WHERE b.project_id=?2) SELECT EXISTS(SELECT 1 FROM scene WHERE project_id=?2 AND branch_id IN (SELECT id FROM lineage) AND book_number=?3 AND chapter_number=?4)",
                                rusqlite::params![branch_id, project_id, book_number, chapter_number],
                                |r| r.get(0),
                            )?;
                            if drafted {
                                return Err(rusqlite::Error::ToSqlConversionFailure(
                                    anyhow::anyhow!(
                                        "planning cannot overwrite a chapter with persisted prose"
                                    )
                                    .into(),
                                ));
                            }
                            let active_run_exists: bool = conn.query_row(
                                "SELECT EXISTS(SELECT 1 FROM authoring_run WHERE project_id=?1 AND active_branch_id=?2 AND book_number=?3 AND start_chapter<=?4 AND end_chapter>=?4 AND status NOT IN ('completed','cancelled','abandoned'))",
                                rusqlite::params![project_id, branch_id, book_number, chapter_number],
                                |r| r.get(0),
                            )?;
                            if !active_run_exists {
                                return Ok(None);
                            }
                            let prev: i64 = conn
                                .query_row(
                                    "SELECT plan_revision FROM chapter_plan WHERE project_id=?1 AND branch_id=?2 AND book_number=?3 AND chapter_number=?4",
                                    rusqlite::params![project_id, branch_id, book_number, chapter_number],
                                    |r| r.get(0),
                                )
                                .unwrap_or(1);
                            Ok(Some(prev))
                        })
                        .await?
                } else {
                    None
                };
                let synopsis = plan_input.synopsis.clone();
                let pov_character_id = plan_input.pov_character_id.clone();
                let reseed_scenes: Vec<(Option<i32>, spindle_core::models::PlanChapterSceneInput)> =
                    plan_input
                        .scenes
                        .iter()
                        .map(|sc| (Some(sc.scene_order), sc.clone()))
                        .collect();
                let out = self.plan_chapter(plan_input).await?;
                ids.insert(item.output_ref.clone(), out.chapter_plan_id.clone());
                if let (Some(scope), Some(prev_rev)) = (scope, prior_rev) {
                    let next_revision = prev_rev.max(1) + 1;
                    self.repository
                        .set_chapter_plan_revision(
                            &scope.project_id,
                            &scope.branch_id,
                            book_number,
                            chapter_number,
                            next_revision,
                        )
                        .await?;
                    self.repository
                        .reseed_authoring_runs_for_chapter_plan(
                            &scope.project_id,
                            &scope.branch_id,
                            book_number,
                            chapter_number,
                            &synopsis,
                            pov_character_id.as_deref(),
                            &reseed_scenes,
                            next_revision,
                        )
                        .await?;
                }
                PlanningAppliedOutput::PlanChapter(Box::new(out))
            }
            SetBookOutline(_) => {
                let out = self
                    .set_book_outline(serde_json::from_value(input)?)
                    .await?;
                let stored = self
                    .repository
                    .get_book_outline(&out.outline.book_id, &out.outline.branch_id)
                    .await?
                    .context("missing accepted book outline")?;
                ids.insert(item.output_ref.clone(), stored.id);
                PlanningAppliedOutput::SetBookOutline(Box::new(out))
            }
            SetChapterOutline(_) => {
                let out = self
                    .set_chapter_outline(serde_json::from_value(input)?)
                    .await?;
                let stored = self
                    .repository
                    .get_chapter_outline(&out.outline.chapter_id, &out.outline.branch_id)
                    .await?
                    .context("missing accepted chapter outline")?;
                ids.insert(item.output_ref.clone(), stored.id);
                PlanningAppliedOutput::SetChapterOutline(Box::new(out))
            }
            UpdateReaderContract(_) => {
                let project = scope.as_ref().context("reader contract requires project")?;
                let out = self
                    .update_entity(UpdateEntityInput {
                        entity_type: "project".into(),
                        entity_id: project.project_id.clone(),
                        changes: json!({"reader_contract":input}),
                        allow_rename: None,
                    })
                    .await?;
                ids.insert(item.output_ref.clone(), project.project_id.clone());
                PlanningAppliedOutput::UpdateReaderContract(Box::new(out))
            }
        };
        Ok((output, ids, root))
    }
}
