use super::*;

impl Repository {
    // =========================================================================
    // Location + world state reads
    // =========================================================================

    pub async fn get_location(&self, id: &str) -> Result<Location> {
        let id = id.to_string();
        let loc = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {LOCATION_COLUMNS} FROM location WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Location::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("location not found"))?;
        Ok(loc)
    }

    pub async fn list_locations_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Location>> {
        list_branch_scoped(
            &self.inner.pool,
            LOCATION_COLUMNS,
            "location",
            "normalized_name",
            project_id,
            branch_id,
            |r| Location::try_from(r),
            |l| l.normalized_name.clone(),
        )
        .await
    }

    /// Create a location and its paired world_state row in one transaction.
    /// Returns both. Mirrors the SurrealDB repo's `(Location, WorldState)`
    /// return shape.
    pub async fn create_location(
        &self,
        input: &CreateLocationInput,
    ) -> Result<(Location, WorldState)> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let location_id = mint_id("location");
        let world_state_id = mint_id("world_state");
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let kind = input.kind.clone();
        let realm = input.realm.clone();
        let summary = input.summary.clone();
        let state = input.initial_state.clone();
        let sensory_json =
            serde_json::to_string(&state.sensory_details).context("serializing sensory_details")?;
        let now = timestamp_to_micros(chrono::Utc::now());
        let location_id_out = location_id.clone();
        let world_state_id_out = world_state_id.clone();

        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.savepoint()?;
                tx.execute(
                    "INSERT INTO location (id, project_id, branch_id, name, normalized_name, \
                     kind, realm, summary, notes, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?9)",
                    rusqlite::params![
                        &location_id,
                        &project_id,
                        &branch_id,
                        &name,
                        &normalized,
                        &kind,
                        &realm,
                        &summary,
                        now,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO world_state (id, project_id, branch_id, location_id, \
                     controlling_faction, status, prosperity, stability, threat_level, \
                     sensory_details, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        &world_state_id,
                        &project_id,
                        &branch_id,
                        &location_id,
                        &state.controlling_faction,
                        &state.status,
                        &state.prosperity,
                        &state.stability,
                        &state.threat_level,
                        &sensory_json,
                        now,
                    ],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;

        let location = self.get_location(&location_id_out).await?;
        let world_state = self
            .get_world_state_for_location(&location.branch_id, &location_id_out)
            .await?
            .ok_or_else(|| anyhow!("world state vanished after insert: {world_state_id_out}"))?;
        Ok((location, world_state))
    }

    pub async fn create_faction(&self, input: &CreateFactionInput) -> Result<Faction> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("faction");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let faction_type = input.faction_type.clone();
        let realm = input.realm.clone();
        let summary = input.summary.clone();
        let tags_json = serde_json::to_string(&input.tags).context("serializing tags")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO faction (id, project_id, branch_id, name, normalized_name, \
                     faction_type, realm, summary, tags, notes, archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, ?10, ?10)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &name, &normalized,
                        &faction_type, &realm, &summary, &tags_json, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_faction(&id_out).await
    }

    pub async fn get_faction(&self, id: &str) -> Result<Faction> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {FACTION_COLUMNS} FROM faction WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Faction::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("faction not found"))
    }

    pub async fn create_religion(&self, input: &CreateReligionInput) -> Result<Religion> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("religion");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let deity_or_principle = input.deity_or_principle.clone();
        let summary = input.summary.clone();
        let tags_json = serde_json::to_string(&input.tags).context("serializing tags")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO religion (id, project_id, branch_id, name, normalized_name, \
                     deity_or_principle, summary, tags, notes, archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?9)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &name, &normalized,
                        &deity_or_principle, &summary, &tags_json, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_religion(&id_out).await
    }

    pub async fn get_religion(&self, id: &str) -> Result<Religion> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {RELIGION_COLUMNS} FROM religion WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Religion::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("religion not found"))
    }

    pub async fn create_economy(&self, input: &CreateEconomyInput) -> Result<Economy> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("economy");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let realm = input.realm.clone();
        let summary = input.summary.clone();
        let scarce = serde_json::to_string(&input.scarce_resources)
            .context("serializing scarce_resources")?;
        let trade = serde_json::to_string(&input.trade_goods).context("serializing trade_goods")?;
        let currency = input.currency.clone();
        // The spindle-core CreateEconomyInput.notes is Vec<String> for the API,
        // but the SQLite economy.notes column is `TEXT NULL` for a free-form
        // human note. We serialize the vec to JSON and store it there to
        // preserve the existing semantics until the upstream contract is
        // reconciled.
        let notes_json = if input.notes.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&input.notes).context("serializing notes")?)
        };
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO economy (id, project_id, branch_id, name, normalized_name, \
                     realm, summary, scarce_resources, trade_goods, currency, notes, \
                     archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, ?12)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &name,
                        &normalized,
                        &realm,
                        &summary,
                        &scarce,
                        &trade,
                        &currency,
                        &notes_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_economy(&id_out).await
    }

    pub async fn get_economy(&self, id: &str) -> Result<Economy> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {ECONOMY_COLUMNS} FROM economy WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Economy::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("economy not found"))
    }

    pub async fn create_term(&self, input: &CreateTermInput) -> Result<Term> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("term");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let term_text = input.term_text.clone();
        let normalized = normalize_name(&input.term_text);
        let pronunciation = input.pronunciation.clone();
        let definition = input.definition.clone();
        let usage_context = input.usage_context.clone();
        let origin = input.origin.clone();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO term (id, project_id, branch_id, term_text, normalized_term, \
                     pronunciation, definition, usage_context, origin, notes, archived_at, \
                     created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, ?10, ?10)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &term_text,
                        &normalized,
                        &pronunciation,
                        &definition,
                        &usage_context,
                        &origin,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_term(&id_out).await
    }

    pub async fn get_term(&self, id: &str) -> Result<Term> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {TERM_COLUMNS} FROM term WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Term::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("term not found"))
    }

    // -------------------------------------------------------------------------
    // List methods for project+branch-scoped narrative entities.
    // All follow the same pattern: WHERE project_id = ?1 AND branch_id = ?2
    // ORDER BY normalized_name (or the entity's natural sort key).
    // -------------------------------------------------------------------------

    pub async fn list_factions_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Faction>> {
        list_branch_scoped(
            &self.inner.pool,
            FACTION_COLUMNS,
            "faction",
            "normalized_name",
            project_id,
            branch_id,
            |r| Faction::try_from(r),
            |f| f.normalized_name.clone(),
        )
        .await
    }

    pub async fn list_religions_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Religion>> {
        list_branch_scoped(
            &self.inner.pool,
            RELIGION_COLUMNS,
            "religion",
            "normalized_name",
            project_id,
            branch_id,
            |r| Religion::try_from(r),
            |rel| rel.normalized_name.clone(),
        )
        .await
    }

    pub async fn list_economies_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Economy>> {
        list_branch_scoped(
            &self.inner.pool,
            ECONOMY_COLUMNS,
            "economy",
            "normalized_name",
            project_id,
            branch_id,
            |r| Economy::try_from(r),
            |e| e.normalized_name.clone(),
        )
        .await
    }

    pub async fn list_terms_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Term>> {
        list_branch_scoped(
            &self.inner.pool,
            TERM_COLUMNS,
            "term",
            "normalized_term",
            project_id,
            branch_id,
            |r| Term::try_from(r),
            |t| t.normalized_term.clone(),
        )
        .await
    }

    // =========================================================================
    // World rule
    // =========================================================================

    pub async fn create_world_rule(&self, input: &CreateWorldRuleInput) -> Result<WorldRule> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("world_rule");
        let project_id = input.project_id.clone();
        let rule_name = input.rule_name.clone();
        let rule_type = input.rule_type.clone();
        let description = input.description.clone();
        let scan_pattern = input.scan_pattern.clone();
        let established: Option<StoredEstablishedIn> = input.established_in.clone().map(Into::into);
        let established_json = match established.as_ref() {
            Some(e) => Some(serde_json::to_string(e).context("serializing established_in")?),
            None => None,
        };
        let relevance_tags_json = if input.relevance_tags.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&input.relevance_tags)
                    .context("serializing relevance_tags")?,
            )
        };
        let now = timestamp_to_micros(chrono::Utc::now());

        let persisted_id = self
            .inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO world_rule (id, project_id, branch_id, rule_name, rule_type, \
                     description, established_in, relevance_tags, scan_pattern, notes, \
                     created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?10) \
                     ON CONFLICT(project_id, branch_id, rule_type, rule_name) DO UPDATE SET \
                        description = excluded.description, \
                        established_in = COALESCE(excluded.established_in, world_rule.established_in), \
                        relevance_tags = COALESCE(excluded.relevance_tags, world_rule.relevance_tags), \
                        scan_pattern = COALESCE(excluded.scan_pattern, world_rule.scan_pattern), \
                        updated_at = excluded.updated_at",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &rule_name,
                        &rule_type,
                        &description,
                        &established_json,
                        &relevance_tags_json,
                        &scan_pattern,
                        now,
                    ],
                )?;
                conn.query_row(
                    "SELECT id FROM world_rule \
                     WHERE project_id = ?1 AND branch_id = ?2 AND rule_type = ?3 AND rule_name = ?4",
                    rusqlite::params![&project_id, &branch_id, &rule_type, &rule_name],
                    |r| r.get::<_, String>(0),
                )
            })
            .await?;
        self.get_world_rule(&persisted_id).await
    }

    /// Re-home the given world rules onto `target_branch_id` so branch-local
    /// world-rule canon is not lost on merge. If the target branch already has
    /// a rule with the same `(project_id, rule_type, rule_name)`, replace it
    /// with the child branch's overriding row.
    pub async fn merge_world_rules_to_branch(
        &self,
        target_branch_id: &str,
        rule_ids: &[String],
    ) -> Result<usize> {
        if rule_ids.is_empty() {
            return Ok(0);
        }
        let target_branch_id = target_branch_id.to_string();
        let ids: Vec<String> = rule_ids.to_vec();
        let count = ids.len();
        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                let now = timestamp_to_micros(chrono::Utc::now());
                for id in &ids {
                    if let Some((project_id, rule_type, rule_name)) = tx
                        .query_row(
                            "SELECT project_id, rule_type, rule_name FROM world_rule WHERE id = ?1",
                            [id],
                            |r| {
                                Ok((
                                    r.get::<_, String>(0)?,
                                    r.get::<_, String>(1)?,
                                    r.get::<_, String>(2)?,
                                ))
                            },
                        )
                        .optional_inner()?
                    {
                        tx.execute(
                            "DELETE FROM world_rule \
                             WHERE project_id = ?1 AND branch_id = ?2 \
                               AND rule_type = ?3 AND rule_name = ?4 AND id != ?5",
                            rusqlite::params![
                                &project_id,
                                &target_branch_id,
                                &rule_type,
                                &rule_name,
                                id
                            ],
                        )?;
                    }
                    tx.execute(
                        "UPDATE world_rule SET branch_id = ?1, updated_at = ?2 WHERE id = ?3",
                        rusqlite::params![&target_branch_id, now, id],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        Ok(count)
    }

    pub async fn get_world_rule(&self, id: &str) -> Result<WorldRule> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {WORLD_RULE_COLUMNS} FROM world_rule WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| WorldRule::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("world_rule not found"))
    }

    pub async fn list_world_rules_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<WorldRule>> {
        list_branch_scoped(
            &self.inner.pool,
            WORLD_RULE_COLUMNS,
            "world_rule",
            "rule_type, rule_name",
            project_id,
            branch_id,
            |r| WorldRule::try_from(r),
            |w| (w.rule_type.clone(), w.rule_name.clone()),
        )
        .await
    }

    pub async fn create_system_overlay(
        &self,
        input: &CreateSystemOverlayInput,
    ) -> Result<SystemOverlay> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("system_overlay");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let system_name = input.system_name.clone();
        let normalized = normalize_name(&input.system_name);
        let system_type = input.system_type.clone();
        let rules = input.rules.clone();
        let visibility = input.visibility.clone();
        let progression_currency = input.progression_currency.clone();
        let stats_json = serde_json::to_string(&input.stats)?;
        let advancement_json = serde_json::to_string(&input.advancement_tiers)?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO system_overlay (id, project_id, branch_id, system_name, \
                     normalized_name, system_type, rules, visibility, progression_currency, \
                     stats, advancement_tiers, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &system_name,
                        &normalized,
                        &system_type,
                        &rules,
                        &visibility,
                        &progression_currency,
                        &stats_json,
                        &advancement_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_system_overlay(&id_out).await
    }

    pub async fn get_system_overlay(&self, id: &str) -> Result<SystemOverlay> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {SYSTEM_OVERLAY_COLUMNS} FROM system_overlay WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| SystemOverlay::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("system_overlay not found"))
    }

    pub async fn list_system_overlays_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<SystemOverlay>> {
        list_branch_scoped(
            &self.inner.pool,
            SYSTEM_OVERLAY_COLUMNS,
            "system_overlay",
            "normalized_name",
            project_id,
            branch_id,
            |r| SystemOverlay::try_from(r),
            |o| o.normalized_name.clone(),
        )
        .await
    }

    /// World state lookup that resolves through the project's active branch.
    /// Mirrors the SurrealDB repo's get_world_state_by_location convenience.
    pub async fn get_world_state_by_location(
        &self,
        location_id: &str,
    ) -> Result<Option<WorldState>> {
        let location = self.get_location(location_id).await?;
        let branch_id = self.active_branch_id(&location.project_id).await?;
        self.get_world_state_for_location(&branch_id, location_id)
            .await
    }

    pub async fn list_locations_by_project(&self, project_id: &str) -> Result<Vec<Location>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_locations_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_factions_by_project(&self, project_id: &str) -> Result<Vec<Faction>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_factions_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_religions_by_project(&self, project_id: &str) -> Result<Vec<Religion>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_religions_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_economies_by_project(&self, project_id: &str) -> Result<Vec<Economy>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_economies_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_terms_by_project(&self, project_id: &str) -> Result<Vec<Term>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_terms_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_world_rules_by_project(&self, project_id: &str) -> Result<Vec<WorldRule>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_world_rules_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_system_overlays_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<SystemOverlay>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_system_overlays_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn get_world_state_for_location(
        &self,
        branch_id: &str,
        location_id: &str,
    ) -> Result<Option<WorldState>> {
        let branch_id = branch_id.to_string();
        let location_id = location_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, None, &branch_id)?;
                let sql = format!(
                    "SELECT {WORLD_STATE_COLUMNS} FROM world_state \
                     WHERE branch_id = ?1 AND location_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                for candidate_branch_id in &chain {
                    if let Some(ws) = stmt
                        .query_row(rusqlite::params![candidate_branch_id, &location_id], |r| {
                            WorldState::try_from(r)
                        })
                        .optional_inner()?
                    {
                        return Ok(Some(ws));
                    }
                }
                Ok(None)
            })
            .await
    }

    /// List every world_state row on a branch (inheriting from ancestor branches).
    pub async fn list_world_states_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<WorldState>> {
        list_branch_scoped(
            &self.inner.pool,
            WORLD_STATE_COLUMNS,
            "world_state",
            "location_id",
            project_id,
            branch_id,
            |r| WorldState::try_from(r),
            |ws| ws.location_id.clone(),
        )
        .await
    }

    pub async fn list_all_locations(&self, project_id: &str) -> Result<Vec<Location>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {LOCATION_COLUMNS} FROM location \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Location::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_world_states(&self, project_id: &str) -> Result<Vec<WorldState>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {WORLD_STATE_COLUMNS} FROM world_state \
                     WHERE project_id = ?1 ORDER BY updated_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| WorldState::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_world_rules(&self, project_id: &str) -> Result<Vec<WorldRule>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {WORLD_RULE_COLUMNS} FROM world_rule \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| WorldRule::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_factions(&self, project_id: &str) -> Result<Vec<Faction>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {FACTION_COLUMNS} FROM faction \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Faction::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_religions(&self, project_id: &str) -> Result<Vec<Religion>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RELIGION_COLUMNS} FROM religion \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Religion::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_economies(&self, project_id: &str) -> Result<Vec<Economy>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {ECONOMY_COLUMNS} FROM economy \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Economy::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_terms(&self, project_id: &str) -> Result<Vec<Term>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {TERM_COLUMNS} FROM term \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Term::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_system_overlays(&self, project_id: &str) -> Result<Vec<SystemOverlay>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SYSTEM_OVERLAY_COLUMNS} FROM system_overlay \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| SystemOverlay::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn delete_world_rule(&self, project_id: &str, rule_id: &str) -> Result<bool> {
        let project_id = project_id.to_string();
        let project_id_for_delete = project_id.clone();
        let id = rule_id.to_string();
        let deleted = self
            .inner
            .pool
            .write(move |conn| {
                let affected = conn.execute(
                    "DELETE FROM world_rule WHERE id = ?1 AND project_id = ?2",
                    rusqlite::params![&id, &project_id_for_delete],
                )?;
                Ok(affected > 0)
            })
            .await?;
        if deleted {
            let _ = self
                .delete_search_embedding_for_entity(project_id.as_str(), rule_id)
                .await;
        }
        Ok(deleted)
    }
}
