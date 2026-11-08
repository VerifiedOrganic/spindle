use super::*;

impl SqliteSpindleService {
    // =========================================================================
    // Plan-amendment ratification (ADR 0003 — the living-outline apply
    // dispatcher). Mirrors the canon-delta ratifier (D3/D5) exactly, plus the
    // ADR 0003 D3 immutability guard and D4 prior-state/plan_revision capture.
    // =========================================================================

    /// List the plan amendments staged on a project's active branch (ADR 0003 —
    /// the living-outline ratify queue). Thin over
    /// [`Repository::list_plan_amendments`]; filters compose (AND):
    /// `status` narrows to `staged`/`applied`/`rejected`/`superseded`, and
    /// `book_number` + `source_chapter` scope to one replan pass's provenance
    /// (both required together — a lone one is an input error). Mirrors
    /// [`Self::list_canon_deltas`].
    pub async fn list_plan_amendments(
        &self,
        input: spindle_core::models::ListPlanAmendmentsInput,
    ) -> Result<spindle_core::models::ListPlanAmendmentsOutput> {
        use spindle_core::models::ListPlanAmendmentsOutput;

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;

        let provenance = match (input.book_number, input.source_chapter) {
            (Some(book), Some(chapter)) => Some((book, chapter)),
            (None, None) => None,
            _ => anyhow::bail!(
                "list_plan_amendments: book_number and source_chapter must be supplied together \
                 (both scope to one replan pass's provenance)"
            ),
        };

        let stored = self
            .repository
            .list_plan_amendments(
                &input.project_id,
                &active_branch.id,
                input.status.as_deref(),
                provenance,
            )
            .await?;
        Ok(ListPlanAmendmentsOutput {
            amendments: stored.into_iter().map(|a| a.into_core()).collect(),
        })
    }

    /// The living-outline apply dispatcher (ADR 0003 D3/D4/D5). Ratifies a batch
    /// of operator decisions on staged plan amendments:
    ///
    /// 1. **Pre-flight ALL before applying ANY.** Every decision is checked: the
    ///    amendment exists, belongs to the project, and is currently `staged`
    ///    (deciding a decided/superseded row is an input error — decisions are
    ///    final). For `apply` decisions, the (edited) payload re-validates per
    ///    class, the **ADR D3 immutability guard** runs (the target chapter has
    ///    ZERO persisted scenes on the active branch AT APPLY TIME for every
    ///    chapter-targeting class; `promise_followup` instead requires its
    ///    placement be at or after the earliest undrafted chapter in its book),
    ///    and referential/coherence checks run against the CURRENT plan
    ///    (scene_order exists for drop/replace, new_order is a permutation of the
    ///    live spine, insert_at coherent). Any failure aborts with **zero
    ///    writes** — every row stays staged.
    /// 2. **Apply phase (input order).** Each `apply` reads the CURRENT chapter
    ///    plan, snapshots the full plan row JSON as `prior_state` (ADR D4),
    ///    computes the amended [`PlanChapterInput`], replays it through the
    ///    EXISTING [`Self::plan_chapter`] service path (never a raw write), then
    ///    increments `chapter_plan.plan_revision` (the replay resets it, so the
    ///    increment is written back explicitly), then records the decision with
    ///    the prior state. `promise_followup` replays through
    ///    [`Self::create_narrative_promise`] and captures no prior state (it
    ///    targets a placement, not a chapter row). A `reject` records the
    ///    decision only (prior_state `None`).
    /// 3. **Mid-apply honesty.** A real write error after pre-flight stops the
    ///    dispatcher: earlier applies stay applied (they are real outline
    ///    changes — nothing is rolled back), the failing row is reported `failed`
    ///    with its error and stays staged, and remaining rows read `not_reached`
    ///    and stay staged. Mirrors [`Self::decide_canon_deltas`].
    pub async fn decide_plan_amendments(
        &self,
        input: spindle_core::models::DecidePlanAmendmentsInput,
    ) -> Result<spindle_core::models::DecidePlanAmendmentsOutput> {
        self.decide_plan_amendments_with_seam(input, || async {})
            .await
    }

    /// Implementation of [`Self::decide_plan_amendments`] with an injectable hook
    /// run once, after pre-flight and before the apply phase. The public entry
    /// point passes a no-op; a test injects a real mutation (e.g. deleting a plan
    /// row pre-flight validated) to exercise the mid-apply-failure path through
    /// the genuine dispatch code. Mirrors [`Self::decide_canon_deltas_with_seam`].
    pub(crate) async fn decide_plan_amendments_with_seam<H, Fut>(
        &self,
        input: spindle_core::models::DecidePlanAmendmentsInput,
        after_preflight: H,
    ) -> Result<spindle_core::models::DecidePlanAmendmentsOutput>
    where
        H: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        use crate::sqlite::repository::PlanAmendmentDecision;
        use spindle_core::models::{DecidePlanAmendmentsOutput, PlanAmendmentDecisionResult};

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let decided_by = input
            .decided_by
            .as_deref()
            .unwrap_or("operator")
            .to_string();

        // ── Pre-flight ALL ───────────────────────────────────────────────────
        struct Preflighted {
            amendment_id: String,
            action: Action,
            effective_payload: serde_json::Value,
            amendment: crate::sqlite::records::StoredPlanAmendment,
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
                        decision.amendment_id
                    ));
                    continue;
                }
            };
            let amendment = match self
                .repository
                .read_plan_amendment_public(&decision.amendment_id)
                .await?
            {
                Some(amendment) => amendment,
                None => {
                    offenders.push(format!(
                        "amendment '{}' does not exist",
                        decision.amendment_id
                    ));
                    continue;
                }
            };
            if amendment.project_id != input.project_id {
                offenders.push(format!(
                    "amendment '{}' does not belong to project '{}'",
                    decision.amendment_id, input.project_id
                ));
                continue;
            }
            if amendment.status != "staged" {
                offenders.push(format!(
                    "amendment '{}' is '{}', not staged (decisions are final)",
                    decision.amendment_id, amendment.status
                ));
                continue;
            }
            let effective_payload = decision
                .edit
                .clone()
                .unwrap_or_else(|| amendment.payload.clone());
            // Box the fan-out future (pre-flight re-validates the payload, runs
            // the ADR D3 immutability guard, and does referential checks against
            // the current plan for ~8 classes). One heap indirection per apply
            // decision keeps the enclosing MCP `call_tool` future off the worker
            // stack (same class as the canon-delta fan-out fix).
            if matches!(action, Action::Apply)
                && let Err(err) = Box::pin(self.preflight_apply_plan_amendment(
                    &amendment,
                    &effective_payload,
                    &active_branch.id,
                ))
                .await
            {
                offenders.push(format!(
                    "amendment '{}' failed pre-flight: {err}",
                    decision.amendment_id
                ));
                continue;
            }
            preflighted.push(Preflighted {
                amendment_id: decision.amendment_id.clone(),
                action,
                effective_payload,
                amendment,
                note: decision.note.clone(),
            });
        }

        if !offenders.is_empty() {
            anyhow::bail!(
                "decide_plan_amendments pre-flight failed ({} offender(s)); zero writes applied: {}",
                offenders.len(),
                offenders.join("; ")
            );
        }

        // Test/instrumentation seam: mutate state between pre-flight and apply.
        after_preflight().await;

        // ── Apply phase (input order) ────────────────────────────────────────
        let mut results: Vec<PlanAmendmentDecisionResult> = Vec::new();
        let mut applied_count = 0usize;
        let mut rejected_count = 0usize;
        let mut failed_count = 0usize;
        let mut stopped = false;

        for item in preflighted {
            if stopped {
                results.push(PlanAmendmentDecisionResult {
                    amendment_id: item.amendment_id,
                    outcome: "not_reached".to_string(),
                    error: None,
                    applied_record_id: None,
                    note: item.note,
                });
                continue;
            }
            match item.action {
                Action::Reject => {
                    // Reject records the decision with no prior state (no write).
                    self.repository
                        .decide_plan_amendment(
                            &item.amendment_id,
                            PlanAmendmentDecision::Rejected,
                            &decided_by,
                            None,
                        )
                        .await?;
                    rejected_count += 1;
                    results.push(PlanAmendmentDecisionResult {
                        amendment_id: item.amendment_id,
                        outcome: "rejected".to_string(),
                        error: None,
                        applied_record_id: None,
                        note: item.note,
                    });
                }
                Action::Apply => {
                    match Box::pin(self.apply_plan_amendment(
                        &item.amendment,
                        &item.effective_payload,
                        &active_branch.id,
                    ))
                    .await
                    {
                        Ok((applied_record_id, prior_state)) => {
                            self.repository
                                .decide_plan_amendment(
                                    &item.amendment_id,
                                    PlanAmendmentDecision::Applied,
                                    &decided_by,
                                    prior_state,
                                )
                                .await?;
                            applied_count += 1;
                            results.push(PlanAmendmentDecisionResult {
                                amendment_id: item.amendment_id,
                                outcome: "applied".to_string(),
                                error: None,
                                applied_record_id,
                                note: item.note,
                            });
                        }
                        Err(err) => {
                            failed_count += 1;
                            results.push(PlanAmendmentDecisionResult {
                                amendment_id: item.amendment_id,
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

        Ok(DecidePlanAmendmentsOutput {
            results,
            applied_count,
            rejected_count,
            failed_count,
        })
    }

    /// The earliest chapter (in `book_number`) with ZERO persisted scenes on the
    /// active branch — the "next undrafted chapter". Used by the ADR D3
    /// immutability guard: a `promise_followup` placement must be at or after
    /// this. `None` when no plan/chapter is undrafted (every planned chapter is
    /// drafted); callers treat that as "any placement is fine" only when there is
    /// genuinely nothing left to protect.
    pub(super) async fn earliest_undrafted_chapter(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
    ) -> Result<Option<i32>> {
        let plans = self
            .repository
            .list_chapter_plans_by_project(project_id)
            .await?;
        let scenes = self
            .repository
            .list_scenes_by_project_and_branch(project_id, branch_id)
            .await?;
        let drafted: std::collections::BTreeSet<i32> = scenes
            .iter()
            .filter(|s| s.book_number == book_number)
            .map(|s| s.chapter_number)
            .collect();
        let earliest = plans
            .iter()
            .filter(|p| p.book_number == book_number && !drafted.contains(&p.chapter_number))
            .map(|p| p.chapter_number)
            .min();
        Ok(earliest)
    }

    /// Whether a chapter has any persisted scene on the active branch (ADR D3
    /// immutability guard — drafted reality is never rewritten by the outline).
    pub(super) async fn chapter_has_persisted_scenes(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<bool> {
        let scenes = self
            .repository
            .list_scenes_by_project_and_branch(project_id, branch_id)
            .await?;
        Ok(scenes
            .iter()
            .any(|s| s.book_number == book_number && s.chapter_number == chapter_number))
    }

    /// Fetch the CURRENT chapter plan for one chapter on the active branch, or an
    /// error naming the missing plan. The apply dispatcher reads through this so
    /// a plan deleted between pre-flight and apply fails honestly at write time.
    pub(super) async fn current_chapter_plan(
        &self,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<crate::sqlite::records::ChapterPlan> {
        let plans = self
            .repository
            .list_chapter_plans_by_project(project_id)
            .await?;
        plans
            .into_iter()
            .find(|p| p.book_number == book_number && p.chapter_number == chapter_number)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "chapter {chapter_number} (book {book_number}) has no current plan to amend"
                )
            })
    }

    /// Cheap referential + immutability pre-flight for one `apply` decision (ADR
    /// 0003 D3). Re-validates the payload per class, runs the immutability guard,
    /// and confirms the amendment coheres with the CURRENT plan. No writes. Runs
    /// for every apply before any apply so a stale/dangling amendment aborts the
    /// batch with zero writes.
    pub(super) async fn preflight_apply_plan_amendment(
        &self,
        amendment: &crate::sqlite::records::StoredPlanAmendment,
        payload: &serde_json::Value,
        branch_id: &str,
    ) -> Result<()> {
        let class = amendment.amendment_class.as_str();
        // 1. Payload re-validates per class (Part A's validator).
        if !plan_amendment_payload_is_valid(class, payload) {
            anyhow::bail!("payload no longer valid for class '{class}'");
        }

        if class == "promise_followup" {
            // Placement guard (ADR D3): the followup must plant at or after the
            // earliest undrafted chapter in its book — never into drafted reality.
            let placement = payload
                .get("planted_at_placement")
                .context("promise_followup missing planted_at_placement")?;
            let book = placement
                .get("book_number")
                .and_then(|v| v.as_i64())
                .context("planted_at_placement missing book_number")? as i32;
            let chapter = placement
                .get("chapter_number")
                .and_then(|v| v.as_i64())
                .context("planted_at_placement missing chapter_number")?
                as i32;
            if let Some(earliest) = self
                .earliest_undrafted_chapter(&amendment.project_id, branch_id, book)
                .await?
                && chapter < earliest
            {
                anyhow::bail!(
                    "promise_followup placement chapter {chapter} precedes the earliest \
                     undrafted chapter {earliest} (ADR D3: cannot plant into drafted reality)"
                );
            }
            return Ok(());
        }

        // Every chapter-targeting class names a target chapter.
        let target_chapter = amendment
            .target_chapter
            .context("chapter-targeting amendment missing target_chapter")?;

        // 2. Immutability guard (ADR D3): the target chapter has ZERO persisted
        //    scenes on the active branch at apply time.
        if self
            .chapter_has_persisted_scenes(
                &amendment.project_id,
                branch_id,
                amendment.book_number,
                target_chapter,
            )
            .await?
        {
            anyhow::bail!(
                "immutability guard: target chapter {target_chapter} already has a persisted \
                 scene on the active branch — drafted reality is never rewritten by the outline \
                 (ADR 0003 D3)"
            );
        }

        // 3. Referential coherence against the CURRENT plan.
        let plan = self
            .current_chapter_plan(&amendment.project_id, amendment.book_number, target_chapter)
            .await?;
        let orders: Vec<i32> = plan.scenes.iter().map(|s| s.scene_order).collect();
        match class {
            "scene_drop" | "scene_replace" => {
                let order = payload
                    .get("scene_order")
                    .and_then(|v| v.as_i64())
                    .context("scene payload missing scene_order")?
                    as i32;
                if !orders.contains(&order) {
                    anyhow::bail!(
                        "{class}: scene_order {order} is not in chapter {target_chapter}'s current \
                         spine {orders:?}"
                    );
                }
            }
            "scene_reorder" => {
                let new_order: Vec<i32> = payload
                    .get("new_order")
                    .and_then(|v| v.as_array())
                    .context("scene_reorder missing new_order")?
                    .iter()
                    .filter_map(|v| v.as_i64().map(|n| n as i32))
                    .collect();
                let mut want = orders.clone();
                want.sort_unstable();
                let mut got = new_order.clone();
                got.sort_unstable();
                if got != want {
                    anyhow::bail!(
                        "scene_reorder new_order {new_order:?} is not a permutation of chapter \
                         {target_chapter}'s current spine {orders:?}"
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Apply one pre-flighted plan amendment by replaying the class through the
    /// EXISTING plan write path (ADR 0003 D1 — never a novel write). Returns
    /// `(applied_record_id, prior_state)`: the created promise id +
    /// no-prior-state for `promise_followup`, or `None` + the full pre-apply plan
    /// row JSON for the chapter-targeting classes (ADR D4). After a chapter-plan
    /// replay the caller-owned `plan_revision` is re-written incremented, because
    /// the replay resets it.
    pub(super) async fn apply_plan_amendment(
        &self,
        amendment: &crate::sqlite::records::StoredPlanAmendment,
        payload: &serde_json::Value,
        branch_id: &str,
    ) -> Result<(Option<String>, Option<String>)> {
        use spindle_core::models::{
            CreateNarrativePromiseInput, PlanChapterInput, PlanChapterSceneInput,
        };

        let class = amendment.amendment_class.as_str();

        if class == "promise_followup" {
            let placement = payload
                .get("planted_at_placement")
                .context("promise_followup missing planted_at_placement")?;
            let planted_at = self.story_placement_from_json(placement)?;
            let planned_payoff = match payload.get("planned_payoff") {
                None | Some(serde_json::Value::Null) => None,
                Some(payoff) => Some(self.story_placement_from_json(payoff)?),
            };
            let promise_type = payload
                .get("promise_type")
                .and_then(|v| v.as_str())
                .context("promise_followup missing promise_type")?
                .to_string();
            let description = payload
                .get("description")
                .and_then(|v| v.as_str())
                .context("promise_followup missing description")?
                .to_string();
            let out = self
                .create_narrative_promise(CreateNarrativePromiseInput {
                    project_id: amendment.project_id.clone(),
                    promise_type,
                    description,
                    planted_at,
                    planned_payoff,
                    notes: Vec::new(),
                })
                .await?;
            return Ok((Some(out.narrative_promise_id), None));
        }

        // Chapter-targeting classes: read the CURRENT plan, snapshot it as
        // prior_state, compute the amended input, replay, then bump plan_revision.
        let target_chapter = amendment
            .target_chapter
            .context("chapter-targeting amendment missing target_chapter")?;
        let plan = self
            .current_chapter_plan(&amendment.project_id, amendment.book_number, target_chapter)
            .await?;
        let prior_state = serde_json::to_string(&plan).context("snapshotting prior plan state")?;
        let next_revision = plan.plan_revision.unwrap_or(0) + 1;

        // Rebuild the plan's scenes as `(original_scene_order, PlanChapterSceneInput)`
        // so the amendment mutates the SAME shape the replay consumes while keeping
        // pre-amendment scene lineage for active authoring run re-seeding.
        let mut scenes: Vec<(Option<i32>, PlanChapterSceneInput)> = plan
            .scenes
            .iter()
            .map(|s| {
                (
                    Some(s.scene_order),
                    PlanChapterSceneInput {
                        scene_order: s.scene_order,
                        summary: s.summary.clone(),
                        beat_structure: s.beat_structure.clone(),
                        character_ids: s.character_ids.clone(),
                        location_id: s.location_id.clone(),
                        content_rating: s.content_rating.clone(),
                        purpose: s.purpose.clone(),
                        research_required: s.research_required,
                        research_tags: s.research_tags.clone(),
                        explicit_query: s.explicit_query.clone(),
                    },
                )
            })
            .collect();
        scenes.sort_by_key(|(_, s)| s.scene_order);

        let mut synopsis = plan.synopsis.clone();
        let mut target_theme_ids = plan.target_theme_ids.clone();
        let mut target_conflict_ids = plan.target_conflict_ids.clone();
        let mut target_plot_line_ids = plan.target_plot_line_ids.clone();

        match class {
            "synopsis_update" => {
                synopsis = payload
                    .get("synopsis")
                    .and_then(|v| v.as_str())
                    .context("synopsis_update missing synopsis")?
                    .to_string();
            }
            "scene_add" => {
                let insert_at = payload
                    .get("insert_at")
                    .and_then(|v| v.as_i64())
                    .context("scene_add missing insert_at")?
                    as usize;
                // insert_at is 1-based; clamp into [0, len] for the Vec index.
                let idx = insert_at.saturating_sub(1).min(scenes.len());
                let neighbor = scenes
                    .get(idx)
                    .or_else(|| idx.checked_sub(1).and_then(|i| scenes.get(i)))
                    .map(|(_, s)| s);
                let mut character_ids: Vec<String> = payload
                    .get("character_ids")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                if character_ids.is_empty() {
                    if let Some(nb) = neighbor
                        && !nb.character_ids.is_empty()
                    {
                        character_ids = nb.character_ids.clone();
                    } else if let Some(pov) = plan.pov_character_id.as_ref() {
                        character_ids = vec![pov.clone()];
                    }
                }
                let location_id = payload
                    .get("location_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_string)
                    .or_else(|| neighbor.and_then(|nb| nb.location_id.clone()));
                let content_rating = payload
                    .get("content_rating")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .or_else(|| neighbor.and_then(|nb| nb.content_rating.clone()));
                let new_scene = PlanChapterSceneInput {
                    scene_order: 0, // renumbered below
                    summary: payload
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    beat_structure: payload
                        .get("beat_structure")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    character_ids,
                    location_id,
                    content_rating,
                    purpose: payload
                        .get("purpose")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    research_required: None,
                    research_tags: Vec::new(),
                    explicit_query: None,
                };
                scenes.insert(idx, (None, new_scene));
            }
            "scene_drop" => {
                let order = payload
                    .get("scene_order")
                    .and_then(|v| v.as_i64())
                    .context("scene_drop missing scene_order")? as i32;
                scenes.retain(|(_, s)| s.scene_order != order);
            }
            "scene_replace" => {
                let order = payload
                    .get("scene_order")
                    .and_then(|v| v.as_i64())
                    .context("scene_replace missing scene_order")?
                    as i32;
                if let Some((_, target)) = scenes.iter_mut().find(|(_, s)| s.scene_order == order) {
                    target.summary = payload
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&target.summary)
                        .to_string();
                    target.purpose = payload
                        .get("purpose")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&target.purpose)
                        .to_string();
                    if let Some(ids) = payload.get("character_ids").and_then(|v| v.as_array()) {
                        let parsed: Vec<String> = ids
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect();
                        if !parsed.is_empty() {
                            target.character_ids = parsed;
                        }
                    }
                    if let Some(loc) = payload.get("location_id").and_then(|v| v.as_str()) {
                        target.location_id = Some(loc.to_string());
                    }
                    if let Some(rating) = payload.get("content_rating")
                        && !rating.is_null()
                        && let Ok(parsed) = serde_json::from_value(rating.clone())
                    {
                        target.content_rating = Some(parsed);
                    }
                }
            }
            "scene_reorder" => {
                let new_order: Vec<i32> = payload
                    .get("new_order")
                    .and_then(|v| v.as_array())
                    .context("scene_reorder missing new_order")?
                    .iter()
                    .filter_map(|v| v.as_i64().map(|n| n as i32))
                    .collect();
                // Reorder the scenes to follow new_order's sequence of original
                // scene_orders; the renumber pass below assigns 1..n.
                let mut reordered: Vec<(Option<i32>, PlanChapterSceneInput)> =
                    Vec::with_capacity(scenes.len());
                for order in &new_order {
                    if let Some(pos) = scenes.iter().position(|(_, s)| s.scene_order == *order) {
                        reordered.push(scenes.remove(pos));
                    }
                }
                reordered.append(&mut scenes);
                scenes = reordered;
            }
            "thread_promote" => {
                let (kind, id) = self.plan_amendment_thread_kv(payload)?;
                let bucket = match kind.as_str() {
                    "theme" => &mut target_theme_ids,
                    "conflict" => &mut target_conflict_ids,
                    "plot_line" => &mut target_plot_line_ids,
                    other => anyhow::bail!("thread_promote unknown kind '{other}'"),
                };
                if !bucket.contains(&id) {
                    bucket.push(id);
                }
            }
            "thread_retire" => {
                let (kind, id) = self.plan_amendment_thread_kv(payload)?;
                let bucket = match kind.as_str() {
                    "theme" => &mut target_theme_ids,
                    "conflict" => &mut target_conflict_ids,
                    "plot_line" => &mut target_plot_line_ids,
                    other => anyhow::bail!("thread_retire unknown kind '{other}'"),
                };
                bucket.retain(|existing| existing != &id);
            }
            other => anyhow::bail!("unknown plan amendment class '{other}'"),
        }

        // Renumber the spine 1..n so orders stay dense after insert/drop/reorder.
        for (index, (_, scene)) in scenes.iter_mut().enumerate() {
            scene.scene_order = index as i32 + 1;
        }
        let replay_scenes: Vec<PlanChapterSceneInput> =
            scenes.iter().map(|(_, s)| s.clone()).collect();

        // Replay through the EXISTING plan write path (never a raw write).
        self.plan_chapter(PlanChapterInput {
            project_id: amendment.project_id.clone(),
            book_number: amendment.book_number,
            chapter_number: target_chapter,
            pov_character_id: plan.pov_character_id.clone(),
            synopsis: synopsis.clone(),
            target_theme_ids,
            target_conflict_ids,
            target_plot_line_ids,
            scenes: replay_scenes,
        })
        .await?;

        // The replay reset plan_revision (it is not in the plan_chapter INSERT);
        // write the incremented value back so the ADR D4 counter survives.
        self.repository
            .set_chapter_plan_revision(
                &amendment.project_id,
                branch_id,
                amendment.book_number,
                target_chapter,
                next_revision,
            )
            .await?;

        // Keep any active/non-completed authoring run covering this undrafted
        // chapter in sync with the amended chapter plan so reconcile_chapter_plan
        // does not block the run on synopsis / scene_order / character_ids drift,
        // and invalidate pre-amendment scene generation artifacts.
        self.repository
            .reseed_authoring_runs_for_chapter_plan(
                &amendment.project_id,
                branch_id,
                amendment.book_number,
                target_chapter,
                &synopsis,
                plan.pov_character_id.as_deref(),
                &scenes,
                next_revision,
            )
            .await?;

        Ok((None, Some(prior_state)))
    }

    /// Extract the `(kind, id)` pair from a `thread_promote`/`thread_retire`
    /// payload, normalizing kind to lowercase. Both are validated present by the
    /// payload validator; this maps them for the apply.
    pub(super) fn plan_amendment_thread_kv(
        &self,
        payload: &serde_json::Value,
    ) -> Result<(String, String)> {
        let kind = payload
            .get("kind")
            .and_then(|v| v.as_str())
            .map(|k| k.trim().to_ascii_lowercase())
            .context("thread amendment missing kind")?;
        let id = payload
            .get("id")
            .and_then(|v| v.as_str())
            .context("thread amendment missing id")?
            .to_string();
        Ok((kind, id))
    }

    /// Map a JSON placement object (`{book_number, chapter_number, scene_order?}`)
    /// into a [`StoryPlacement`] for `promise_followup` apply.
    pub(super) fn story_placement_from_json(
        &self,
        value: &serde_json::Value,
    ) -> Result<spindle_core::models::StoryPlacement> {
        Ok(spindle_core::models::StoryPlacement {
            book_number: value
                .get("book_number")
                .and_then(|v| v.as_i64())
                .context("placement missing book_number")? as i32,
            chapter_number: value
                .get("chapter_number")
                .and_then(|v| v.as_i64())
                .context("placement missing chapter_number")? as i32,
            scene_order: value
                .get("scene_order")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32),
            note: value
                .get("note")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    }
}
