use super::*;

impl SqliteSpindleService {
    // =========================================================================
    // Plain narrative-entity create tools (faction/religion/economy/term/
    // plot_line/conflict/theme/motif/narrative_promise/world_rule)
    // =========================================================================
    //
    // These all follow the SurrealDB service shape: validate project, call
    // repo.create_X (which resolves the active branch internally), refresh
    // the search embedding when the entity has searchable text. Each output
    // is a single-id record.

    pub async fn create_faction(&self, input: CreateFactionInput) -> Result<CreateFactionOutput> {
        self.repository.get_project(&input.project_id).await?;
        let faction = self.repository.create_faction(&input).await?;
        self.refresh_entity_index(
            &faction.project_id,
            &faction.branch_id,
            &faction.id,
            "faction",
            &faction.name,
            &faction.summary,
        )
        .await?;
        Ok(CreateFactionOutput {
            faction_id: faction.id,
        })
    }

    pub async fn create_religion(
        &self,
        input: CreateReligionInput,
    ) -> Result<CreateReligionOutput> {
        self.repository.get_project(&input.project_id).await?;
        let religion = self.repository.create_religion(&input).await?;
        self.refresh_entity_index(
            &religion.project_id,
            &religion.branch_id,
            &religion.id,
            "religion",
            &religion.name,
            &religion.summary,
        )
        .await?;
        Ok(CreateReligionOutput {
            religion_id: religion.id,
        })
    }

    pub async fn create_economy(&self, input: CreateEconomyInput) -> Result<CreateEconomyOutput> {
        self.repository.get_project(&input.project_id).await?;
        let economy = self.repository.create_economy(&input).await?;
        self.refresh_entity_index(
            &economy.project_id,
            &economy.branch_id,
            &economy.id,
            "economy",
            &economy.name,
            &economy.summary,
        )
        .await?;
        Ok(CreateEconomyOutput {
            economy_id: economy.id,
        })
    }

    pub async fn create_term(&self, input: CreateTermInput) -> Result<CreateTermOutput> {
        self.repository.get_project(&input.project_id).await?;
        let term = self.repository.create_term(&input).await?;
        self.refresh_entity_index(
            &term.project_id,
            &term.branch_id,
            &term.id,
            "term",
            &term.term_text,
            &term.definition,
        )
        .await?;
        Ok(CreateTermOutput { term_id: term.id })
    }

    pub async fn create_plot_line(
        &self,
        input: CreatePlotLineInput,
    ) -> Result<CreatePlotLineOutput> {
        self.repository.get_project(&input.project_id).await?;
        let plot = self.repository.create_plot_line(&input).await?;
        self.refresh_entity_index(
            &plot.project_id,
            &plot.branch_id,
            &plot.id,
            "plot_line",
            &plot.name,
            &plot.summary,
        )
        .await?;
        Ok(CreatePlotLineOutput {
            plot_line_id: plot.id,
        })
    }

    pub async fn create_conflict(
        &self,
        input: CreateConflictInput,
    ) -> Result<CreateConflictOutput> {
        self.repository.get_project(&input.project_id).await?;
        let conflict = self.repository.create_conflict(&input).await?;
        self.refresh_entity_index(
            &conflict.project_id,
            &conflict.branch_id,
            &conflict.id,
            "conflict",
            &conflict.name,
            &conflict.stakes,
        )
        .await?;
        Ok(CreateConflictOutput {
            conflict_id: conflict.id,
        })
    }

    pub async fn create_theme(&self, input: CreateThemeInput) -> Result<CreateThemeOutput> {
        self.repository.get_project(&input.project_id).await?;
        let theme = self.repository.create_theme(&input).await?;
        self.refresh_entity_index(
            &theme.project_id,
            &theme.branch_id,
            &theme.id,
            "theme",
            &theme.theme_statement,
            &theme.thesis_antithesis,
        )
        .await?;
        Ok(CreateThemeOutput { theme_id: theme.id })
    }

    pub async fn create_motif(&self, input: CreateMotifInput) -> Result<CreateMotifOutput> {
        self.repository.get_project(&input.project_id).await?;
        let motif = self.repository.create_motif(&input).await?;
        self.refresh_entity_index(
            &motif.project_id,
            &motif.branch_id,
            &motif.id,
            "motif",
            &motif.name,
            &motif.description,
        )
        .await?;
        Ok(CreateMotifOutput { motif_id: motif.id })
    }

    pub async fn create_narrative_promise(
        &self,
        input: CreateNarrativePromiseInput,
    ) -> Result<CreateNarrativePromiseOutput> {
        self.repository.get_project(&input.project_id).await?;
        let promise = self.repository.create_narrative_promise(&input).await?;
        self.refresh_entity_index(
            &promise.project_id,
            &promise.branch_id,
            &promise.id,
            "narrative_promise",
            &promise.promise_type,
            &promise.description,
        )
        .await?;
        Ok(CreateNarrativePromiseOutput {
            narrative_promise_id: promise.id,
        })
    }

    pub async fn create_world_rule(
        &self,
        input: CreateWorldRuleInput,
    ) -> Result<CreateWorldRuleOutput> {
        self.repository.get_project(&input.project_id).await?;
        let rule = self.repository.create_world_rule(&input).await?;
        let mut cache_ids = vec![PhaseFourCacheId::WorldRuleSemanticDrift];
        if rule.rule_type.eq_ignore_ascii_case("style") {
            cache_ids.push(PhaseFourCacheId::StyleCompliance);
        }
        self.resolve_phase_four_caches(&rule.project_id, &rule.branch_id, &cache_ids)
            .await?;
        self.refresh_entity_index(
            &rule.project_id,
            &rule.branch_id,
            &rule.id,
            "world_rule",
            &rule.rule_name,
            &rule.description,
        )
        .await?;
        Ok(CreateWorldRuleOutput {
            world_rule_id: rule.id,
        })
    }

    /// Persist a chapter summary, replacing any previous one at the same
    /// (project, branch, book#, chapter#) tuple.
    pub async fn save_summary(&self, input: SaveSummaryInput) -> Result<SaveSummaryOutput> {
        self.repository.get_project(&input.project_id).await?;
        let summary = self.repository.save_summary(&input).await?;
        // Recompute realized arc pacing for the affected book so the previously
        // inert pacing_budget_audit reflects actual chapter throughput.
        let branch_id = self
            .repository
            .get_active_branch(&input.project_id)
            .await?
            .id;
        self.repository
            .recompute_pacing_for_book(&input.project_id, &branch_id, input.book_number)
            .await?;
        Ok(SaveSummaryOutput {
            chapter_summary_id: summary.id,
        })
    }

    // =========================================================================
    // Batch creates
    // =========================================================================

    pub async fn batch_create_terms(
        &self,
        input: BatchCreateTermsInput,
    ) -> Result<BatchCreateTermsOutput> {
        self.repository.get_project(&input.project_id).await?;
        let mut term_ids = Vec::with_capacity(input.items.len());
        for item in input.items {
            let term = self
                .create_term(spindle_core::models::CreateTermInput {
                    project_id: input.project_id.clone(),
                    term_text: item.term_text,
                    pronunciation: item.pronunciation,
                    definition: item.definition,
                    usage_context: item.usage_context,
                    origin: item.origin,
                })
                .await?;
            term_ids.push(term.term_id);
        }
        let created = term_ids.len();
        Ok(BatchCreateTermsOutput { term_ids, created })
    }

    pub async fn batch_create_motifs(
        &self,
        input: BatchCreateMotifsInput,
    ) -> Result<BatchCreateMotifsOutput> {
        self.repository.get_project(&input.project_id).await?;
        let mut motif_ids = Vec::with_capacity(input.items.len());
        for item in input.items {
            let motif = self
                .create_motif(spindle_core::models::CreateMotifInput {
                    project_id: input.project_id.clone(),
                    name: item.name,
                    description: item.description,
                    max_uses_per_chapter: item.max_uses_per_chapter,
                    connected_theme_ids: item.connected_theme_ids,
                })
                .await?;
            motif_ids.push(motif.motif_id);
        }
        let created = motif_ids.len();
        Ok(BatchCreateMotifsOutput { motif_ids, created })
    }

    pub async fn batch_create_narrative_promises(
        &self,
        input: BatchCreateNarrativePromisesInput,
    ) -> Result<BatchCreateNarrativePromisesOutput> {
        self.repository.get_project(&input.project_id).await?;
        let mut narrative_promise_ids = Vec::with_capacity(input.items.len());
        for item in input.items {
            let p = self
                .create_narrative_promise(spindle_core::models::CreateNarrativePromiseInput {
                    project_id: input.project_id.clone(),
                    promise_type: item.promise_type,
                    description: item.description,
                    planted_at: item.planted_at,
                    planned_payoff: item.planned_payoff,
                    notes: item.notes,
                })
                .await?;
            narrative_promise_ids.push(p.narrative_promise_id);
        }
        let created = narrative_promise_ids.len();
        Ok(BatchCreateNarrativePromisesOutput {
            narrative_promise_ids,
            created,
        })
    }

    /// Shared helper for the create_* methods above: builds a SearchDocument
    /// from the entity's text fields and upserts it into the search index.
    pub(super) async fn refresh_entity_index(
        &self,
        project_id: &str,
        branch_id: &str,
        entity_id: &str,
        entity_table: &str,
        title: &str,
        excerpt: &str,
    ) -> Result<()> {
        let document = SearchDocument {
            entity_table: entity_table.to_string(),
            title: title.to_string(),
            excerpt: excerpt.to_string(),
            content: format!("{title}\n\n{excerpt}"),
        };
        self.repository
            .refresh_search_embedding_for_entity(project_id, branch_id, entity_id, &document)
            .await
    }
}
