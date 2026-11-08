use super::*;

impl SqliteSpindleService {
    // =========================================================================
    // Branch tools
    // =========================================================================

    pub async fn create_branch(&self, input: CreateBranchInput) -> Result<CreateBranchOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let parent_branch_id = match input.parent_branch_id.as_deref() {
            Some(id) => id.to_string(),
            None => project.active_branch_id.clone().unwrap_or_else(|| {
                // Fall back to a synthetic main-branch lookup if no active is set.
                // The repo's create_branch will reject if the parent is invalid,
                // so this is best-effort.
                String::new()
            }),
        };
        let branch = self
            .repository
            .create_branch(
                &input.project_id,
                &parent_branch_id,
                &input.name,
                &input.branch_type,
                input.description,
            )
            .await?;
        Ok(CreateBranchOutput {
            branch_id: branch.id,
            parent_branch_id: branch.parent_branch_id.unwrap_or_default(),
        })
    }

    pub async fn switch_branch(&self, input: SwitchBranchInput) -> Result<SwitchBranchOutput> {
        let branch = self.repository.get_branch(&input.branch_id).await?;
        // Enforce project ownership when the branch carries a project_id.
        if let Some(branch_project_id) = branch.project_id.as_deref()
            && branch_project_id != input.project_id
        {
            anyhow::bail!(
                "branch {} does not belong to project {}",
                input.branch_id,
                input.project_id
            );
        }
        let updated = self
            .repository
            .switch_active_branch(&input.project_id, &input.branch_id)
            .await?;
        let _ = updated;
        Ok(SwitchBranchOutput {
            branch_id: branch.id,
            branch_name: branch.name,
        })
    }

    /// Set or clear the project's narrator-voice directive (the prose-level
    /// narration style consumed by scene-context assembly, the style gate, the
    /// `style_compliance` validator, and the review's Target Reader persona).
    pub async fn set_narrator_voice(
        &self,
        input: spindle_core::models::SetNarratorVoiceInput,
    ) -> Result<spindle_core::models::SetNarratorVoiceOutput> {
        let cleared = input.narrator_voice.is_empty();
        let project = self
            .repository
            .set_narrator_voice(&input.project_id, input.narrator_voice)
            .await?;
        self.resolve_phase_four_caches_for_project(
            &input.project_id,
            &[PhaseFourCacheId::StyleCompliance],
        )
        .await?;
        let narrator_voice = project
            .narrator_voice
            .map(|stored| stored.into_core())
            .unwrap_or_default();
        // Surface any word-count contradiction the new voice introduces
        // against reader_contract.style_notes or a style world rule.
        let warnings = self
            .style_surface_contradiction_warnings(&input.project_id)
            .await?;
        Ok(spindle_core::models::SetNarratorVoiceOutput {
            project_id: project.id,
            narrator_voice,
            cleared,
            warnings,
        })
    }
}
