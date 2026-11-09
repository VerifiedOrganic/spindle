use super::*;

fn ensure_batch_len<T>(items: &[T]) -> Result<()> {
    anyhow::ensure!(
        !items.is_empty() && items.len() <= 50,
        "batch items must contain 1..=50 entries"
    );
    Ok(())
}

impl SqliteSpindleService {
    /// Canonical atomic batch create for glossary terms.
    pub async fn create_terms(&self, input: CreateTermsInput) -> Result<CreateTermsOutput> {
        ensure_batch_len(&input.items)?;
        self.repository.get_project(&input.project_id).await?;
        for (index, item) in input.items.iter().enumerate() {
            anyhow::ensure!(
                !item.term_text.trim().is_empty(),
                "item {index}: term_text must not be empty"
            );
            anyhow::ensure!(
                !item.definition.trim().is_empty(),
                "item {index}: definition must not be empty"
            );
        }
        if input.dry_run {
            return Ok(CreateTermsOutput {
                term_ids: Vec::new(),
                created: 0,
                dry_run: true,
            });
        }
        let branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        let project_id = input.project_id;
        let items = input.items;
        let tx_items = items.clone();
        let tx_project = project_id.clone();
        let tx_branch = branch_id.clone();
        let ids = self
            .repository
            .with_transaction(move |tx| {
                let mut ids = Vec::with_capacity(tx_items.len());
                for (index, item) in tx_items.iter().enumerate() {
                    let id = tx
                        .create_term_in_tx(&tx_project, &tx_branch, item)
                        .with_context(|| format!("item {index}"))?;
                    ids.push(id);
                }
                Ok(ids)
            })
            .await?;
        for (id, item) in ids.iter().zip(items.iter()) {
            self.refresh_entity_index(
                &project_id,
                &branch_id,
                id,
                "term",
                &item.term_text,
                &item.definition,
            )
            .await?;
        }
        Ok(CreateTermsOutput {
            created: ids.len(),
            term_ids: ids,
            dry_run: false,
        })
    }

    /// Canonical atomic batch create for motifs.
    pub async fn create_motifs(&self, input: CreateMotifsInput) -> Result<CreateMotifsOutput> {
        ensure_batch_len(&input.items)?;
        self.repository.get_project(&input.project_id).await?;
        for (index, item) in input.items.iter().enumerate() {
            anyhow::ensure!(
                !item.name.trim().is_empty(),
                "item {index}: name must not be empty"
            );
        }
        if input.dry_run {
            return Ok(CreateMotifsOutput {
                motif_ids: Vec::new(),
                created: 0,
                dry_run: true,
            });
        }
        let branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        let project_id = input.project_id;
        let items = input.items;
        let tx_items = items.clone();
        let tx_project = project_id.clone();
        let tx_branch = branch_id.clone();
        let ids = self
            .repository
            .with_transaction(move |tx| {
                let mut ids = Vec::with_capacity(tx_items.len());
                for (index, item) in tx_items.iter().enumerate() {
                    let id = tx
                        .create_motif_in_tx(&tx_project, &tx_branch, item)
                        .with_context(|| format!("item {index}"))?;
                    ids.push(id);
                }
                Ok(ids)
            })
            .await?;
        for (id, item) in ids.iter().zip(items.iter()) {
            self.refresh_entity_index(
                &project_id,
                &branch_id,
                id,
                "motif",
                &item.name,
                &item.description,
            )
            .await?;
        }
        Ok(CreateMotifsOutput {
            created: ids.len(),
            motif_ids: ids,
            dry_run: false,
        })
    }

    /// Canonical atomic batch create for narrative promises.
    pub async fn create_narrative_promises(
        &self,
        input: CreateNarrativePromisesInput,
    ) -> Result<CreateNarrativePromisesOutput> {
        ensure_batch_len(&input.items)?;
        self.repository.get_project(&input.project_id).await?;
        for (index, item) in input.items.iter().enumerate() {
            anyhow::ensure!(
                !item.promise_type.trim().is_empty(),
                "item {index}: promise_type must not be empty"
            );
            anyhow::ensure!(
                !item.description.trim().is_empty(),
                "item {index}: description must not be empty"
            );
        }
        if input.dry_run {
            return Ok(CreateNarrativePromisesOutput {
                narrative_promise_ids: Vec::new(),
                created: 0,
                dry_run: true,
            });
        }
        let branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        let project_id = input.project_id;
        let items = input.items;
        let tx_items = items.clone();
        let tx_project = project_id.clone();
        let tx_branch = branch_id.clone();
        let ids = self
            .repository
            .with_transaction(move |tx| {
                let mut ids = Vec::with_capacity(tx_items.len());
                for (index, item) in tx_items.iter().enumerate() {
                    let id = tx
                        .create_narrative_promise_in_tx(
                            &tx_project,
                            &tx_branch,
                            &item.promise_type,
                            &item.description,
                            &item.planted_at,
                            item.planned_payoff.as_ref(),
                            &item.notes,
                        )
                        .with_context(|| format!("item {index}"))?;
                    ids.push(id);
                }
                Ok(ids)
            })
            .await?;
        for (id, item) in ids.iter().zip(items.iter()) {
            self.refresh_entity_index(
                &project_id,
                &branch_id,
                id,
                "narrative_promise",
                &item.promise_type,
                &item.description,
            )
            .await?;
        }
        Ok(CreateNarrativePromisesOutput {
            created: ids.len(),
            narrative_promise_ids: ids,
            dry_run: false,
        })
    }

    /// Canonical atomic batch set for character voice profiles.
    pub async fn set_character_voice_profiles(
        &self,
        input: SetCharacterVoiceProfilesInput,
    ) -> Result<SetCharacterVoiceProfilesOutput> {
        ensure_batch_len(&input.items)?;
        self.repository.get_project(&input.project_id).await?;
        let active_branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        anyhow::ensure!(
            input.branch_id == active_branch_id,
            "branch_id must be the active branch for a batch voice update"
        );
        for (index, item) in input.items.iter().enumerate() {
            anyhow::ensure!(
                item.profile.updated_at.is_none(),
                "item {index}: voice profile updated_at is server-managed and must be omitted"
            );
            let character = self.repository.get_character(&item.character_id).await?;
            anyhow::ensure!(
                character.project_id == input.project_id,
                "item {index}: character does not belong to the requested project"
            );
            anyhow::ensure!(
                character.branch_id == active_branch_id,
                "item {index}: character must belong to the active branch"
            );
            if let Some(scene_id) = item.profile.established_in_scene_id.as_deref() {
                let scene = self.repository.get_scene(scene_id).await?;
                anyhow::ensure!(
                    scene.project_id == input.project_id,
                    "item {index}: established_in_scene_id must belong to the requested project"
                );
                anyhow::ensure!(
                    self.repository
                        .branch_inherits_from(
                            &input.project_id,
                            &active_branch_id,
                            &scene.branch_id
                        )
                        .await?,
                    "item {index}: established_in_scene_id must belong to the active branch"
                );
            }
        }
        if input.dry_run {
            return Ok(SetCharacterVoiceProfilesOutput {
                profiles: Vec::new(),
                updated: 0,
                dry_run: true,
            });
        }
        let project_id = input.project_id;
        let items = input.items;
        let tx_items = items.clone();
        self.repository
            .with_transaction(move |tx| {
                for (index, item) in tx_items.iter().enumerate() {
                    tx.upsert_character_voice_profile_in_tx(&item.character_id, &item.profile)
                        .with_context(|| format!("item {index}"))?;
                }
                Ok(())
            })
            .await?;
        self.resolve_phase_four_caches(
            &project_id,
            &active_branch_id,
            &[PhaseFourCacheId::VoiceDrift],
        )
        .await?;
        let mut profiles = Vec::with_capacity(items.len());
        for item in items {
            let character = self.repository.get_character(&item.character_id).await?;
            let activity = self
                .repository
                .append_session_activity(AppendSessionActivityParams {
                    project_id: project_id.clone(),
                    branch_id: active_branch_id.clone(),
                    kind: "voice_profile_set".to_string(),
                    subject_table: Some("character".to_string()),
                    subject_id: Some(item.character_id.clone()),
                    summary: format!("updated voice profile for {}", character.name),
                    details_json: Some(serde_json::json!({
                        "character_id": item.character_id,
                        "character_name": character.name,
                        "tone": item.profile.tone,
                        "vocabulary_count": item.profile.vocabulary.len(),
                        "sentence_structure_count": item.profile.sentence_structure.len(),
                        "tics_count": item.profile.tics.len(),
                        "forbidden_words_count": item.profile.forbidden_words.len(),
                        "example_lines_count": item.profile.example_lines.len(),
                        "established_in_scene_id": item.profile.established_in_scene_id,
                    })),
                })
                .await
                .ok();
            profiles.push(SetCharacterVoiceProfileOutput {
                character_id: item.character_id,
                branch_id: active_branch_id.clone(),
                profile: item.profile,
                activity_id: activity.map(|activity| activity.id),
            });
        }
        Ok(SetCharacterVoiceProfilesOutput {
            updated: profiles.len(),
            profiles,
            dry_run: false,
        })
    }

    // ---- One-release aliases -------------------------------------------------

    pub async fn batch_create_terms(
        &self,
        input: BatchCreateTermsInput,
    ) -> Result<BatchCreateTermsOutput> {
        let out = self
            .create_terms(CreateTermsInput {
                project_id: input.project_id,
                items: input.items,
                dry_run: input.dry_run.unwrap_or(false),
            })
            .await?;
        Ok(BatchCreateTermsOutput {
            term_ids: out.term_ids,
            created: out.created,
        })
    }

    pub async fn batch_create_motifs(
        &self,
        input: BatchCreateMotifsInput,
    ) -> Result<BatchCreateMotifsOutput> {
        let out = self
            .create_motifs(CreateMotifsInput {
                project_id: input.project_id,
                items: input.items,
                dry_run: input.dry_run.unwrap_or(false),
            })
            .await?;
        Ok(BatchCreateMotifsOutput {
            motif_ids: out.motif_ids,
            created: out.created,
        })
    }

    pub async fn batch_create_narrative_promises(
        &self,
        input: BatchCreateNarrativePromisesInput,
    ) -> Result<BatchCreateNarrativePromisesOutput> {
        let out = self
            .create_narrative_promises(CreateNarrativePromisesInput {
                project_id: input.project_id,
                items: input.items,
                dry_run: input.dry_run.unwrap_or(false),
            })
            .await?;
        Ok(BatchCreateNarrativePromisesOutput {
            narrative_promise_ids: out.narrative_promise_ids,
            created: out.created,
        })
    }

    pub async fn batch_set_character_voice_profiles(
        &self,
        input: BatchSetCharacterVoiceProfilesInput,
    ) -> Result<BatchSetCharacterVoiceProfilesOutput> {
        let out = self
            .set_character_voice_profiles(SetCharacterVoiceProfilesInput {
                project_id: input.project_id,
                branch_id: input.branch_id,
                items: input.items,
                dry_run: input.dry_run.unwrap_or(false),
            })
            .await?;
        Ok(BatchSetCharacterVoiceProfilesOutput {
            profiles: out.profiles,
            updated: out.updated,
        })
    }
}
