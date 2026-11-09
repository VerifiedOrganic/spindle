use super::*;

use spindle_core::models::{CommitSceneInput, CommitSceneOutput, CommitSceneRunPhase};

impl ToolRouter {
    /// Commit a scene through the atomic service boundary and, when the call
    /// names an authoring run, advance that run's scene phase only after the
    /// live commit succeeds. A dry run or blocked commit never advances it.
    pub(super) async fn handle_commit_scene(
        &self,
        input: CommitSceneInput,
    ) -> anyhow::Result<CommitSceneOutput> {
        let run_id = input
            .run_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let expected_phase = input.expected_phase;
        let dry_run = input.dry_run;

        // A lost-response retry must reach its stored receipt before any live
        // phase assertion: the first success already advanced the run phase.
        if let Some(replay) = self.service.commit_scene_replay(&input).await? {
            return Ok(replay);
        }

        if let Some(run_id) = run_id.as_deref() {
            self.validate_commit_scene_run(run_id, &input, expected_phase)
                .await?;
        } else if expected_phase.is_some() {
            anyhow::bail!("expected_phase requires run_id");
        }

        let output = self.service.commit_scene(input).await?;

        if !dry_run
            && output.blocking_count == 0
            && let Some(run_id) = run_id.as_deref()
        {
            self.advance_commit_scene_run(run_id, &output).await?;
        }
        Ok(output)
    }

    /// Preflight the run phase assertion BEFORE the prose transaction opens, so
    /// a mismatched expected phase cannot leave a live commit behind.
    async fn validate_commit_scene_run(
        &self,
        run_id: &str,
        input: &CommitSceneInput,
        expected_phase: Option<CommitSceneRunPhase>,
    ) -> anyhow::Result<()> {
        let repo = self.service.repository();
        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(run_id)
            .await?
            .with_context(|| format!("authoring run {run_id} not found"))?;
        if matches!(run.status.as_str(), "completed" | "paused" | "cancelled") {
            anyhow::bail!(
                "authoring run {run_id} is {}; cannot commit a scene into it",
                run.status
            );
        }
        if input.project_id != run.project_id {
            anyhow::bail!(
                "commit_scene run integration requires project {} but the payload targets {}",
                run.project_id,
                input.project_id
            );
        }
        let branch_id = match input
            .branch_id
            .as_deref()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
        {
            Some(branch_id) => branch_id.to_string(),
            None => repo.active_branch_id_public(&input.project_id).await?,
        };
        if branch_id != run.active_branch_id {
            anyhow::bail!(
                "commit_scene run integration requires branch {} but the payload targets {}",
                run.active_branch_id,
                branch_id
            );
        }
        let (book_number, chapter_number, scene_order) = match input.scene_id.as_deref() {
            Some(scene_id) => {
                let scene = repo.get_scene(scene_id).await?;
                if scene.project_id != run.project_id {
                    anyhow::bail!(
                        "scene {scene_id} does not belong to authoring run {run_id}'s project"
                    );
                }
                if scene.branch_id != run.active_branch_id {
                    anyhow::bail!(
                        "scene {scene_id} does not belong to authoring run {run_id}'s branch"
                    );
                }
                (scene.book_number, scene.chapter_number, scene.scene_order)
            }
            None => (
                input
                    .book_number
                    .context("commit_scene run integration requires book_number or scene_id")?,
                input
                    .chapter_number
                    .context("commit_scene run integration requires chapter_number or scene_id")?,
                input
                    .scene_order
                    .context("commit_scene run integration requires scene_order or scene_id")?,
            ),
        };
        if book_number != run.book_number {
            anyhow::bail!(
                "commit_scene run integration requires book {} but the payload targets {}",
                run.book_number,
                book_number
            );
        }
        let state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);
        let (chapter_index, scene_index) =
            authoring_scene_indices(&state, chapter_number, scene_order).with_context(|| {
                format!(
                    "scene {chapter_number}.{scene_order} is not part of authoring run {run_id}"
                )
            })?;
        let current = &state.chapters[chapter_index].scenes[scene_index];
        if let Some(expected) = expected_phase {
            let actual = serde_json::to_value(current.phase)?
                .as_str()
                .unwrap_or_default()
                .to_string();
            let expected = serde_json::to_value(expected)?
                .as_str()
                .unwrap_or_default()
                .to_string();
            if actual != expected {
                anyhow::bail!(
                    "expected_phase {expected:?} does not match the run scene's current phase {actual:?}"
                );
            }
        }
        Ok(())
    }

    async fn advance_commit_scene_run(
        &self,
        run_id: &str,
        output: &CommitSceneOutput,
    ) -> anyhow::Result<()> {
        let repo = self.service.repository();
        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(run_id)
            .await?
            .with_context(|| format!("authoring run {run_id} not found"))?;
        if output.branch_id != run.active_branch_id {
            anyhow::bail!(
                "committed scene branch {} does not match authoring run {run_id}'s branch {}",
                output.branch_id,
                run.active_branch_id
            );
        }
        let mut state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);
        let mut found = false;
        // A first commit on a planned run scene has no stored scene id yet, so
        // locate it by the committed placement before stamping the id.
        if let Some(cursor) = output.writer_cursor.as_ref()
            && let Some((chapter_index, scene_index)) =
                authoring_scene_indices(&state, cursor.chapter_number, cursor.scene_order)
        {
            let scene = &mut state.chapters[chapter_index].scenes[scene_index];
            scene.phase = ScenePhase::BeatsAnnotated;
            scene.scene_id = Some(output.scene_id.clone());
            scene.blocked_reason = None;
            found = true;
        }
        if !found {
            for chapter in &mut state.chapters {
                for scene in &mut chapter.scenes {
                    if scene.scene_id.as_deref() == Some(output.scene_id.as_str()) {
                        scene.phase = ScenePhase::BeatsAnnotated;
                        scene.blocked_reason = None;
                        found = true;
                    }
                }
            }
        }
        if !found {
            anyhow::bail!(
                "committed scene {} is not part of authoring run {}; run phase was not advanced",
                output.scene_id,
                run_id
            );
        }
        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(run_id, &state, &run.status, Some(run.created_at));
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;
        Ok(())
    }
}
