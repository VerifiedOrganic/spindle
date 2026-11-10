use super::*;

impl Repository {
    // ── Story-time side tables (V0017) ──────────────────────────────

    pub async fn upsert_project_calendar(
        &self,
        project_id: &str,
        calendar: &spindle_core::models::CalendarDef,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let week_day_names = serde_json::to_string(&calendar.week_day_names)?;
        let months = serde_json::to_string(&calendar.months)?;
        let days_per_week = calendar.days_per_week;
        let hours_per_day = calendar.hours_per_day;
        let days_per_year = calendar.days_per_year;
        let epoch_label = calendar.epoch_label.clone();
        let leap_rule = serde_json::to_string(&calendar.leap_rule)?;
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                conn.execute(
                    "INSERT INTO project_calendar \
                     (project_id, days_per_week, hours_per_day, week_day_names, months, \
                      days_per_year, epoch_label, created_at, updated_at, leap_rule) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9) \
                     ON CONFLICT(project_id) DO UPDATE SET \
                       days_per_week = excluded.days_per_week, \
                       hours_per_day = excluded.hours_per_day, \
                       week_day_names = excluded.week_day_names, \
                       months = excluded.months, \
                       days_per_year = excluded.days_per_year, \
                       epoch_label = excluded.epoch_label, \
                       updated_at = excluded.updated_at, \
                       leap_rule = excluded.leap_rule",
                    rusqlite::params![
                        &project_id,
                        days_per_week,
                        hours_per_day,
                        &week_day_names,
                        &months,
                        days_per_year,
                        &epoch_label,
                        now,
                        &leap_rule,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn get_project_calendar(
        &self,
        project_id: &str,
    ) -> Result<Option<crate::sqlite::records::StoredProjectCalendar>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM project_calendar WHERE project_id = ?1",
                    crate::sqlite::records::PROJECT_CALENDAR_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&project_id], |r| {
                    crate::sqlite::records::StoredProjectCalendar::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn upsert_scene_clock(
        &self,
        scene_id: &str,
        project_id: &str,
        branch_id: &str,
        clock: &spindle_core::models::StoryClock,
        temporal_mode: Option<&str>,
        thread_key: Option<&str>,
    ) -> Result<()> {
        let scene_id = scene_id.to_string();
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let day_index = clock.day_index;
        let time_of_day = clock.time_of_day;
        let duration_days = clock.duration_days;
        let precision = clock.precision.clone();
        let temporal_mode = temporal_mode.map(str::to_string);
        let thread_key = thread_key.map(str::to_string);
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                conn.execute(
                    "INSERT INTO scene_clock \
                     (scene_id, project_id, branch_id, day_index, time_of_day, duration_days, \
                      precision, temporal_mode, thread_key, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10) \
                     ON CONFLICT(scene_id) DO UPDATE SET \
                       project_id = excluded.project_id, \
                       branch_id = excluded.branch_id, \
                       day_index = excluded.day_index, \
                       time_of_day = excluded.time_of_day, \
                       duration_days = excluded.duration_days, \
                       precision = excluded.precision, \
                       temporal_mode = excluded.temporal_mode, \
                       thread_key = excluded.thread_key, \
                       updated_at = excluded.updated_at",
                    rusqlite::params![
                        &scene_id,
                        &project_id,
                        &branch_id,
                        day_index,
                        time_of_day,
                        duration_days,
                        &precision,
                        &temporal_mode,
                        &thread_key,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn get_scene_clock(
        &self,
        scene_id: &str,
    ) -> Result<Option<crate::sqlite::records::StoredSceneClock>> {
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM scene_clock WHERE scene_id = ?1",
                    crate::sqlite::records::SCENE_CLOCK_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&scene_id], |r| {
                    crate::sqlite::records::StoredSceneClock::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn list_scene_clocks_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<crate::sqlite::records::StoredSceneClock>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM scene_clock WHERE project_id = ?1 AND branch_id = ?2",
                    crate::sqlite::records::SCENE_CLOCK_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id, &branch_id], |r| {
                        crate::sqlite::records::StoredSceneClock::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// The most recent dated scene clock at or before `cursor_index` (a packed
    /// `format::story_index` value) on the branch. Used to surface the current
    /// in-world time when assembling drafting context.
    pub async fn latest_scene_clock_at_or_before(
        &self,
        project_id: &str,
        branch_id: &str,
        cursor_index: i64,
    ) -> Result<Option<crate::sqlite::records::StoredSceneClock>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                // Same packing as `format::story_index` (book*1_000_000 + chapter*1_000 + scene_order).
                let position =
                    "(s.book_number * 1000000 + s.chapter_number * 1000 + s.scene_order)";
                let sql = format!(
                    "SELECT sc.scene_id, sc.project_id, sc.branch_id, sc.day_index, sc.time_of_day, \
                            sc.duration_days, sc.precision, sc.temporal_mode, sc.thread_key, \
                            sc.created_at, sc.updated_at \
                     FROM scene_clock sc JOIN scene s ON s.id = sc.scene_id \
                     WHERE sc.project_id = ?1 AND sc.branch_id = ?2 AND sc.day_index IS NOT NULL \
                       AND {position} <= ?3 \
                     ORDER BY {position} DESC LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![&project_id, &branch_id, cursor_index],
                    |r| crate::sqlite::records::StoredSceneClock::try_from(r),
                )
                .optional_inner()
            })
            .await
    }

    pub async fn upsert_timeline_event_clock(
        &self,
        timeline_event_id: &str,
        project_id: &str,
        branch_id: &str,
        clock: &spindle_core::models::StoryClock,
    ) -> Result<()> {
        let timeline_event_id = timeline_event_id.to_string();
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let day_index = clock.day_index;
        let time_of_day = clock.time_of_day;
        let duration_days = clock.duration_days;
        let precision = clock.precision.clone();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                conn.execute(
                    "INSERT INTO timeline_event_clock \
                     (timeline_event_id, project_id, branch_id, day_index, time_of_day, \
                      duration_days, precision, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8) \
                     ON CONFLICT(timeline_event_id) DO UPDATE SET \
                       project_id = excluded.project_id, \
                       branch_id = excluded.branch_id, \
                       day_index = excluded.day_index, \
                       time_of_day = excluded.time_of_day, \
                       duration_days = excluded.duration_days, \
                       precision = excluded.precision, \
                       updated_at = excluded.updated_at",
                    rusqlite::params![
                        &timeline_event_id,
                        &project_id,
                        &branch_id,
                        day_index,
                        time_of_day,
                        duration_days,
                        &precision,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn get_timeline_event_clock(
        &self,
        timeline_event_id: &str,
    ) -> Result<Option<crate::sqlite::records::StoredTimelineEventClock>> {
        let timeline_event_id = timeline_event_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM timeline_event_clock WHERE timeline_event_id = ?1",
                    crate::sqlite::records::TIMELINE_EVENT_CLOCK_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&timeline_event_id], |r| {
                    crate::sqlite::records::StoredTimelineEventClock::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn upsert_character_birth(
        &self,
        character_id: &str,
        project_id: &str,
        clock: &spindle_core::models::StoryClock,
    ) -> Result<()> {
        let character_id = character_id.to_string();
        let project_id = project_id.to_string();
        let day_index = clock.day_index;
        let time_of_day = clock.time_of_day;
        let precision = clock.precision.clone();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                conn.execute(
                    "INSERT INTO character_birth \
                     (character_id, project_id, birth_day_index, time_of_day, precision, \
                      created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) \
                     ON CONFLICT(character_id) DO UPDATE SET \
                       project_id = excluded.project_id, \
                       birth_day_index = excluded.birth_day_index, \
                       time_of_day = excluded.time_of_day, \
                       precision = excluded.precision, \
                       updated_at = excluded.updated_at",
                    rusqlite::params![
                        &character_id,
                        &project_id,
                        day_index,
                        time_of_day,
                        &precision,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn get_character_birth(
        &self,
        character_id: &str,
    ) -> Result<Option<crate::sqlite::records::StoredCharacterBirth>> {
        let character_id = character_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM character_birth WHERE character_id = ?1",
                    crate::sqlite::records::CHARACTER_BIRTH_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&character_id], |r| {
                    crate::sqlite::records::StoredCharacterBirth::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    // =========================================================================
    // Timeline, temporal interventions, system overlays
    // =========================================================================

    pub async fn create_timeline_event(
        &self,
        input: &CreateTimelineEventInput,
    ) -> Result<TimelineEvent> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("timeline_event");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let title = input.title.clone();
        let event_type = input.event_type.clone();
        let placement: StoredStoryPlacement = input.placement.clone().into();
        let placement_json = serde_json::to_string(&placement)?;
        let summary = input.summary.clone();
        let related_json = serde_json::to_string(&input.related_entity_ids)?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO timeline_event (id, project_id, branch_id, title, event_type, \
                     placement, summary, related_entity_ids, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &title,
                        &event_type,
                        &placement_json,
                        &summary,
                        &related_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_timeline_event(&id_out).await
    }

    pub async fn get_timeline_event(&self, id: &str) -> Result<TimelineEvent> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {TIMELINE_EVENT_COLUMNS} FROM timeline_event WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| TimelineEvent::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("timeline_event not found"))
    }

    pub async fn list_timeline_events_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<TimelineEvent>> {
        list_branch_scoped(
            &self.inner.pool,
            TIMELINE_EVENT_COLUMNS,
            "timeline_event",
            "title",
            project_id,
            branch_id,
            |r| TimelineEvent::try_from(r),
            |e| e.title.clone(),
        )
        .await
    }

    pub async fn create_temporal_intervention(
        &self,
        source_event_id: Option<String>,
        target_event_id: Option<String>,
        input: &CreateTemporalInterventionInput,
    ) -> Result<TemporalIntervention> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("temporal_intervention");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let title = input.title.clone();
        let intervention_type = input.intervention_type.clone();
        let summary = input.summary.clone();
        let consequences_json = serde_json::to_string(&input.consequences)?;
        let status = input
            .status
            .clone()
            .unwrap_or_else(|| "planned".to_string());
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO temporal_intervention (id, project_id, branch_id, title, \
                     intervention_type, source_event_id, target_event_id, summary, consequences, \
                     status, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &title,
                        &intervention_type,
                        &source_event_id,
                        &target_event_id,
                        &summary,
                        &consequences_json,
                        &status,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_temporal_intervention(&id_out).await
    }

    pub async fn get_temporal_intervention(&self, id: &str) -> Result<TemporalIntervention> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {TEMPORAL_INTERVENTION_COLUMNS} FROM temporal_intervention WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| TemporalIntervention::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("temporal_intervention not found"))
    }

    pub async fn list_temporal_interventions_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<TemporalIntervention>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_temporal_interventions_by_project_and_branch(project_id, &branch_id)
            .await
    }

    /// Explicit-branch variant for the Phase-4 retcon scanner, which resolves
    /// along the branch parent chain up to `main`.
    pub async fn list_temporal_interventions_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<TemporalIntervention>> {
        list_branch_scoped(
            &self.inner.pool,
            TEMPORAL_INTERVENTION_COLUMNS,
            "temporal_intervention",
            "title",
            project_id,
            branch_id,
            |r| TemporalIntervention::try_from(r),
            |t| t.title.clone(),
        )
        .await
    }

    pub async fn list_timeline_events_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<TimelineEvent>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_timeline_events_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_all_timeline_events(&self, project_id: &str) -> Result<Vec<TimelineEvent>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {TIMELINE_EVENT_COLUMNS} FROM timeline_event \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| TimelineEvent::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_temporal_interventions(
        &self,
        project_id: &str,
    ) -> Result<Vec<TemporalIntervention>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {TEMPORAL_INTERVENTION_COLUMNS} FROM temporal_intervention \
                     WHERE project_id = ?1 ORDER BY title"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| TemporalIntervention::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
