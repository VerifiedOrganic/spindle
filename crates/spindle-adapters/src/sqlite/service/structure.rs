use super::*;

impl SqliteSpindleService {
    // =========================================================================
    // Book / chapter / save point / pacing / character_arc / timeline / system
    // overlay / temporal intervention / future knowledge / annotate beats /
    // plan chapter
    // =========================================================================

    pub async fn create_book(&self, input: CreateBookInput) -> Result<CreateBookOutput> {
        self.repository.get_project(&input.project_id).await?;
        let book = self
            .repository
            .create_book(&input.project_id, input.title)
            .await?;
        Ok(CreateBookOutput {
            book_id: book.id,
            book_number: book.book_number,
        })
    }

    pub async fn set_project_calendar(
        &self,
        input: spindle_core::models::SetProjectCalendarInput,
    ) -> Result<spindle_core::models::SetProjectCalendarOutput> {
        self.repository.get_project(&input.project_id).await?;
        input
            .calendar
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid calendar: {error}"))?;
        self.repository
            .upsert_project_calendar(&input.project_id, &input.calendar)
            .await?;
        Ok(spindle_core::models::SetProjectCalendarOutput {
            project_id: input.project_id,
        })
    }

    pub async fn set_scene_clock(
        &self,
        input: spindle_core::models::SetSceneClockInput,
    ) -> Result<spindle_core::models::SetSceneClockOutput> {
        let scene = self.repository.get_scene(&input.scene_id).await?;
        if scene.project_id != input.project_id {
            anyhow::bail!("scene does not belong to the requested project");
        }
        let calendar = self.story_validation_calendar(&input.project_id).await?;
        input
            .clock
            .validate(&calendar)
            .map_err(|error| anyhow::anyhow!("invalid story clock: {error}"))?;
        if let Some(mode) = input.temporal_mode.as_deref()
            && !matches!(mode, "linear" | "flashback" | "flashforward" | "concurrent")
        {
            anyhow::bail!(
                "invalid temporal_mode '{mode}' (expected linear|flashback|flashforward|concurrent)"
            );
        }
        self.repository
            .upsert_scene_clock(
                &input.scene_id,
                &input.project_id,
                &scene.branch_id,
                &input.clock,
                input.temporal_mode.as_deref(),
                input.thread_key.as_deref(),
            )
            .await?;
        Ok(spindle_core::models::SetSceneClockOutput {
            scene_id: input.scene_id,
        })
    }

    pub async fn set_timeline_event_clock(
        &self,
        input: spindle_core::models::SetTimelineEventClockInput,
    ) -> Result<spindle_core::models::SetTimelineEventClockOutput> {
        let event = self
            .repository
            .get_timeline_event(&input.timeline_event_id)
            .await?;
        if event.project_id != input.project_id {
            anyhow::bail!("timeline event does not belong to the requested project");
        }
        let calendar = self.story_validation_calendar(&input.project_id).await?;
        input
            .clock
            .validate(&calendar)
            .map_err(|error| anyhow::anyhow!("invalid story clock: {error}"))?;
        self.repository
            .upsert_timeline_event_clock(
                &input.timeline_event_id,
                &input.project_id,
                &event.branch_id,
                &input.clock,
            )
            .await?;
        Ok(spindle_core::models::SetTimelineEventClockOutput {
            timeline_event_id: input.timeline_event_id,
        })
    }

    pub async fn set_character_birth(
        &self,
        input: spindle_core::models::SetCharacterBirthInput,
    ) -> Result<spindle_core::models::SetCharacterBirthOutput> {
        let character = self.repository.get_character(&input.character_id).await?;
        if character.project_id != input.project_id {
            anyhow::bail!("character does not belong to the requested project");
        }
        let calendar = self.story_validation_calendar(&input.project_id).await?;
        input
            .clock
            .validate(&calendar)
            .map_err(|error| anyhow::anyhow!("invalid story clock: {error}"))?;
        self.repository
            .upsert_character_birth(&input.character_id, &input.project_id, &input.clock)
            .await?;
        Ok(spindle_core::models::SetCharacterBirthOutput {
            character_id: input.character_id,
        })
    }

    pub async fn set_project_quantity_scheme(
        &self,
        input: spindle_core::models::SetProjectQuantitySchemeInput,
    ) -> Result<spindle_core::models::SetProjectQuantitySchemeOutput> {
        self.repository.get_project(&input.project_id).await?;
        input
            .scheme
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid quantity scheme: {error}"))?;
        let branch = self.repository.get_active_branch(&input.project_id).await?;
        let measure = input.scheme.measure.clone();
        self.repository
            .upsert_project_quantity_scheme(&input.project_id, &branch.id, &input.scheme)
            .await?;
        Ok(spindle_core::models::SetProjectQuantitySchemeOutput {
            project_id: input.project_id,
            measure,
        })
    }

    pub async fn commit_quantity_state(
        &self,
        input: spindle_core::models::CommitQuantityStateInput,
    ) -> Result<spindle_core::models::CommitQuantityStateOutput> {
        self.repository.get_project(&input.project_id).await?;
        let branch = self.repository.get_active_branch(&input.project_id).await?;
        // Validate the reading against the declared scheme for this measure, if
        // one exists (band must be declared, amount non-negative).
        let scheme = self
            .repository
            .get_project_quantity_scheme(&input.project_id, &branch.id, &input.measure)
            .await?
            .map(|stored| stored.scheme);
        input
            .state
            .validate(scheme.as_ref())
            .map_err(|error| anyhow::anyhow!("invalid quantity state: {error}"))?;

        // Band-jump advisory (WarnOnly per design §9.5): a large unexplained tier
        // jump relative to the latest prior stamp is surfaced, not blocked.
        let mut warnings = Vec::new();
        if let Some(scheme) = scheme.as_ref()
            && let Some(new_band) = input.state.band.as_deref()
            && input.state.change_reason.is_none()
        {
            let cursor = input.book_number as i64 * 1_000_000
                + input.chapter_number as i64 * 1_000
                + input.scene_order as i64;
            if let Some(prior) = self
                .repository
                .latest_quantity_state_at_or_before(
                    &input.project_id,
                    &branch.id,
                    &input.subject_table,
                    &input.subject_id,
                    &input.measure,
                    cursor,
                )
                .await?
                && let Some(prior_band) = prior.state.band.as_deref()
                && let Some(jump) = scheme.band_jump(prior_band, new_band)
                && jump > scheme.max_band_jump_or_default()
            {
                warnings.push(format!(
                    "{} jumped {jump} tiers ({prior_band} → {new_band}) without a change_reason",
                    input.measure
                ));
            }
        }

        let stored = self
            .repository
            .append_quantity_state(crate::sqlite::repository::AppendQuantityStateParams {
                project_id: input.project_id,
                branch_id: branch.id,
                subject_table: input.subject_table,
                subject_id: input.subject_id,
                measure: input.measure,
                state: input.state,
                scene_id: input.scene_id,
                book_number: input.book_number,
                chapter_number: input.chapter_number,
                scene_order: input.scene_order,
            })
            .await?;
        Ok(spindle_core::models::CommitQuantityStateOutput {
            quantity_state_id: stored.id,
            warnings,
        })
    }

    /// Derive a quantity scheme from a system overlay so LitRPG / cultivation
    /// progression reuses the band-monotonicity machinery — the overlay's
    /// `advancement_tiers` become the scheme's ordered bands.
    pub async fn derive_quantity_scheme_from_system_overlay(
        &self,
        input: spindle_core::models::DeriveQuantitySchemeFromOverlayInput,
    ) -> Result<spindle_core::models::DeriveQuantitySchemeFromOverlayOutput> {
        use spindle_core::models::{QuantityBand, QuantityScheme};

        let overlay = self
            .repository
            .get_system_overlay(&input.system_overlay_id)
            .await?;
        if overlay.project_id != input.project_id {
            anyhow::bail!("system overlay does not belong to the requested project");
        }
        if overlay.advancement_tiers.is_empty() {
            anyhow::bail!(
                "system overlay '{}' has no advancement_tiers to derive bands from",
                overlay.system_name
            );
        }
        let measure = overlay
            .progression_currency
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| overlay.system_name.clone());
        let scheme = QuantityScheme {
            measure: measure.clone(),
            denominations: Vec::new(),
            bands: overlay
                .advancement_tiers
                .iter()
                .map(|tier| QuantityBand {
                    name: tier.clone(),
                    lower_bound: None,
                })
                .collect(),
            max_band_jump: Some(1),
        };
        scheme
            .validate()
            .map_err(|error| anyhow::anyhow!("derived quantity scheme invalid: {error}"))?;
        let branch = self.repository.get_active_branch(&input.project_id).await?;
        let band_count = scheme.bands.len();
        self.repository
            .upsert_project_quantity_scheme(&input.project_id, &branch.id, &scheme)
            .await?;
        Ok(
            spindle_core::models::DeriveQuantitySchemeFromOverlayOutput {
                measure,
                band_count,
            },
        )
    }

    /// Currency-unit vocabulary for prose price scanning: declared scheme
    /// denominations + economy currency names + a small default coinage set.
    pub(super) async fn project_currency_units(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<String>> {
        let mut units: std::collections::BTreeSet<String> = [
            "gold", "silver", "copper", "coin", "crown", "mark", "piece", "shilling", "penny",
        ]
        .iter()
        .map(|unit| unit.to_string())
        .collect();
        for stored in self
            .repository
            .list_project_quantity_schemes(project_id, branch_id)
            .await?
        {
            for denom in stored.scheme.denominations {
                units.insert(denom.name.to_ascii_lowercase());
            }
        }
        for economy in self
            .repository
            .list_economies_by_project_and_branch(project_id, branch_id)
            .await?
        {
            if let Some(currency) = economy.currency {
                units.insert(currency.to_ascii_lowercase());
            }
        }
        Ok(units.into_iter().collect())
    }

    /// Scan a scene's prose for "<number> <unit>" price mentions (review-gated;
    /// nothing is auto-registered). Powers price-canon seeding and affordability.
    pub async fn scan_scene_prices(
        &self,
        input: spindle_core::models::ScanScenePricesInput,
    ) -> Result<spindle_core::models::ScanScenePricesOutput> {
        let scene = self.repository.get_scene(&input.scene_id).await?;
        if scene.project_id != input.project_id {
            anyhow::bail!("scene does not belong to the requested project");
        }
        let branch = self.repository.get_active_branch(&input.project_id).await?;
        let units = self
            .project_currency_units(&input.project_id, &branch.id)
            .await?;
        let mentions = spindle_core::models::extract_price_mentions(&scene.full_text, &units);
        Ok(spindle_core::models::ScanScenePricesOutput { mentions })
    }

    /// The project's calendar if set, otherwise a default 24h Gregorian calendar
    /// used purely to validate clock ranges (e.g. `time_of_day`).
    pub(super) async fn story_validation_calendar(
        &self,
        project_id: &str,
    ) -> Result<spindle_core::models::CalendarDef> {
        Ok(self
            .repository
            .get_project_calendar(project_id)
            .await?
            .map(|stored| stored.calendar)
            .unwrap_or_else(|| spindle_core::models::CalendarDef {
                days_per_week: 7,
                hours_per_day: 24,
                week_day_names: Vec::new(),
                months: Vec::new(),
                days_per_year: 365,
                epoch_label: None,
            }))
    }

    pub async fn create_chapter(&self, input: CreateChapterInput) -> Result<CreateChapterOutput> {
        self.repository.get_project(&input.project_id).await?;
        // The book can come from (a) explicit book_id, (b) book_number, or
        // (c) defaulting to book 1. Mirror the SurrealDB service's resolution.
        let book = if let Some(book_id) = input.book_id.as_deref() {
            self.repository.get_book(book_id).await?
        } else {
            let book_number = input.book_number.unwrap_or(1);
            self.repository
                .ensure_book(&input.project_id, book_number)
                .await?
        };
        // Chapter number: explicit, else next-after-existing.
        let chapter_number = match input.chapter_number {
            Some(n) => n,
            None => {
                let existing = self.repository.list_chapters_by_book(&book.id).await?;
                existing.iter().map(|c| c.chapter_number).max().unwrap_or(0) + 1
            }
        };
        // Keep chapter numbers below the ordering radix so story-position packing
        // stays collision-free (see `format::CHAPTER_RADIX`).
        if !(0..crate::format::CHAPTER_RADIX as i32).contains(&chapter_number) {
            anyhow::bail!(
                "chapter_number {chapter_number} is out of range 0..{}; split into another book or raise CHAPTER_RADIX",
                crate::format::CHAPTER_RADIX
            );
        }
        let chapter = self
            .repository
            .ensure_chapter(&input.project_id, book.book_number, chapter_number)
            .await?;
        // Persist the title. The ensure path can return a PRE-EXISTING
        // untitled chapter (e.g. the book-1 chapter-1 row create_project
        // mints), so fill a missing title but never clobber one the operator
        // already set. The live bug silently dropped this field and
        // preflight later reported chapter_missing_title for a chapter that
        // was created with a title.
        let mut effective_title = chapter.title.clone();
        if let Some(title) = input
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            && chapter
                .title
                .as_deref()
                .map(str::trim)
                .is_none_or(str::is_empty)
        {
            self.repository
                .update_entity_field(
                    "chapter",
                    &chapter.id,
                    "title",
                    serde_json::Value::String(title.to_string()),
                )
                .await?;
            effective_title = Some(title.to_string());
        }
        Ok(CreateChapterOutput {
            chapter_id: chapter.id,
            book_number: chapter.book_number,
            chapter_number: chapter.chapter_number,
            title: effective_title,
        })
    }

    pub async fn create_save_point(
        &self,
        input: CreateSavePointInput,
    ) -> Result<CreateSavePointOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?
            .to_string();
        let branch = self.repository.get_branch(&branch_id).await?;
        let sp = self
            .repository
            .create_save_point(
                &input.project_id,
                &branch_id,
                &input.name,
                input.description.clone(),
            )
            .await?;

        // Persist the snapshot file + sha256/record_count metadata. Mirrors
        // `persist_save_point_snapshot` (`services/mod.rs:1618..1716` in
        // 705b835^). On any failure we delete the save_point row so the
        // database stays consistent with the on-disk view.
        if let Err(err) = self
            .persist_save_point_snapshot(&input.project_id, &branch.name, &sp.id, &input.name)
            .await
        {
            let _ = self.repository.delete_save_point(&sp.id).await;
            return Err(err);
        }

        Ok(CreateSavePointOutput {
            save_point_id: sp.id,
            branch_id: sp.branch_id,
        })
    }

    /// Walk the project, write the snapshot JSON to `data_dir/save-points/`,
    /// then update the save_point row with the file path + sha256 + record
    /// count metadata. Mirrors `persist_save_point_snapshot` in
    /// `services/mod.rs:1618..1716` in 705b835^.
    pub(super) async fn persist_save_point_snapshot(
        &self,
        project_id: &str,
        branch_name: &str,
        save_point_id: &str,
        save_point_name: &str,
    ) -> Result<()> {
        use std::collections::BTreeMap;

        const SAVE_POINT_SNAPSHOT_FORMAT: &str = "spindle-save-point-v1";

        let snapshot_relative_path = format!(
            "save-points/{}-{}-{}.json",
            slugify_filename_component(branch_name),
            slugify_filename_component(save_point_name),
            save_point_id.replace(':', "-"),
        );

        let mut extra_metadata: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        extra_metadata.insert(
            "save_point_id".to_string(),
            serde_json::Value::String(save_point_id.to_string()),
        );
        extra_metadata.insert(
            "save_point_name".to_string(),
            serde_json::Value::String(save_point_name.to_string()),
        );
        extra_metadata.insert(
            "branch_name".to_string(),
            serde_json::Value::String(branch_name.to_string()),
        );
        extra_metadata.insert(
            "snapshot_created_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );

        let artifact = self
            .build_project_export_payload(project_id, SAVE_POINT_SNAPSHOT_FORMAT, extra_metadata)
            .await?;
        let snapshot_bytes = serde_json::to_vec_pretty(&artifact.payload)?;
        let snapshot_sha256 = crate::sqlite::import::sha256_hex(&snapshot_bytes);

        let snapshot_path = self.repository.data_dir().join(&snapshot_relative_path);
        if let Some(parent) = snapshot_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&snapshot_path, &snapshot_bytes)?;

        if let Err(err) = self
            .repository
            .update_save_point_snapshot(
                save_point_id,
                &snapshot_relative_path,
                SAVE_POINT_SNAPSHOT_FORMAT,
                artifact.exported_records as i64,
                &snapshot_sha256,
            )
            .await
        {
            let _ = std::fs::remove_file(&snapshot_path);
            return Err(err);
        }

        Ok(())
    }

    pub async fn create_pacing_config(
        &self,
        input: CreatePacingConfigInput,
    ) -> Result<CreatePacingConfigOutput> {
        self.repository.get_project(&input.project_id).await?;
        let cfg = self.repository.create_pacing_config(&input).await?;
        Ok(CreatePacingConfigOutput {
            pacing_config_id: cfg.id,
        })
    }

    pub async fn create_pacing_curve(
        &self,
        input: CreatePacingCurveInput,
    ) -> Result<CreatePacingCurveOutput> {
        self.repository.get_project(&input.project_id).await?;
        let curve = self.repository.create_pacing_curve(&input).await?;
        Ok(CreatePacingCurveOutput {
            pacing_curve_id: curve.id,
        })
    }

    pub async fn create_character_arc(
        &self,
        input: CreateCharacterArcInput,
    ) -> Result<CreateCharacterArcOutput> {
        self.repository.get_project(&input.project_id).await?;
        let arc = self.repository.create_character_arc(&input).await?;
        // SurrealDB service spins up a paired pacing_tracker on arc creation;
        // mirror that here.
        let tracker = self
            .repository
            .create_pacing_tracker(&input.project_id, &arc.id)
            .await?;
        Ok(CreateCharacterArcOutput {
            character_arc_id: arc.id,
            pacing_tracker_id: tracker.id,
        })
    }

    pub async fn create_timeline_event(
        &self,
        input: CreateTimelineEventInput,
    ) -> Result<CreateTimelineEventOutput> {
        self.repository.get_project(&input.project_id).await?;
        let event = self.repository.create_timeline_event(&input).await?;
        self.resolve_phase_four_caches(
            &event.project_id,
            &event.branch_id,
            &[PhaseFourCacheId::RetconReachability],
        )
        .await?;
        Ok(CreateTimelineEventOutput {
            timeline_event_id: event.id,
        })
    }

    pub async fn create_temporal_intervention(
        &self,
        input: CreateTemporalInterventionInput,
    ) -> Result<CreateTemporalInterventionOutput> {
        self.repository.get_project(&input.project_id).await?;
        let intervention = self
            .repository
            .create_temporal_intervention(
                input.source_event_id.clone(),
                input.target_event_id.clone(),
                &input,
            )
            .await?;
        self.resolve_phase_four_caches(
            &intervention.project_id,
            &intervention.branch_id,
            &[PhaseFourCacheId::RetconReachability],
        )
        .await?;
        Ok(CreateTemporalInterventionOutput {
            temporal_intervention_id: intervention.id,
        })
    }

    pub async fn create_system_overlay(
        &self,
        input: CreateSystemOverlayInput,
    ) -> Result<CreateSystemOverlayOutput> {
        self.repository.get_project(&input.project_id).await?;
        let overlay = self.repository.create_system_overlay(&input).await?;
        Ok(CreateSystemOverlayOutput {
            system_overlay_id: overlay.id,
        })
    }

    pub async fn create_future_knowledge(
        &self,
        input: CreateFutureKnowledgeInput,
    ) -> Result<CreateFutureKnowledgeOutput> {
        self.repository.get_project(&input.project_id).await?;
        let fk = self.repository.create_future_knowledge(&input).await?;
        Ok(CreateFutureKnowledgeOutput {
            future_knowledge_id: fk.id,
        })
    }

    /// Annotate the beats on a scene — overwrites any existing annotation for
    /// the (branch, scene) pair.
    pub async fn annotate_scene_beats(
        &self,
        input: AnnotateSceneBeatsInput,
    ) -> Result<AnnotateSceneBeatsOutput> {
        let scene = self.repository.get_scene(&input.scene_id).await?;
        if scene.project_id != input.project_id {
            anyhow::bail!("scene does not belong to the requested project");
        }
        let beats: Vec<AnnotatedBeat> = input
            .beats
            .into_iter()
            .map(|b| AnnotatedBeat {
                beat_type: b.beat_type,
                summary: b.summary,
            })
            .collect();
        let annotation = self
            .repository
            .annotate_scene_beats(
                &input.project_id,
                &scene.branch_id,
                &scene.id,
                input.motif_ids,
                input.theme_ids,
                input.conflict_ids,
                beats,
                input.intensity,
            )
            .await?;
        Ok(AnnotateSceneBeatsOutput {
            scene_annotation_id: annotation.id,
        })
    }

    /// Plan or replan a chapter — upserts chapter_plan + chapter_outline.
    pub async fn plan_chapter(&self, input: PlanChapterInput) -> Result<PlanChapterOutput> {
        self.repository.get_project(&input.project_id).await?;

        // Validate the planned scenes' tone/beat descriptors against the style
        // contract BEFORE persisting, so genre drift is caught at planning time
        // (cheap) rather than after a chapter of prose is drafted (expensive).
        let branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        let style_warnings = match self
            .style_directive_for(&input.project_id, &branch_id)
            .await
        {
            Ok(directive) => validate_chapter_plan_style(&directive, &input.scenes),
            Err(_) => Vec::new(),
        };

        let plan = self.repository.plan_chapter(&input).await?;
        Ok(PlanChapterOutput {
            chapter_plan_id: plan.id,
            style_warnings,
        })
    }
}
