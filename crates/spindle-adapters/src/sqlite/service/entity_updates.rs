use super::*;

impl SqliteSpindleService {
    // =========================================================================
    // Generic update / archive / promise / revision_marker
    // =========================================================================

    pub async fn archive_entity(&self, input: ArchiveEntityInput) -> Result<ArchiveEntityOutput> {
        self.repository
            .archive_entity(&input.entity_type, &input.entity_id)
            .await?;
        Ok(ArchiveEntityOutput {
            entity_type: input.entity_type,
            entity_id: input.entity_id,
            archived: true,
        })
    }

    /// Generic field-merge update on a row identified by (entity_type, entity_id).
    /// The repository enforces a per-table column allowlist; unknown columns
    /// surface an error rather than silently being ignored.
    ///
    /// Character renames are the one field that does NOT go through the plain
    /// column path: a `name` change is routed through `rename_character`,
    /// which moves the normalized-name uniqueness key, keeps the old name as
    /// an alias, refreshes the search index, and reports every record still
    /// referencing the old name. The caller must opt in with
    /// `allow_rename: true`; without it the call is rejected before any
    /// write.
    pub async fn update_entity(&self, input: UpdateEntityInput) -> Result<UpdateEntityOutput> {
        let entity_type = input.entity_type.clone();
        let entity_id = input.entity_id.clone();
        let allow_rename = input.allow_rename.unwrap_or(false);
        let mut changes: std::collections::BTreeMap<String, serde_json::Value> = match input.changes
        {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            serde_json::Value::Null => std::collections::BTreeMap::new(),
            _ => anyhow::bail!("update_entity changes must be a JSON object"),
        };
        // Cache invalidation is computed over the FULL change set (the
        // character-name arm keys off changes containing "name"), before the
        // rename peels the name off.
        let cache_target = self
            .phase_four_cache_target_for_entity_update(&entity_type, &entity_id, &changes)
            .await?;
        // The emotional-profile columns only carry json_valid CHECKs, but the
        // row readers deserialize into typed shapes — a wrong-shaped value
        // would write fine and then break every later read of the profile
        // (briefings embed it). Validate against the stored types up front.
        if entity_type == "character_emotional_profile" {
            validate_emotional_profile_changes(&changes)?;
        }
        // `character.aliases` is the same class of hazard: the column only
        // checks json_valid, but the row reader deserializes Vec<String>.
        if entity_type == "character"
            && let Some(value) = changes.get("aliases")
        {
            validate_character_aliases_value(value)?;
        }
        // Reader-contract fields (`promise`, `style_notes`, `boundaries`, or a
        // whole `reader_contract` object) live in a single JSON column, so they
        // cannot go through the per-column allowlist path. Peel them off and
        // apply a validated partial merge through the dedicated repository
        // writer, then surface any style-surface contradictions introduced by
        // the change (e.g. a chapter word-count target that now disagrees with
        // the narrator voice or a style world rule).
        let mut warnings: Vec<String> = Vec::new();
        let mut reader_contract_updated = false;
        if entity_type == "project" {
            let contract_changes: std::collections::BTreeMap<String, serde_json::Value> =
                ["reader_contract", "promise", "style_notes", "boundaries"]
                    .into_iter()
                    .filter_map(|key| changes.remove(key).map(|value| (key.to_string(), value)))
                    .collect();
            if !contract_changes.is_empty() {
                self.update_project_reader_contract_fields(&entity_id, &contract_changes)
                    .await?;
                reader_contract_updated = true;
                warnings = self
                    .style_surface_contradiction_warnings(&entity_id)
                    .await?;
            }
        }
        let mut rename_report = None;
        if entity_type == "character"
            && let Some(name_value) = changes.remove("name")
        {
            let new_name = match name_value {
                serde_json::Value::String(name) => name,
                other => anyhow::bail!("character name must be a string, got {other}"),
            };
            rename_report = Some(
                self.rename_character(&entity_id, &new_name, allow_rename)
                    .await?,
            );
        }
        if !changes.is_empty() || (rename_report.is_none() && !reader_contract_updated) {
            self.repository
                .update_entity_fields(&entity_type, &entity_id, changes)
                .await?;
        }
        if let Some((project_id, branch_id, cache_ids)) = cache_target {
            match branch_id {
                Some(branch_id) => {
                    self.resolve_phase_four_caches(&project_id, &branch_id, &cache_ids)
                        .await?;
                }
                None => {
                    self.resolve_phase_four_caches_for_project(&project_id, &cache_ids)
                        .await?;
                }
            }
        }
        Ok(UpdateEntityOutput {
            entity_type,
            entity_id,
            rename_report,
            warnings,
        })
    }

    /// Partial-merge update of `project.reader_contract` — the only write path
    /// for the contract, since the update allowlist operates on flat columns
    /// while the contract is a single JSON column. Provided fields are
    /// validated against the stored shape BEFORE being merged into the
    /// existing contract, so a wrong-shaped value cannot corrupt the row
    /// (the column only carries a `json_valid` CHECK). Fields not mentioned
    /// are preserved, so a `style_notes`-only edit never wipes `promise`.
    pub(super) async fn update_project_reader_contract_fields(
        &self,
        project_id: &str,
        changes: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<()> {
        let project = self.repository.get_project(project_id).await?;
        let mut contract = project.reader_contract.into_core();

        // A nested `reader_contract` object merges field-by-field (only the
        // keys actually present), then top-level scalar/array keys override.
        if let Some(value) = changes.get("reader_contract") {
            let obj = value
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("reader_contract must be a JSON object"))?;
            if let Some(promise) = obj.get("promise") {
                contract.promise = parse_contract_string("reader_contract.promise", promise)?;
            }
            if let Some(style_notes) = obj.get("style_notes") {
                contract.style_notes =
                    parse_contract_string_list("reader_contract.style_notes", style_notes)?;
            }
            if let Some(boundaries) = obj.get("boundaries") {
                contract.boundaries =
                    parse_contract_string_list("reader_contract.boundaries", boundaries)?;
            }
        }
        if let Some(value) = changes.get("promise") {
            contract.promise = parse_contract_string("promise", value)?;
        }
        if let Some(value) = changes.get("style_notes") {
            contract.style_notes = parse_contract_string_list("style_notes", value)?;
        }
        if let Some(value) = changes.get("boundaries") {
            contract.boundaries = parse_contract_string_list("boundaries", value)?;
        }

        self.repository
            .update_project_reader_contract(project_id, &contract)
            .await
    }

    /// Cross-check the hard numbers — currently chapter/scene word-count
    /// targets — across the three style surfaces drafting reads:
    /// `reader_contract.style_notes`, the narrator voice, and style world
    /// rules. Returns one warning per pair of surfaces whose targets disagree.
    /// This is the guard against the stale-contract failure where one surface
    /// is updated and another still carries the superseded figure, so chapters
    /// get drafted against the wrong number. Advisory, never blocking.
    pub(super) async fn style_surface_contradiction_warnings(
        &self,
        project_id: &str,
    ) -> Result<Vec<String>> {
        let project = self.repository.get_project(project_id).await?;
        let contract = project.reader_contract.into_core();
        let narrator_voice = project
            .narrator_voice
            .map(|stored| stored.into_core())
            .unwrap_or_default();

        // Collect (surface label, word-count ranges) for every surface that
        // carries at least one range.
        let mut surfaces: Vec<(String, Vec<(i64, i64)>)> = Vec::new();

        let mut contract_ranges: Vec<(i64, i64)> = Vec::new();
        for note in &contract.style_notes {
            contract_ranges.extend(extract_word_count_ranges(note));
        }
        if !contract_ranges.is_empty() {
            surfaces.push(("reader_contract.style_notes".to_string(), contract_ranges));
        }

        let mut voice_ranges: Vec<(i64, i64)> = Vec::new();
        for text in narrator_voice_text_fields(&narrator_voice) {
            voice_ranges.extend(extract_word_count_ranges(&text));
        }
        if !voice_ranges.is_empty() {
            surfaces.push(("narrator_voice".to_string(), voice_ranges));
        }

        let branch_id = self.repository.active_branch_id_public(project_id).await?;
        let rules = self
            .repository
            .list_world_rules_by_project_and_branch(project_id, &branch_id)
            .await?;
        for rule in rules.iter().filter(|r| r.rule_type == "style") {
            let ranges = extract_word_count_ranges(&rule.description);
            if !ranges.is_empty() {
                surfaces.push((format!("world_rule '{}'", rule.rule_name), ranges));
            }
        }

        // Warn for every pair of surfaces with non-overlapping targets.
        let mut warnings = Vec::new();
        for i in 0..surfaces.len() {
            for j in (i + 1)..surfaces.len() {
                let (name_a, ranges_a) = &surfaces[i];
                let (name_b, ranges_b) = &surfaces[j];
                if word_ranges_contradict(ranges_a, ranges_b) {
                    warnings.push(format!(
                        "style-surface contradiction: {} targets {} but {} targets {}; \
                         update the stale surface so drafting reads one consistent figure",
                        name_a,
                        format_word_ranges(ranges_a),
                        name_b,
                        format_word_ranges(ranges_b)
                    ));
                }
            }
        }
        Ok(warnings)
    }

    /// Controlled character rename (the engine behind `update_entity` with
    /// `changes: {"name": ...}` + `allow_rename: true`). Validates the new
    /// name, refuses without the explicit flag, then moves the name +
    /// normalized uniqueness key, keeps the old name as an alias, refreshes
    /// the search embedding, and reports stale references. Scene prose is
    /// never rewritten — the report exists so the author can decide which
    /// mentions follow the rename and which are historical.
    pub(super) async fn rename_character(
        &self,
        character_id: &str,
        new_name: &str,
        allow_rename: bool,
    ) -> Result<RenameReport> {
        let character = self.repository.get_character(character_id).await?;
        let old_name = character.name.clone();
        let new_name = new_name.trim();
        if new_name.is_empty() {
            anyhow::bail!("character name must not be empty");
        }
        if !allow_rename {
            anyhow::bail!(
                "renaming a character requires allow_rename: true on update_entity: \
                 the rename moves the name's uniqueness key, keeps '{old_name}' as an alias, \
                 refreshes the search index, and reports records still referencing the old name. \
                 To add a name WITHOUT renaming, set the character's aliases field instead"
            );
        }
        let (updated, old_name_kept_as_alias) = self
            .repository
            .rename_character(character_id, new_name)
            .await?;
        // FTS syncs by trigger; the embedding needs an explicit refresh so
        // the semantic path stops returning the old name.
        let document = character_search_document(&updated);
        self.repository
            .refresh_search_embedding_for_entity(
                &updated.project_id,
                &updated.branch_id,
                &updated.id,
                &document,
            )
            .await?;
        // Records still referencing the OLD name. Relationships and character
        // states link by id and are unaffected; scenes, facts, knowledge, and
        // arc prose are scanned and reported, never rewritten.
        let scenes_referencing_old_name = self
            .repository
            .list_scene_ids_mentioning_phrase(
                &updated.project_id,
                &updated.branch_id,
                &old_name,
                200,
            )
            .await?;
        let canonical_facts_referencing_old_name = self
            .repository
            .list_canonical_fact_ids_mentioning(&updated.project_id, &updated.branch_id, &old_name)
            .await?;
        let knowledge_facts_referencing_old_name = self
            .repository
            .list_knowledge_fact_ids_mentioning(&updated.project_id, &updated.branch_id, &old_name)
            .await?;
        let arcs_referencing_old_name = self
            .repository
            .list_character_arc_ids_mentioning(
                &updated.project_id,
                &updated.branch_id,
                &updated.id,
                &old_name,
            )
            .await?;
        Ok(RenameReport {
            character_id: updated.id.clone(),
            old_name,
            new_name: updated.name.clone(),
            old_name_kept_as_alias,
            scenes_referencing_old_name,
            canonical_facts_referencing_old_name,
            knowledge_facts_referencing_old_name,
            arcs_referencing_old_name,
        })
    }

    pub async fn update_world_rule(
        &self,
        input: UpdateWorldRuleInput,
    ) -> Result<UpdateWorldRuleOutput> {
        let before = self.repository.get_world_rule(&input.world_rule_id).await?;
        let changes: std::collections::BTreeMap<String, serde_json::Value> = match input.changes {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            _ => anyhow::bail!("update_world_rule changes must be a JSON object"),
        };
        let mut cache_ids = vec![PhaseFourCacheId::WorldRuleSemanticDrift];
        let style_relevant = before.rule_type.eq_ignore_ascii_case("style")
            || changes
                .get("rule_type")
                .and_then(|value| value.as_str())
                .is_some_and(|rule_type| rule_type.eq_ignore_ascii_case("style"));
        if style_relevant {
            cache_ids.push(PhaseFourCacheId::StyleCompliance);
        }
        self.repository
            .update_entity_fields("world_rule", &input.world_rule_id, changes)
            .await?;
        self.resolve_phase_four_caches(&before.project_id, &before.branch_id, &cache_ids)
            .await?;
        // A style world rule is one of the surfaces drafting reads for hard
        // numbers; warn if it now contradicts reader_contract.style_notes or
        // the narrator voice on a target like chapter length.
        let warnings = if style_relevant {
            self.style_surface_contradiction_warnings(&before.project_id)
                .await?
        } else {
            Vec::new()
        };
        Ok(UpdateWorldRuleOutput {
            world_rule_id: input.world_rule_id,
            warnings,
        })
    }

    pub async fn update_promise_status(
        &self,
        input: UpdatePromiseStatusInput,
    ) -> Result<UpdatePromiseStatusOutput> {
        self.repository.update_promise_status(&input).await?;
        let status = self
            .repository
            .get_narrative_promise(&input.narrative_promise_id)
            .await?
            .status;
        Ok(UpdatePromiseStatusOutput {
            narrative_promise_id: input.narrative_promise_id,
            status,
        })
    }

    /// Targeted single-milestone update for a character arc, addressed by
    /// milestone label. Reads the arc, patches only the supplied fields on
    /// the matching milestone, and writes the full milestones array back
    /// through the allowlisted `character_arc.milestones` column — so the
    /// caller never has to resend milestone fields (description, unlocks)
    /// it may not be able to read.
    pub async fn update_arc_milestone(
        &self,
        input: UpdateArcMilestoneInput,
    ) -> Result<UpdateArcMilestoneOutput> {
        if input.placement.is_none() && input.reached_at.is_none() {
            anyhow::bail!(
                "update_arc_milestone requires placement and/or reached_at; \
                 omitting both would be a no-op"
            );
        }
        let label = input.label.trim();
        if label.is_empty() {
            anyhow::bail!("update_arc_milestone requires a non-empty label");
        }
        let arc = self.repository.get_character_arc(&input.arc_id).await?;
        let mut milestones = arc.milestones.clone();
        let milestone = milestones
            .iter_mut()
            .find(|milestone| milestone.label == label)
            .ok_or_else(|| {
                let available = arc
                    .milestones
                    .iter()
                    .map(|milestone| milestone.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                anyhow::anyhow!(
                    "no milestone labelled '{label}' on arc {}; available labels: {available}",
                    input.arc_id
                )
            })?;
        if let Some(placement) = input.placement {
            milestone.placement = Some(crate::sqlite::json_records::StoredStoryPlacement::from(
                placement,
            ));
        }
        if let Some(reached_at) = input.reached_at {
            milestone.reached_at = Some(crate::sqlite::json_records::StoredStoryPlacement::from(
                reached_at,
            ));
        }
        let updated = milestone.clone();
        let milestones_json = serde_json::to_value(&milestones)?;
        self.repository
            .update_entity_field("character_arc", &arc.id, "milestones", milestones_json)
            .await?;
        Ok(UpdateArcMilestoneOutput {
            arc_id: arc.id,
            milestone: updated.into_core(),
        })
    }

    pub async fn resolve_revision_marker(
        &self,
        input: ResolveRevisionMarkerInput,
    ) -> Result<ResolveRevisionMarkerOutput> {
        let marker = self
            .repository
            .resolve_revision_marker(&input.marker_id)
            .await?;
        Ok(ResolveRevisionMarkerOutput {
            marker_id: marker.id,
            status: marker.status,
        })
    }
}
