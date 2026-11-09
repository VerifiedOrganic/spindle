use super::*;

impl SqliteSpindleService {
    /// Save scene draft — the minimal MVP version. Returns the scene id and
    /// status. The full SurrealDB-side service runs many validators here
    /// (pacing, agency, tone deviation, world rule scan, etc.); those land
    /// in subsequent Phase 6 commits. For now the validation-derived fields
    /// default to "no warnings" so the MCP path is structurally correct.
    /// Assemble the project [`StyleDirective`](spindle_core::style::StyleDirective)
    /// — the single source of truth for genre-voice enforcement — from the
    /// reader contract, `style`-typed world rules, and narrator voice. Used by
    /// the save gate, the `style_compliance` validator, and the review's Target
    /// Reader persona.
    pub async fn style_directive_for(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<spindle_core::style::StyleDirective> {
        let project = self.repository.get_project(project_id).await?;
        let style_rules: Vec<spindle_core::style::StyleRule> = self
            .repository
            .list_world_rules_by_project_and_branch(project_id, branch_id)
            .await?
            .into_iter()
            .filter(|rule| rule.rule_type.eq_ignore_ascii_case("style"))
            .map(|rule| spindle_core::style::StyleRule {
                rule_name: rule.rule_name,
                description: rule.description,
            })
            .collect();
        Ok(spindle_core::style::StyleDirective::assemble(
            project.genre,
            project.project_type,
            project.reader_contract.promise,
            project.reader_contract.style_notes,
            project.reader_contract.boundaries,
            style_rules,
            project.narrator_voice.map(|stored| stored.into_core()),
        ))
    }

    /// Style-learning capture policy (evolution §3.9). Decides whether this
    /// operator re-save is a style signal and, if so, stages/replaces the
    /// candidate. Called only for operator (non-agent) UPDATE saves.
    ///
    /// Capture iff ALL hold:
    ///   * the project has `style_learning` enabled;
    ///   * the prose changed (byte compare after trim);
    ///   * EITHER the prior version has AGENT provenance (`draft_origin` starts
    ///     with `agent:`) — the fresh agent→operator edit — OR a pending
    ///     candidate already exists for this scene (a subsequent operator edit
    ///     of an already-captured pair, which replaces it).
    ///
    /// Provenance note (reported honestly): the only reliable DB signal for
    /// agent authorship is `scene.draft_origin` starting with `agent:`, which is
    /// stamped only on the explicit-agent draft save path. Non-explicit agent
    /// drafts leave `draft_origin` NULL and are therefore NOT covered by
    /// capture. To keep the operator-vs-operator boundary crisp, a capture
    /// stamps `draft_origin = "operator"` so a later same-scene save is
    /// recognized as operator-over-operator (replace, never a new pair).
    ///
    /// Degenerate guard: a scene whose agent draft OR operator edit exceeds
    /// [`STYLE_EDIT_MAX_CHARS`] skips capture with a `tracing::warn` (the pair
    /// would bloat the refresh corpus; scenes this large are not a clean style
    /// signal).
    pub(super) async fn maybe_capture_style_edit(
        &self,
        branch_id: &str,
        prior: &crate::sqlite::records::Scene,
        scene: &crate::sqlite::records::Scene,
        input: &SaveSceneDraftInput,
    ) -> Result<()> {
        if !self
            .repository
            .project_style_learning_enabled_public(&input.project_id)
            .await?
        {
            return Ok(());
        }
        // Prose must have actually changed (trim-equal is not an edit).
        if prior.full_text.trim() == scene.full_text.trim() {
            return Ok(());
        }

        let prior_is_agent = prior
            .draft_origin
            .as_deref()
            .is_some_and(|o| o.starts_with("agent:") || o == "assistant:host");
        let pending_exists = !self
            .repository
            .list_style_edit_candidates(&input.project_id, branch_id, Some("pending"))
            .await?
            .into_iter()
            .filter(|c| c.scene_id == scene.id)
            .collect::<Vec<_>>()
            .is_empty();

        // Agent→operator (fresh) or operator→operator (replace an existing pair).
        // A pure operator-over-operator save with no prior agent draft and no
        // pending candidate is NOT a style signal.
        if !prior_is_agent && !pending_exists {
            return Ok(());
        }

        // Degenerate guard: skip capture for pathologically large scenes.
        if prior.full_text.chars().count() > STYLE_EDIT_MAX_CHARS
            || scene.full_text.chars().count() > STYLE_EDIT_MAX_CHARS
        {
            tracing::warn!(
                scene_id = %scene.id,
                "style-edit capture skipped: scene exceeds {STYLE_EDIT_MAX_CHARS} chars"
            );
            return Ok(());
        }

        let captured = self
            .repository
            .capture_style_edit_candidate(crate::sqlite::repository::CaptureStyleEditParams {
                project_id: input.project_id.clone(),
                branch_id: branch_id.to_string(),
                scene_id: scene.id.clone(),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: scene.scene_order,
                // The agent draft (contrast) is the PRIOR prose. On a replace,
                // capture_style_edit_candidate preserves the original agent_draft
                // and only updates operator_edit.
                agent_draft: prior.full_text.clone(),
                operator_edit: scene.full_text.clone(),
                content_rating: scene.content_rating.trim().to_ascii_lowercase(),
            })
            .await?;
        if let Some(pack) = load_anti_slop_pack() {
            let learned = spindle_core::style::antislop::learn_suppressions_from_edit(
                &pack,
                &captured.agent_draft,
                &captured.operator_edit,
            );
            if !learned.is_empty()
                && let Err(error) = self
                    .repository
                    .upsert_anti_slop_suppressions(
                        &input.project_id,
                        branch_id,
                        &learned,
                        "style_edit",
                    )
                    .await
            {
                tracing::warn!(
                    scene_id = %scene.id,
                    error = %error,
                    "anti-slop suppression learn failed"
                );
            }
        }

        // Re-stamp provenance so a subsequent same-scene operator save is
        // recognized as operator-over-operator (replace, not a fresh pair).
        self.repository
            .update_scene_draft_origin(&scene.id, "operator")
            .await?;
        Ok(())
    }

    /// Best-effort content rating of the scene a caller is assembling context
    /// for, used by the cross-rating elision in `get_scene_context`.
    ///
    /// Two sources, in order of authority:
    ///   1. The drafted scene row, when it already exists (a re-draft/revision).
    ///   2. The chapter plan's per-scene `content_rating` — the only source for
    ///      a scene that has not been drafted yet, which is the normal state
    ///      when an agent asks for drafting context.
    ///
    /// `None` means "cannot establish", which callers must treat as
    /// not-cleared rather than as permission.
    pub(super) async fn resolve_target_scene_rating(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Option<String>> {
        if let Some(scene) = self
            .repository
            .find_scene_by_natural_key(
                project_id,
                branch_id,
                book_number,
                chapter_number,
                scene_order,
            )
            .await?
        {
            let rating = scene.content_rating.trim();
            if !rating.is_empty() {
                return Ok(Some(rating.to_ascii_lowercase()));
            }
        }

        let planned = self
            .repository
            .list_chapter_plans_by_project_and_branch(project_id, branch_id)
            .await?
            .into_iter()
            .find(|plan| plan.book_number == book_number && plan.chapter_number == chapter_number)
            .and_then(|plan| {
                plan.scenes
                    .into_iter()
                    .find(|scene| scene.scene_order == scene_order)
            })
            .and_then(|scene| scene.content_rating)
            .map(|rating| rating.as_str().to_ascii_lowercase());
        Ok(planned)
    }

    pub async fn save_scene_draft(
        &self,
        input: SaveSceneDraftInput,
    ) -> Result<SaveSceneDraftOutput> {
        self.save_scene_draft_with_seam(input, || async {}).await
    }

    pub async fn save_scene_draft_with_seam<H, Fut>(
        &self,
        input: SaveSceneDraftInput,
        before_scene_write: H,
    ) -> Result<SaveSceneDraftOutput>
    where
        H: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        // Resolve placement BEFORE anything else runs. `chapter_id` is
        // documented as sufficient: derive the numeric placement from the
        // existing chapter. Naming neither is a validation error — silently
        // defaulting to book/chapter 0 here is what let a failed save burn an
        // explicit receipt on a `book 0, chapter 0` claim in the live run.
        // `save_input` carries the resolved placement everywhere downstream
        // (persist, receipt claim key, prior-scene lookup). The caller's
        // prose is authoritative at every rating; no field is ever rewritten
        // from a receipt (see the explicit branch below).
        let mut save_input = input.clone();
        if let Some(chapter_id) = input.chapter_id.as_deref() {
            let chapter = self.repository.get_chapter(chapter_id).await?;
            if chapter.project_id != input.project_id {
                anyhow::bail!("chapter does not belong to the requested project");
            }
            if input.book_number > 0 && input.book_number != chapter.book_number {
                anyhow::bail!(
                    "chapter_id {} does not match book_number {}",
                    chapter_id,
                    input.book_number
                );
            }
            if input.chapter_number > 0 && input.chapter_number != chapter.chapter_number {
                anyhow::bail!(
                    "chapter_id {} does not match chapter_number {}",
                    chapter_id,
                    input.chapter_number
                );
            }
            save_input.book_number = chapter.book_number;
            save_input.chapter_number = chapter.chapter_number;
        } else if input.book_number <= 0 || input.chapter_number <= 0 {
            anyhow::bail!(
                "save_scene_draft requires chapter_id or both book_number and chapter_number"
            );
        }
        // The explicit-path receipt claim, deferred to the scene's own save
        // transaction below (receipt id, scene key). `None` for every
        // non-explicit save.
        let mut receipt_claim: Option<(String, String)> = None;
        // Provenance vocabulary (see the `draft_origin` field on
        // `crate::sqlite::records::Scene` for the canonical definition):
        //   * `agent:<agent-id>` — the run's draft agent authored this prose.
        //     Stamped for EVERY agent-origin save regardless of rating: the
        //     agent-draft seam always passes a valid `draft`-route
        //     `generation_id`. At every rating (explicit and non-explicit alike)
        //     the caller's `full_text` is authoritative and is what persists; the
        //     receipt is provenance only (rating clearance + agent identity). See
        //     the explicit-branch comment below for why the receipt output is
        //     never substituted for the prose. This uniform stamp is what unlocks
        //     style-edit capture for the non-explicit agent-draft majority (a
        //     NULL origin was never recognized as an agent draft).
        //   * `host` — a hybrid host draft through `authoring_save_scene_draft`
        //     (stamped by that handler after the save). The operator's own prose,
        //     not an agent style signal.
        //   * `operator` — re-stamped by `maybe_capture_style_edit` on capture so
        //     a later same-scene save is operator-over-operator (replace, never a
        //     fresh pair).
        // The capture predicate keys on `starts_with("agent:")`, so both explicit
        // and non-explicit agent stamps trip it uniformly.
        let mut draft_origin = (input.authorship
            == spindle_core::models::DraftAuthorship::Assistant)
            .then(|| "assistant:host".to_string());
        let mut originating_model_call_ids: Vec<String> = Vec::new();
        if matches!(
            input.content_rating,
            spindle_core::models::ContentRating::Explicit
        ) {
            if input
                .generation_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty())
            {
                // The receipt is PROVENANCE ONLY: it proves the save was
                // authorized by a cleared, explicit-capable draft-route
                // generation and identifies the producing agent. It is NOT the
                // source of prose. The caller's `full_text` is authoritative and
                // is what gets persisted.
                //
                // Data-loss regression (bugs 1 + 6b): the original code rewrote
                // `save_input.full_text = receipt.output_text` to "guarantee the
                // persisted explicit prose is byte-for-byte what the cleared
                // agent generated" (introduced in e6f6395). In production the
                // receipt is `prior_output + continuation` (see
                // register_generation_receipt) and carries agent narration +
                // truncated turn fragments, so it is NOT clean prose — the
                // rewrite silently stored receipt stubs (e.g. `{"ok":I'll
                // bootstrap Spindle…`) while reporting `status: saved`, throwing
                // away the real 15KB prose.
                //
                // Policy (v1): NO content-equality requirement against the
                // receipt. Because the companion adapter bug means receipts
                // routinely contain narration, an equality check would
                // false-reject every legitimate save. So the receipt validates
                // clearance + provenance only; `full_text` is left untouched.
                // If a future adapter fix makes receipt.output_text a faithful
                // copy of the prose, a mismatch check may be reintroduced — but
                // it MUST reject loudly (a structured error naming the mismatch),
                // never silently substitute the receipt.
                let receipt = self
                    .verified_explicit_save_receipt(input.generation_id.as_deref())
                    .await?;
                // One clearance authorizes ONE scene (migration V0034). The
                // claim is bound INSIDE the scene's save transaction below, so
                // it commits if and only if the save commits — a save that
                // fails (bad placement, constraint violation, claim conflict)
                // leaves the receipt unbound and reusable. The old standalone
                // claim ran before validation and burned the receipt on
                // `book 0, chapter 0` when the save then failed. Key the claim
                // on the RESOLVED placement, and reject a receipt already
                // spent on a different scene atomically with the write, so a
                // rejected replay still leaves no partial write behind.
                //
                // Without the claim, a single trip through the explicit-capable
                // agent blanket-authorized explicit saves across every other
                // scene — each stamped `agent:<id>` as though that agent had
                // written it. In a mixed-rating chapter that quietly undoes
                // per-scene routing: the one legitimately-cleared scene's
                // receipt gets spent on its General neighbours.
                let scene_key = explicit_receipt_scene_key(&save_input);
                // Pre-flight a receipt already spent on ANOTHER scene so the
                // rejection lands with the operator-facing message BEFORE any
                // write is attempted. This read is UX only — the binding
                // itself is enforced inside the save transaction below, where
                // a racing writer can never double-spend the receipt.
                match self.repository.get_generation_receipt(&receipt.id).await? {
                    Some(stored) => {
                        if let Some(claimed_key) = stored.claimed_scene_key.as_deref()
                            && claimed_key != scene_key
                        {
                            anyhow::bail!(
                                "generation_id {:?} already authorized a different scene ({}); \
                                 each explicit save needs its own receipt — call continue_generation \
                                 with route \"draft\" and rating \"explicit\" for this scene",
                                receipt.id,
                                crate::sqlite::repository::describe_receipt_scene_key(claimed_key)
                            );
                        }
                    }
                    None => anyhow::bail!(
                        "generation_id {:?} was not found or has expired",
                        receipt.id
                    ),
                }
                originating_model_call_ids = receipt.model_call_ids.clone();
                receipt_claim = Some((receipt.id.clone(), scene_key));
                draft_origin = Some(format!("agent:{}", receipt.agent_id));
            } else if crate::sqlite::import_service::contains_explicit_sexual_prose(
                &input.full_text,
            ) {
                anyhow::bail!(
                    "generation_id is required when saving explicit sexual prose; call continue_generation with route \"draft\" and rating \"explicit\", then pass the returned generation_id"
                );
            }
        } else if input
            .generation_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
        {
            // Non-explicit agent draft: a valid `draft`-route receipt at any
            // rating identifies the agent-draft seam. Stamp agent provenance so
            // an operator re-save is recognized as a style-contrast pair. The
            // client-supplied prose is authoritative here (only explicit prose is
            // server-held), so `full_text` is left untouched.
            let receipt = self
                .verified_revisable_draft_receipt(input.generation_id.as_deref())
                .await?;
            if let Some(receipt) = receipt {
                originating_model_call_ids = receipt.model_call_ids.clone();
                draft_origin = Some(format!("agent:{}", receipt.agent_id));
            }
        }

        let branch_id = self
            .repository
            .active_branch_id_public(&save_input.project_id)
            .await?;

        self.repository
            .check_active_run_scene_handoff_plan_revision(
                &save_input.project_id,
                &branch_id,
                save_input.book_number,
                save_input.chapter_number,
                save_input.scene_order,
            )
            .await?;

        // Style-learning capture (evolution §3.9): snapshot the scene's prior
        // state (provenance + prose) BEFORE this save overwrites it. Captured
        // after the save succeeds. `draft_origin.is_some()` here means THIS save
        // is itself an agent draft (the explicit-agent path stamped it above) —
        // agent-over-agent, never captured.
        let is_agent_save = draft_origin.is_some();
        let prior_scene = self
            .repository
            .find_scene_by_natural_key(
                &save_input.project_id,
                &branch_id,
                save_input.book_number,
                save_input.chapter_number,
                save_input.scene_order,
            )
            .await?;

        // A changed human draft ceases to be AI-authored even when learning is
        // disabled. Metadata-only saves retain the existing prose provenance.
        if !is_agent_save
            && prior_scene
                .as_ref()
                .is_none_or(|prior| prior.full_text != save_input.full_text)
        {
            draft_origin = Some("operator".to_string());
        }

        // Knowledge-learned package (design §2.3 path 2): validate every secret
        // link BEFORE any write so an invalid link fails the save atomically —
        // no scene row, no knowledge rows. `save_scene_draft` and
        // `upsert_knowledge_fact` are separate transactions, so this pre-flight
        // is what makes the package processing all-or-nothing. The validation is
        // identical to `record_knowledge`'s boundary check: the id must reference
        // an existing canonical fact that is actually marked secret.
        for entry in &save_input.knowledge_learned {
            // Character must exist and belong to this project (record_knowledge
            // enforces the same, but validating up front keeps the package
            // all-or-nothing).
            let character = self.repository.get_character(&entry.character_id).await?;
            if character.project_id != save_input.project_id {
                anyhow::bail!(
                    "knowledge_learned entry names character '{}', which does not belong to the requested project",
                    entry.character_id
                );
            }
            if let Some(fact_id) = entry.secret_of_fact_id.as_deref() {
                let linked = self
                    .repository
                    .get_canonical_fact(fact_id)
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!(
                            "secret_of_fact_id '{fact_id}' does not reference an existing canonical fact"
                        )
                    })?;
                if !linked.secret {
                    anyhow::bail!(
                        "secret_of_fact_id '{fact_id}' references a canonical fact that is not marked secret"
                    );
                }
            }
        }

        before_scene_write().await;

        let (mut scene, created) = match receipt_claim {
            Some((receipt_id, scene_key)) => {
                self.repository
                    .save_scene_draft_with_receipt_claim(
                        &save_input.project_id,
                        &branch_id,
                        &save_input,
                        &receipt_id,
                        &scene_key,
                    )
                    .await?
            }
            None => {
                self.repository
                    .save_scene_draft(&save_input.project_id, &branch_id, &save_input)
                    .await?
            }
        };
        if !originating_model_call_ids.is_empty() {
            let project_id = save_input.project_id.clone();
            let scene_id = scene.id.clone();
            let _ = self
                .repository
                .pool()
                .write(move |conn| {
                    for call_id in &originating_model_call_ids {
                        conn.execute(
                            "UPDATE model_call
                             SET scene_id = ?1,
                                 project_id = COALESCE(project_id, ?3),
                                 payload = json_set(
                                     payload,
                                     '$.scene_id', ?1,
                                     '$.project_id', COALESCE(project_id, ?3)
                                 )
                             WHERE id = ?2
                               AND (project_id = ?3 OR project_id IS NULL)
                               AND scene_id IS NULL",
                            rusqlite::params![scene_id, call_id, project_id],
                        )?;
                    }
                    Ok(())
                })
                .await;
        }

        // Cutover evidence metadata: preserve the accepted revision bytes/hash
        // so evidence-bound claims can be verified later. Best-effort and only
        // active during dual-write/shadow-read/canon-read.
        if !created || !scene.full_text.trim().is_empty() {
            let revision_id = format!(
                "scene-revision:{}:{}",
                scene.id,
                spindle_core::canon::exact_text_sha256(&scene.full_text)
            );
            if let Err(error) = self
                .repository
                .record_canon_scene_revision(
                    &save_input.project_id,
                    &branch_id,
                    &scene.id,
                    &revision_id,
                    &scene.full_text,
                )
                .await
            {
                tracing::warn!(error = %error, "canon scene revision source map failed");
            }
        }

        // Layered derived memory is derived, not canon: an edited scene marks
        // its scene summary and every ancestor stale and queues lazy recompute.
        if let Err(error) = self
            .invalidate_layered_memory_for_scene(
                &save_input.project_id,
                &branch_id,
                &scene.id,
                "scene_edit",
            )
            .await
        {
            tracing::warn!(error = %error, "layered memory invalidation failed");
        }

        // Record the knowledge_learned package (design §2.3 path 2). Each entry
        // is written through the SAME service path record_knowledge uses, stamped
        // with THIS scene's placement so the reveal cursor is the scene's own
        // position (flashback-safe). Secret links are already validated above.
        for entry in &save_input.knowledge_learned {
            self.record_knowledge(RecordKnowledgeInput {
                project_id: save_input.project_id.clone(),
                branch_id: Some(branch_id.clone()),
                character_id: entry.character_id.clone(),
                fact: entry.fact.clone(),
                source_summary: entry.source_summary.clone().unwrap_or_default(),
                learned_at: Some(spindle_core::models::StoryPlacement {
                    book_number: scene.book_number,
                    chapter_number: scene.chapter_number,
                    scene_order: Some(scene.scene_order),
                    note: None,
                }),
                confidence: None,
                tags: Vec::new(),
                reader_visible: entry.reader_visible.unwrap_or(true),
                secret_of_fact_id: entry.secret_of_fact_id.clone(),
            })
            .await?;
        }

        if !save_input.research_source_ids.is_empty()
            || !save_input.research_note_ids.is_empty()
            || !save_input.research_claim_ids.is_empty()
        {
            let run_id = self
                .repository
                .find_latest_authoring_run_id(&save_input.project_id)
                .await?
                .unwrap_or_else(|| "manual".to_string());

            let query_pack_input = save_input
                .research_query_pack_input
                .clone()
                .unwrap_or_else(|| "{}".to_string());

            let context_hash = save_input.research_context_hash.clone().unwrap_or_default();

            self.repository
                .create_research_usage(
                    &save_input.project_id,
                    &branch_id,
                    &run_id,
                    None,
                    &scene.id,
                    &save_input.research_source_ids,
                    &save_input.research_note_ids,
                    &save_input.research_claim_ids,
                    &query_pack_input,
                    &context_hash,
                )
                .await?;
        }

        if let Some(origin) = draft_origin.as_deref() {
            self.repository
                .update_scene_draft_origin(&scene.id, origin)
                .await?;
            scene = self.repository.get_scene(&scene.id).await?;
        }

        // Style-learning capture (evolution §3.9). Only an OPERATOR re-save over
        // an AGENT draft (or a subsequent operator edit of an already-captured
        // pair) is a style signal. Best-effort — a capture failure must never
        // fail the save.
        if !is_agent_save
            && !created
            && let Some(prior) = prior_scene.as_ref()
            && let Err(e) = self
                .maybe_capture_style_edit(&branch_id, prior, &scene, &save_input)
                .await
        {
            tracing::warn!(scene_id = %scene.id, error = %e, "style-edit capture failed");
        }

        // Match SurrealDB semantics: first save returns "saved", subsequent
        // writes to the same (project, branch, scene_order) return "updated".
        let status = if created { "saved" } else { "updated" };

        // Genre-voice gate: scan the saved prose + declared tone against the
        // project style contract. Best-effort — a missing/empty contract just
        // yields no warnings, and an assembly error must not fail the save.
        // The contemplative-ending check is left to check_consistency, which
        // has the chapter context to know whether this is the last scene.
        let style_directive = self
            .style_directive_for(&save_input.project_id, &branch_id)
            .await
            .unwrap_or_default();
        let style_hits = style_directive.scan(&spindle_core::style::StyleScanInput {
            prose: &scene.full_text,
            declared_tone: save_input.tone.as_deref(),
            is_chapter_end: false,
        });
        let tone_deviation = style_hits
            .iter()
            .any(|hit| hit.severity == spindle_core::style::StyleDriftSeverity::Warning);
        let style_warnings: Vec<String> = style_hits.into_iter().map(|hit| hit.message).collect();

        // Fiction anti-slop: attach the scan as advisory. Soft and hard hits
        // never fail this write. Hard over-limit fail-closes only when the
        // verify/revise loop is on (scene-scoped `anti_slop` check).
        // Phase 5: roll chapter-scoped quotas from earlier scenes and apply
        // learned suppressions. Experimental structural stays off unless
        // configured.
        let chapter_prior = chapter_counters_before(
            &self.repository,
            &save_input.project_id,
            &branch_id,
            scene.book_number,
            scene.chapter_number,
            scene.scene_order,
            Some(&scene.id),
        )
        .await;
        let suppressions =
            load_suppression_store(&self.repository, &save_input.project_id, &branch_id).await;
        let anti_slop =
            scan_scene_anti_slop_with(&scene.full_text, Some(&chapter_prior), Some(&suppressions));

        Ok(SaveSceneDraftOutput {
            scene_id: scene.id.clone(),
            status: status.to_string(),
            draft_origin: scene
                .draft_origin
                .clone()
                .unwrap_or_else(|| "agent".to_string()),
            pacing_warnings: Vec::new(),
            agency_warning: None,
            tone_deviation,
            style_warnings,
            content_rating_valid: true,
            content_rating_warnings: Vec::new(),
            diff: Vec::new(),
            byte_offsets_changed: Vec::new(),
            chars_added: scene.full_text.len(),
            chars_deleted: 0,
            world_rule_hits: Vec::new(),
            voice_drift: Vec::new(),
            retcon_findings: self
                .scan_retcon_findings(&save_input.project_id, &scene)
                .await?,
            // A freshly saved scene is not yet clocked, so the scan is prose-only
            // (no suppression to apply). Advisory `warning`s — never blocks a save.
            temporal_findings: scan_temporal_findings(&scene.id, &scene.full_text, None),
            anti_slop,
        })
    }
}
