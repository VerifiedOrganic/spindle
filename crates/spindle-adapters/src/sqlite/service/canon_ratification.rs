use super::*;

impl SqliteSpindleService {
    // ── Canon-delta ratification (list + decide/apply dispatcher, ADR 0001 D3) ─

    /// List staged/decided canon deltas on a project's active branch (ADR 0001).
    /// `status` and provenance (`scene_id` OR `chapter_range`) compose as AND
    /// filters. `scene_id` and `chapter_range` are mutually exclusive. A chapter
    /// range resolves to the active-branch scenes in those chapters (the same way
    /// `compile_manuscript` resolves a range), then the per-scene queries are
    /// unioned; the result preserves the repository's `(created_at, id)` order.
    pub async fn list_canon_deltas(
        &self,
        input: spindle_core::models::ListCanonDeltasInput,
    ) -> Result<spindle_core::models::ListCanonDeltasOutput> {
        use spindle_core::models::ListCanonDeltasOutput;

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let status = input.status.as_deref();

        if input.scene_id.is_some() && input.chapter_range.is_some() {
            anyhow::bail!(
                "list_canon_deltas: scene_id and chapter_range are mutually exclusive filters"
            );
        }

        let stored = match (&input.scene_id, &input.chapter_range) {
            (Some(scene_id), None) => {
                self.repository
                    .list_canon_deltas(&input.project_id, &active_branch.id, status, Some(scene_id))
                    .await?
            }
            (None, Some(range)) => {
                // Resolve the range to active-branch scene ids in those chapters,
                // then union the per-scene queries. The repository already orders
                // each query by (created_at, id); we re-sort the union to keep the
                // overall result deterministic across scenes.
                let scene_ids = self
                    .canon_delta_scene_ids_for_chapter_range(
                        &input.project_id,
                        &active_branch.id,
                        range,
                    )
                    .await?;
                let mut all = Vec::new();
                for scene_id in &scene_ids {
                    let rows = self
                        .repository
                        .list_canon_deltas(
                            &input.project_id,
                            &active_branch.id,
                            status,
                            Some(scene_id),
                        )
                        .await?;
                    all.extend(rows);
                }
                all.sort_by(|a, b| {
                    a.created_at
                        .cmp(&b.created_at)
                        .then_with(|| a.id.cmp(&b.id))
                });
                all
            }
            (None, None) => {
                self.repository
                    .list_canon_deltas(&input.project_id, &active_branch.id, status, None)
                    .await?
            }
            (Some(_), Some(_)) => unreachable!("guarded above"),
        };

        Ok(ListCanonDeltasOutput {
            deltas: stored.into_iter().map(|d| d.into_core()).collect(),
        })
    }

    /// Resolve an inclusive `ChapterRange` to the active-branch scene ids in
    /// those chapters, in deterministic story order. Mirrors the chapter→scene
    /// resolution `compile_manuscript` performs.
    pub(super) async fn canon_delta_scene_ids_for_chapter_range(
        &self,
        project_id: &str,
        branch_id: &str,
        range: &spindle_core::models::ChapterRange,
    ) -> Result<Vec<String>> {
        let mut scenes = self
            .repository
            .list_scenes_by_project_and_branch(project_id, branch_id)
            .await?;
        scenes.retain(|scene| {
            scene.book_number == range.book_number
                && scene.chapter_number >= range.start
                && scene.chapter_number <= range.end
        });
        scenes.sort_by(|a, b| {
            (a.book_number, a.chapter_number, a.scene_order).cmp(&(
                b.book_number,
                b.chapter_number,
                b.scene_order,
            ))
        });
        Ok(scenes.into_iter().map(|scene| scene.id).collect())
    }

    /// The apply dispatcher (ADR 0001 D3, evolution §3.1). Ratifies a batch of
    /// operator decisions on staged canon deltas:
    ///
    /// 1. **Pre-flight ALL before applying ANY.** Every decision is checked: the
    ///    delta exists, belongs to the project, and is currently `staged`
    ///    (deciding a decided/superseded row is an input error — decisions are
    ///    final). For `apply` decisions, the (edited) payload is deserialized into
    ///    the target tool's input DTO and cheap referential checks run (target
    ///    exists where the class requires one). Any pre-flight failure aborts the
    ///    whole call with **zero writes** — every row stays staged.
    /// 2. **Apply phase (input order).** Each `apply` dispatches to the SAME
    ///    existing write service method the class maps to (never a novel write
    ///    path), then records the decision via
    ///    [`Repository::decide_canon_delta`]. A `reject` records the decision only.
    /// 3. **Mid-apply honesty.** If an apply fails after earlier ones succeeded
    ///    (a real write error despite pre-flight), the dispatcher stops: earlier
    ///    applies stay applied (they are real, recorded canon — nothing is rolled
    ///    back silently), the failing row is reported `failed` with its error and
    ///    stays staged, and remaining rows read `not_reached` and stay staged.
    pub async fn decide_canon_deltas(
        &self,
        input: spindle_core::models::DecideCanonDeltasInput,
    ) -> Result<spindle_core::models::DecideCanonDeltasOutput> {
        self.decide_canon_deltas_with_seam(input, || async {}).await
    }

    /// Implementation of [`Self::decide_canon_deltas`] with an injectable hook
    /// run once, after pre-flight and before the apply phase. The public entry
    /// point passes a no-op; a test can inject a real mutation (e.g. deleting a
    /// row pre-flight validated) to exercise the mid-apply-failure path through
    /// the genuine dispatch code rather than a mock.
    pub(crate) async fn decide_canon_deltas_with_seam<H, Fut>(
        &self,
        input: spindle_core::models::DecideCanonDeltasInput,
        after_preflight: H,
    ) -> Result<spindle_core::models::DecideCanonDeltasOutput>
    where
        H: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        use crate::sqlite::repository::CanonDeltaDecision;
        use spindle_core::models::{CanonDeltaDecisionResult, DecideCanonDeltasOutput};

        self.repository.get_project(&input.project_id).await?;
        let decided_by = input
            .decided_by
            .as_deref()
            .unwrap_or("operator")
            .to_string();

        // ── Pre-flight ALL ───────────────────────────────────────────────────
        // Resolve every referenced row, assert project ownership + staged status,
        // and (for applies) validate the effective payload. Collect offenders so
        // the error lists them all rather than failing on the first.
        struct Preflighted {
            delta_id: String,
            action: Action,
            effective_payload: serde_json::Value,
            edited: bool,
            delta: crate::sqlite::records::StoredCanonDelta,
            note: Option<String>,
        }
        enum Action {
            Apply,
            Reject,
        }

        let mut offenders: Vec<String> = Vec::new();
        let mut preflighted: Vec<Preflighted> = Vec::new();
        for decision in &input.decisions {
            let action = match decision.action.trim().to_ascii_lowercase().as_str() {
                "apply" => Action::Apply,
                "reject" => Action::Reject,
                other => {
                    offenders.push(format!(
                        "decision on '{}' has unknown action '{other}' (expected apply|reject)",
                        decision.delta_id
                    ));
                    continue;
                }
            };
            let delta = match self
                .repository
                .read_canon_delta_public(&decision.delta_id)
                .await?
            {
                Some(delta) => delta,
                None => {
                    offenders.push(format!("delta '{}' does not exist", decision.delta_id));
                    continue;
                }
            };
            if delta.project_id != input.project_id {
                offenders.push(format!(
                    "delta '{}' does not belong to project '{}'",
                    decision.delta_id, input.project_id
                ));
                continue;
            }
            if delta.status != "staged" {
                offenders.push(format!(
                    "delta '{}' is '{}', not staged (decisions are final)",
                    decision.delta_id, delta.status
                ));
                continue;
            }
            let edited = decision.edit.is_some();
            let effective_payload = decision
                .edit
                .clone()
                .unwrap_or_else(|| delta.payload.clone());
            // Box the fan-out futures (pre-flight validates ~14 distinct DTO
            // shapes; apply dispatches to ~13 distinct write-service futures).
            // Un-boxed, the sum inflates the enclosing future — and therefore
            // the MCP `call_tool` future — past a worker thread's stack. One
            // heap indirection per decision keeps every caller's stack flat.
            if matches!(action, Action::Apply)
                && let Err(err) =
                    Box::pin(self.preflight_apply_delta(&delta, &effective_payload)).await
            {
                offenders.push(format!(
                    "delta '{}' failed pre-flight: {err}",
                    decision.delta_id
                ));
                continue;
            }
            preflighted.push(Preflighted {
                delta_id: decision.delta_id.clone(),
                action,
                effective_payload,
                edited,
                delta,
                note: decision.note.clone(),
            });
        }

        if !offenders.is_empty() {
            anyhow::bail!(
                "decide_canon_deltas pre-flight failed ({} offender(s)); zero writes applied: {}",
                offenders.len(),
                offenders.join("; ")
            );
        }

        // Test/instrumentation seam: mutate state between pre-flight and apply.
        after_preflight().await;

        // ── Apply phase (input order) ────────────────────────────────────────
        let mut results: Vec<CanonDeltaDecisionResult> = Vec::new();
        let mut applied_count = 0usize;
        let mut rejected_count = 0usize;
        let mut failed_count = 0usize;
        let mut stopped = false;

        for item in preflighted {
            if stopped {
                results.push(CanonDeltaDecisionResult {
                    delta_id: item.delta_id,
                    outcome: "not_reached".to_string(),
                    error: None,
                    applied_record_id: None,
                    note: item.note,
                });
                continue;
            }
            match item.action {
                Action::Reject => {
                    self.repository
                        .decide_canon_delta(
                            &item.delta_id,
                            CanonDeltaDecision::Rejected,
                            &decided_by,
                            // Record an edited payload only when the operator
                            // actually corrected it; a plain reject keeps the
                            // staged payload intact.
                            item.edited.then(|| item.effective_payload.clone()),
                        )
                        .await?;
                    rejected_count += 1;
                    results.push(CanonDeltaDecisionResult {
                        delta_id: item.delta_id,
                        outcome: "rejected".to_string(),
                        error: None,
                        applied_record_id: None,
                        note: item.note,
                    });
                }
                Action::Apply => {
                    match Box::pin(self.apply_canon_delta(&item.delta, &item.effective_payload))
                        .await
                    {
                        Ok(applied_record_id) => {
                            // Record the decision (with the edited payload so the
                            // row reflects exactly what was applied).
                            self.repository
                                .decide_canon_delta(
                                    &item.delta_id,
                                    CanonDeltaDecision::Applied,
                                    &decided_by,
                                    Some(item.effective_payload.clone()),
                                )
                                .await?;
                            applied_count += 1;
                            results.push(CanonDeltaDecisionResult {
                                delta_id: item.delta_id,
                                outcome: "applied".to_string(),
                                error: None,
                                applied_record_id,
                                note: item.note,
                            });
                        }
                        Err(err) => {
                            // Real write failure after pre-flight: stop honestly.
                            // Earlier applies stay applied; this row stays staged.
                            failed_count += 1;
                            results.push(CanonDeltaDecisionResult {
                                delta_id: item.delta_id,
                                outcome: "failed".to_string(),
                                error: Some(format!("{err:#}")),
                                applied_record_id: None,
                                note: item.note,
                            });
                            stopped = true;
                        }
                    }
                }
            }
        }

        Ok(DecideCanonDeltasOutput {
            results,
            applied_count,
            rejected_count,
            failed_count,
        })
    }

    /// Cheap referential pre-flight for one `apply` decision: deserialize the
    /// effective payload into the class's target DTO shape and confirm any
    /// required target row exists. No writes. Runs for every apply before any
    /// apply so a malformed/dangling delta aborts the batch with zero writes.
    pub(super) async fn preflight_apply_delta(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<()> {
        use spindle_core::models::{
            AnnotateSceneBeatsInput, CommitCharacterStateInput, CommitQuantityStateInput,
            CreateNarrativePromiseInput, RecordKnowledgeInput, UpdateRelationshipInput,
        };

        let normalized = self.normalize_canon_delta_payload(delta, payload).await?;
        let payload = &normalized;
        let scene = &delta.scene_id;
        match delta.delta_class.as_str() {
            "canonical_fact" => {
                self.build_register_fact_input(delta, payload).await?;
            }
            "promise_planted" => {
                let _: CreateNarrativePromiseInput =
                    self.deser_with_provenance(delta, payload).await?;
            }
            "promise_payoff_candidate" | "promise_reinforced" => {
                let promise_id = self.canon_delta_promise_id(delta, payload)?;
                self.repository
                    .get_narrative_promise(&promise_id)
                    .await
                    .with_context(|| format!("narrative_promise '{promise_id}' does not exist"))?;
            }
            "relationship_shift" => {
                let input: UpdateRelationshipInput =
                    self.deser_with_provenance(delta, payload).await?;
                self.repository
                    .get_character(&input.character_a_id)
                    .await
                    .with_context(|| {
                        format!(
                            "relationship character '{}' does not exist",
                            input.character_a_id
                        )
                    })?;
                self.repository
                    .get_character(&input.character_b_id)
                    .await
                    .with_context(|| {
                        format!(
                            "relationship character '{}' does not exist",
                            input.character_b_id
                        )
                    })?;
            }
            "character_state" => {
                let input: CommitCharacterStateInput =
                    self.deser_with_provenance(delta, payload).await?;
                self.repository
                    .get_character(&input.character_id)
                    .await
                    .with_context(|| {
                        format!("character '{}' does not exist", input.character_id)
                    })?;
            }
            "knowledge_learned" => {
                let input: RecordKnowledgeInput =
                    self.deser_with_provenance(delta, payload).await?;
                self.repository
                    .get_character(&input.character_id)
                    .await
                    .with_context(|| {
                        format!("character '{}' does not exist", input.character_id)
                    })?;
                if let Some(fact_id) = input.secret_of_fact_id.as_deref() {
                    let fact = self
                        .repository
                        .get_canonical_fact(fact_id)
                        .await
                        .with_context(|| {
                            format!("secret_of_fact_id '{fact_id}' does not reference a fact")
                        })?;
                    if !fact.secret {
                        anyhow::bail!("secret_of_fact_id '{fact_id}' references a non-secret fact");
                    }
                }
            }
            "beat_annotation" => {
                let _: AnnotateSceneBeatsInput = self.deser_with_provenance(delta, payload).await?;
            }
            "try_fail_cycle" | "consequence_delivered" | "escalation_demonstrated" => {
                let conflict_id = self.canon_delta_conflict_id(delta, payload)?;
                let conflict = self
                    .repository
                    .get_conflict(&conflict_id)
                    .await
                    .with_context(|| format!("conflict '{conflict_id}' does not exist"))?;
                if delta.delta_class == "consequence_delivered" {
                    let index = self.canon_delta_index(payload, "consequence_index")?;
                    if index >= conflict.stated_consequences.len() {
                        anyhow::bail!(
                            "consequence_index {index} out of range ({} consequences)",
                            conflict.stated_consequences.len()
                        );
                    }
                } else if delta.delta_class == "escalation_demonstrated" {
                    let index = self.canon_delta_index(payload, "stage_index")?;
                    if index >= conflict.escalation_stages.len() {
                        anyhow::bail!(
                            "stage_index {index} out of range ({} escalation stages)",
                            conflict.escalation_stages.len()
                        );
                    }
                }
            }
            "arc_milestone_reached" => {
                let arc_id = self.canon_delta_arc_id(delta, payload)?;
                let arc = self
                    .repository
                    .get_character_arc(&arc_id)
                    .await
                    .with_context(|| format!("character_arc '{arc_id}' does not exist"))?;
                let label = self.canon_delta_milestone_label(payload)?;
                if !arc.milestones.iter().any(|m| m.label == label) {
                    anyhow::bail!("arc '{arc_id}' has no milestone labelled '{label}'");
                }
            }
            "quantity_change" => {
                let _: CommitQuantityStateInput =
                    self.deser_with_provenance(delta, payload).await?;
            }
            "entity_candidate" => {
                let kind = payload
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .map(|k| k.trim().to_ascii_lowercase());
                match kind.as_deref() {
                    Some("character") => {
                        let _: spindle_core::models::CreateCharacterInput =
                            self.deser_with_provenance(delta, payload).await?;
                    }
                    Some("location") => {
                        let _: spindle_core::models::CreateLocationInput =
                            self.deser_with_provenance(delta, payload).await?;
                    }
                    Some("term") => {
                        let _: spindle_core::models::CreateTermInput =
                            self.deser_with_provenance(delta, payload).await?;
                    }
                    other => anyhow::bail!(
                        "entity_candidate kind must be character|location|term (got {other:?})"
                    ),
                }
            }
            other => anyhow::bail!("unknown canon delta class '{other}'"),
        }
        let _ = scene;
        Ok(())
    }

    /// Apply one pre-flighted `apply` decision by dispatching to the class's
    /// existing write service method. Returns the fresh record id for classes
    /// that create a row (`canonical_fact`, `promise_planted`, `entity_candidate`),
    /// or `None` for in-place updates. Never a novel write path.
    pub(super) async fn apply_canon_delta(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<Option<String>> {
        use spindle_core::models::{
            AnnotateSceneBeatsInput, CommitCharacterStateInput, CommitQuantityStateInput,
            CreateCharacterInput, CreateLocationInput, CreateNarrativePromiseInput,
            CreateTermInput, RecordKnowledgeInput, UpdatePromiseStatusInput,
            UpdateRelationshipInput,
        };

        let normalized = self.normalize_canon_delta_payload(delta, payload).await?;
        let payload = &normalized;
        match delta.delta_class.as_str() {
            "canonical_fact" => {
                let input = self.build_register_fact_input(delta, payload).await?;
                let out = self.register_canonical_fact(input).await?;
                Ok(Some(out.canonical_fact_id))
            }
            "promise_planted" => {
                let input: CreateNarrativePromiseInput =
                    self.deser_with_provenance(delta, payload).await?;
                let out = self.create_narrative_promise(input).await?;
                Ok(Some(out.narrative_promise_id))
            }
            "promise_payoff_candidate" | "promise_reinforced" => {
                let promise_id = self.canon_delta_promise_id(delta, payload)?;
                let status = self.canon_delta_promise_status(delta, payload)?;
                self.update_promise_status(UpdatePromiseStatusInput {
                    narrative_promise_id: promise_id,
                    status,
                    note: None,
                    source_scene_id: Some(delta.scene_id.clone()),
                    ..Default::default()
                })
                .await?;
                Ok(None)
            }
            "relationship_shift" => {
                let input: UpdateRelationshipInput =
                    self.deser_with_provenance(delta, payload).await?;
                self.update_relationship_with_source(input, "canon_delta")
                    .await?;
                Ok(None)
            }
            "character_state" => {
                let input: CommitCharacterStateInput =
                    self.deser_with_provenance(delta, payload).await?;
                let out = self.commit_character_state(input).await?;
                Ok(Some(out.state_id))
            }
            "knowledge_learned" => {
                let input: RecordKnowledgeInput =
                    self.deser_with_provenance(delta, payload).await?;
                let out = self.record_knowledge(input).await?;
                Ok(Some(out.fact.knowledge_fact_id))
            }
            "beat_annotation" => {
                let input: AnnotateSceneBeatsInput =
                    self.deser_with_provenance(delta, payload).await?;
                let out = self.annotate_scene_beats(input).await?;
                Ok(Some(out.scene_annotation_id))
            }
            "try_fail_cycle" | "consequence_delivered" | "escalation_demonstrated" => {
                self.apply_conflict_delta(delta, payload).await
            }
            "arc_milestone_reached" => self.apply_arc_milestone_delta(delta, payload).await,
            "quantity_change" => {
                let input: CommitQuantityStateInput =
                    self.deser_with_provenance(delta, payload).await?;
                let out = self.commit_quantity_state(input).await?;
                Ok(Some(out.quantity_state_id))
            }
            "entity_candidate" => {
                let kind = payload
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .map(|k| k.trim().to_ascii_lowercase())
                    .unwrap_or_default();
                match kind.as_str() {
                    "character" => {
                        let input: CreateCharacterInput =
                            self.deser_with_provenance(delta, payload).await?;
                        let out = self.create_character(input).await?;
                        Ok(Some(out.character_id))
                    }
                    "location" => {
                        let input: CreateLocationInput =
                            self.deser_with_provenance(delta, payload).await?;
                        let out = self.create_location(input).await?;
                        Ok(Some(out.location_id))
                    }
                    "term" => {
                        let input: CreateTermInput =
                            self.deser_with_provenance(delta, payload).await?;
                        let out = self.create_term(input).await?;
                        Ok(Some(out.term_id))
                    }
                    other => anyhow::bail!("entity_candidate kind '{other}' is not creatable"),
                }
            }
            other => anyhow::bail!("unknown canon delta class '{other}'"),
        }
    }

    /// Apply a conflict-touching delta (`try_fail_cycle`, `consequence_delivered`,
    /// `escalation_demonstrated`) by reading the conflict, computing the merged
    /// JSON, and writing it back through the SAME `update_entity` column path the
    /// tool surface uses (never raw SQL). Returns `None` (an in-place update).
    pub(super) async fn apply_conflict_delta(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<Option<String>> {
        use crate::sqlite::json_records::{
            StoredStatedConsequence, StoredStoryPlacement, StoredTryFailCycleStep,
        };
        use spindle_core::models::UpdateEntityInput;

        let conflict_id = self.canon_delta_conflict_id(delta, payload)?;
        let conflict = self.repository.get_conflict(&conflict_id).await?;
        let placement = self.canon_delta_scene_placement(delta).await?;

        let (column, value) = match delta.delta_class.as_str() {
            "try_fail_cycle" => {
                // Append a new cycle step. The payload carries the step body
                // (label/outcome/cost/revelation); attempt_order defaults to the
                // next slot when omitted.
                let mut cycles: Vec<StoredTryFailCycleStep> = conflict.try_fail_cycles.clone();
                let next_order = cycles.iter().map(|c| c.attempt_order).max().unwrap_or(0) + 1;
                let step = StoredTryFailCycleStep {
                    attempt_order: payload
                        .get("attempt_order")
                        .and_then(|v| v.as_i64())
                        .map(|v| v as i32)
                        .unwrap_or(next_order),
                    label: payload
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    outcome: payload
                        .get("outcome")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    cost: payload
                        .get("cost")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    revelation: payload
                        .get("revelation")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                };
                cycles.push(step);
                ("try_fail_cycles", serde_json::to_value(&cycles)?)
            }
            "consequence_delivered" => {
                let index = self.canon_delta_index(payload, "consequence_index")?;
                let mut consequences: Vec<StoredStatedConsequence> =
                    conflict.stated_consequences.clone();
                let entry = consequences
                    .get_mut(index)
                    .ok_or_else(|| anyhow::anyhow!("consequence_index {index} out of range"))?;
                entry.delivered = true;
                ("stated_consequences", serde_json::to_value(&consequences)?)
            }
            "escalation_demonstrated" => {
                let index = self.canon_delta_index(payload, "stage_index")?;
                let mut demonstrated: Vec<Option<StoredStoryPlacement>> =
                    conflict.escalation_demonstrated.clone();
                // Grow the vec so index is addressable, filling gaps with None.
                if demonstrated.len() <= index {
                    demonstrated.resize(index + 1, None);
                }
                demonstrated[index] = Some(StoredStoryPlacement::from(placement));
                (
                    "escalation_demonstrated",
                    serde_json::to_value(&demonstrated)?,
                )
            }
            other => anyhow::bail!("apply_conflict_delta called for non-conflict class '{other}'"),
        };

        self.update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "conflict".to_string(),
            entity_id: conflict_id,
            changes: serde_json::json!({ column: value }),
        })
        .await?;
        Ok(None)
    }

    /// Apply an `arc_milestone_reached` delta by reading the arc, stamping the
    /// matching-label milestone's `reached_at` with the mined scene's placement,
    /// and writing the whole milestones array back via `update_entity`.
    pub(super) async fn apply_arc_milestone_delta(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<Option<String>> {
        use crate::sqlite::json_records::StoredStoryPlacement;
        use spindle_core::models::UpdateEntityInput;

        let arc_id = self.canon_delta_arc_id(delta, payload)?;
        let arc = self.repository.get_character_arc(&arc_id).await?;
        let label = self.canon_delta_milestone_label(payload)?;
        let placement = self.canon_delta_scene_placement(delta).await?;

        let mut milestones = arc.milestones.clone();
        let target = milestones
            .iter_mut()
            .find(|m| m.label == label)
            .ok_or_else(|| anyhow::anyhow!("arc '{arc_id}' has no milestone '{label}'"))?;
        target.reached_at = Some(StoredStoryPlacement::from(placement));

        self.update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "character_arc".to_string(),
            entity_id: arc_id,
            changes: serde_json::json!({ "milestones": serde_json::to_value(&milestones)? }),
        })
        .await?;
        Ok(None)
    }

    // ── Canon-delta payload helpers ──────────────────────────────────────────

    /// Normalize a staged (or operator-edited) canon-delta payload into the
    /// shape the class DTO expects (defect item 4). The miner routinely emits
    /// bare ULIDs without table prefixes, plural table/kind names,
    /// `description` where a create DTO wants `summary`/`definition`, explicit
    /// null profile structs, and a free-string `planted_at`. Normalizing here —
    /// used by BOTH pre-flight and apply, so they always see the same payload —
    /// keeps previously staged rows ratifiable instead of forcing manual `edit`
    /// reconstruction. Non-object payloads pass through untouched; the class
    /// handlers report those.
    pub(super) async fn normalize_canon_delta_payload(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let serde_json::Value::Object(map) = payload else {
            return Ok(payload.clone());
        };
        let mut object = map.clone();

        match delta.delta_class.as_str() {
            "promise_planted" => {
                // A free-string (or missing/null) planted_at coerces to the
                // mined scene's placement; the miner's string survives as the
                // placement note. A string planned_payoff has no placement to
                // coerce to — it moves into `notes` so nothing is dropped.
                let needs_placement = matches!(
                    object.get("planted_at"),
                    None | Some(serde_json::Value::Null) | Some(serde_json::Value::String(_))
                );
                if needs_placement {
                    let note = object
                        .get("planted_at")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(str::to_string);
                    let scene = self.repository.get_scene(&delta.scene_id).await?;
                    object.insert(
                        "planted_at".to_string(),
                        serde_json::json!({
                            "book_number": scene.book_number,
                            "chapter_number": scene.chapter_number,
                            "scene_order": scene.scene_order,
                            "note": note,
                        }),
                    );
                }
                if let Some(serde_json::Value::String(payoff)) = object.get("planned_payoff") {
                    let payoff = payoff.clone();
                    object.remove("planned_payoff");
                    if !payoff.trim().is_empty() {
                        let notes = object
                            .entry("notes".to_string())
                            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
                        if let serde_json::Value::Array(notes) = notes {
                            notes.push(serde_json::Value::String(format!(
                                "planned payoff: {payoff}"
                            )));
                        }
                    }
                }
            }
            "canonical_fact" | "quantity_change" => {
                if let Some(serde_json::Value::String(table)) = object.get("subject_table") {
                    let singular = canon_singularize_table(table);
                    object.insert(
                        "subject_table".to_string(),
                        serde_json::Value::String(singular),
                    );
                }
            }
            "character_state" | "knowledge_learned" => {
                canon_prefix_payload_id(&mut object, "character_id", "character");
            }
            "relationship_shift" => {
                canon_prefix_payload_id(&mut object, "character_a_id", "character");
                canon_prefix_payload_id(&mut object, "character_b_id", "character");
            }
            "entity_candidate" => {
                if let Some(serde_json::Value::String(kind)) = object.get("kind") {
                    let singular = canon_singularize_table(kind);
                    object.insert("kind".to_string(), serde_json::Value::String(singular));
                }
                let kind = object
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                match kind.as_str() {
                    "character" => {
                        canon_rename_missing(&mut object, "description", "summary");
                        object
                            .entry("role".to_string())
                            .or_insert_with(|| serde_json::Value::String("minor".to_string()));
                        // Null profile structs are the miner's way of saying
                        // "unknown"; drop the keys so the DTO defaults apply.
                        canon_drop_null(&mut object, "voice_profile");
                        canon_drop_null(&mut object, "emotional_profile");
                        canon_drop_null(&mut object, "initial_state");
                    }
                    "location" => {
                        canon_rename_missing(&mut object, "description", "summary");
                        canon_drop_null(&mut object, "initial_state");
                    }
                    "term" => {
                        canon_rename_missing(&mut object, "name", "term_text");
                        canon_rename_missing(&mut object, "description", "definition");
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        Ok(serde_json::Value::Object(object))
    }

    /// Merge the delta's provenance (project_id + the mined scene's
    /// book_number/chapter_number/scene_order + scene_id) into the payload
    /// object, then deserialize into the target DTO. Provenance is injected only
    /// where the payload does not already carry it, so an operator edit can
    /// override. Reads the mined scene once for the numeric position fields the
    /// quantity/fact DTOs require (they live on the scene, not the delta row).
    pub(super) async fn deser_with_provenance<T: serde::de::DeserializeOwned>(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<T> {
        let mut object = match payload {
            serde_json::Value::Object(map) => map.clone(),
            serde_json::Value::Null => serde_json::Map::new(),
            other => anyhow::bail!("canon delta payload must be a JSON object (got {other})"),
        };
        let scene = self.repository.get_scene(&delta.scene_id).await?;
        object
            .entry("project_id".to_string())
            .or_insert_with(|| serde_json::Value::String(delta.project_id.clone()));
        object
            .entry("scene_id".to_string())
            .or_insert_with(|| serde_json::Value::String(delta.scene_id.clone()));
        object
            .entry("book_number".to_string())
            .or_insert(serde_json::Value::Number(scene.book_number.into()));
        object
            .entry("chapter_number".to_string())
            .or_insert(serde_json::Value::Number(scene.chapter_number.into()));
        object
            .entry("scene_order".to_string())
            .or_insert(serde_json::Value::Number(scene.scene_order.into()));
        serde_json::from_value::<T>(serde_json::Value::Object(object)).map_err(|err| {
            anyhow::anyhow!(
                "canon delta '{}' payload does not match the {} DTO: {err}",
                delta.id,
                delta.delta_class
            )
        })
    }

    /// Build a fully-injected `RegisterCanonicalFactInput` from a
    /// `canonical_fact` delta: the payload plus project/scene/book/chapter from
    /// provenance. The book/chapter numbers come from the mined scene (the DTO
    /// requires them and `register_canonical_fact` writes them onto the fact).
    pub(super) async fn build_register_fact_input(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<spindle_core::models::RegisterCanonicalFactInput> {
        use spindle_core::models::RegisterCanonicalFactInput;
        let mut object = match payload {
            serde_json::Value::Object(map) => map.clone(),
            serde_json::Value::Null => serde_json::Map::new(),
            other => anyhow::bail!("canonical_fact payload must be a JSON object (got {other})"),
        };
        let scene = self.repository.get_scene(&delta.scene_id).await?;
        object
            .entry("project_id".to_string())
            .or_insert_with(|| serde_json::Value::String(delta.project_id.clone()));
        object
            .entry("scene_id".to_string())
            .or_insert_with(|| serde_json::Value::String(delta.scene_id.clone()));
        object
            .entry("book_number".to_string())
            .or_insert(serde_json::Value::Number(scene.book_number.into()));
        object
            .entry("chapter_number".to_string())
            .or_insert(serde_json::Value::Number(scene.chapter_number.into()));
        serde_json::from_value::<RegisterCanonicalFactInput>(serde_json::Value::Object(object))
            .map_err(|err| {
                anyhow::anyhow!(
                    "canon delta '{}' canonical_fact payload is malformed: {err}",
                    delta.id
                )
            })
    }

    /// The mined scene's `StoryPlacement`, used to stamp arc/escalation markers.
    pub(super) async fn canon_delta_scene_placement(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
    ) -> Result<spindle_core::models::StoryPlacement> {
        let scene = self.repository.get_scene(&delta.scene_id).await?;
        Ok(spindle_core::models::StoryPlacement {
            book_number: scene.book_number,
            chapter_number: scene.chapter_number,
            scene_order: Some(scene.scene_order),
            note: None,
        })
    }

    pub(super) fn canon_delta_promise_id(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<String> {
        payload
            .get("narrative_promise_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| delta.target_id.clone())
            .map(|id| canon_prefix_id(&id, "narrative_promise"))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "promise delta '{}' has no narrative_promise_id (payload or target_id)",
                    delta.id
                )
            })
    }

    /// The proposed promise status for a payoff/reinforced delta. Payload key is
    /// `proposed_status` (ADR D1); falls back to the class default.
    pub(super) fn canon_delta_promise_status(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<String> {
        if let Some(status) = payload.get("proposed_status").and_then(|v| v.as_str()) {
            return Ok(status.to_string());
        }
        Ok(match delta.delta_class.as_str() {
            "promise_reinforced" => "reinforced".to_string(),
            _ => "paid_off".to_string(),
        })
    }

    pub(super) fn canon_delta_conflict_id(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<String> {
        payload
            .get("conflict_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| delta.target_id.clone())
            .map(|id| canon_prefix_id(&id, "conflict"))
            .ok_or_else(|| anyhow::anyhow!("conflict delta '{}' has no conflict_id", delta.id))
    }

    pub(super) fn canon_delta_arc_id(
        &self,
        delta: &crate::sqlite::records::StoredCanonDelta,
        payload: &serde_json::Value,
    ) -> Result<String> {
        payload
            .get("character_arc_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| delta.target_id.clone())
            .map(|id| canon_prefix_id(&id, "character_arc"))
            .ok_or_else(|| anyhow::anyhow!("arc delta '{}' has no character_arc_id", delta.id))
    }

    pub(super) fn canon_delta_milestone_label(
        &self,
        payload: &serde_json::Value,
    ) -> Result<String> {
        payload
            .get("milestone_label")
            .or_else(|| payload.get("label"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("arc_milestone_reached payload has no milestone_label"))
    }

    pub(super) fn canon_delta_index(
        &self,
        payload: &serde_json::Value,
        key: &str,
    ) -> Result<usize> {
        payload
            .get(key)
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .ok_or_else(|| anyhow::anyhow!("payload is missing required index '{key}'"))
    }
}
