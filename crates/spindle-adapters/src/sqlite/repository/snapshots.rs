use super::*;

impl Repository {
    // =========================================================================
    // assemble_subject_snapshot
    //
    // Port of the SurrealDB `assemble_subject_snapshot[_s]` /
    // `load_snapshot_batch_context` / `assemble_subject_snapshot_from_batch`
    // surface. The big-batch loader fans out one read per table to populate
    // `SnapshotBatchContext`; pure helpers then project the in-memory batch
    // into a `SubjectSnapshot` per subject. ID semantics differ: SQLite uses
    // plain `String` IDs of the form `"<table>:<ulid>"` rather than the
    // SurrealDB `RecordId { table, key }` pair, so id-to-table dispatch is a
    // string split on the leading `':'`.
    // =========================================================================

    pub async fn assemble_subject_snapshot(
        &self,
        project_id: &str,
        branch_id: &str,
        subject: &Subject,
        placement: &StoryPlacement,
    ) -> Result<SubjectSnapshot> {
        let mut snapshots = self
            .assemble_subject_snapshots(
                project_id,
                branch_id,
                std::slice::from_ref(subject),
                placement,
            )
            .await?;
        snapshots
            .pop()
            .context("subject snapshot assembly returned no snapshot")
    }

    pub async fn assemble_subject_snapshots(
        &self,
        project_id: &str,
        branch_id: &str,
        subjects: &[Subject],
        placement: &StoryPlacement,
    ) -> Result<Vec<SubjectSnapshot>> {
        let batch = self
            .load_snapshot_batch_context(project_id, branch_id)
            .await?;
        subjects
            .iter()
            .map(|subject| assemble_subject_snapshot_from_batch(&batch, subject, placement))
            .collect()
    }

    /// Load every table the snapshot helpers depend on in one shot. The
    /// SurrealDB version fans these out via `tokio::try_join!`; the SQLite
    /// pool is single-writer (and `read` calls already share a pool of
    /// connections), so we issue the reads sequentially rather than fighting
    /// the borrow checker over the `&self` capture across futures. The cost
    /// is the same: each table is one prepared statement against an indexed
    /// `(project_id, branch_id)` key.
    pub(super) async fn load_snapshot_batch_context(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<SnapshotBatchContext> {
        let project = self.get_project(project_id).await?;
        let books = self.list_books_by_project(project_id).await?;
        let chapters = self.list_chapters_by_project(project_id).await?;
        let world_rules = self
            .list_world_rules_by_project_and_branch(project_id, branch_id)
            .await?;
        let characters = self
            .list_characters_by_project_and_branch(project_id, branch_id)
            .await?;
        let locations = self
            .list_locations_by_project_and_branch(project_id, branch_id)
            .await?;
        let factions = self
            .list_factions_by_project_and_branch(project_id, branch_id)
            .await?;
        let religions = self
            .list_religions_by_project_and_branch(project_id, branch_id)
            .await?;
        let economies = self
            .list_economies_by_project_and_branch(project_id, branch_id)
            .await?;
        let plot_lines = self
            .list_plot_lines_by_project_and_branch(project_id, branch_id)
            .await?;
        let conflicts = self
            .list_conflicts_by_project_and_branch(project_id, branch_id)
            .await?;
        let themes = self
            .list_themes_by_project_and_branch(project_id, branch_id)
            .await?;
        let motifs = self
            .list_motifs_by_project_and_branch(project_id, branch_id)
            .await?;
        let overlays = self
            .list_system_overlays_by_project_and_branch(project_id, branch_id)
            .await?;
        let promises = self
            .list_narrative_promises_by_project_and_branch(project_id, branch_id)
            .await?;
        let arcs = self
            .list_character_arcs_by_project_and_branch(project_id, branch_id)
            .await?;
        let terms = self
            .list_terms_by_project_and_branch(project_id, branch_id)
            .await?;
        let relationships = self.list_relationships_by_branch(branch_id).await?;
        let events = self
            .list_timeline_events_by_project_and_branch(project_id, branch_id)
            .await?;
        let scenes = self
            .list_scenes_by_project_and_branch(project_id, branch_id)
            .await?;
        let canonical_facts = self
            .list_active_canonical_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let knowledge_facts = self
            .list_knowledge_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let character_states = self
            .list_character_states_by_project_and_branch(project_id, branch_id)
            .await?;
        let voice_profiles = self
            .list_voice_profiles_by_project_and_branch(project_id, branch_id)
            .await?;
        let world_states = self
            .list_world_states_by_project_and_branch(project_id, branch_id)
            .await?;

        Ok(SnapshotBatchContext {
            project,
            books,
            chapters,
            world_rules,
            characters,
            locations,
            factions,
            religions,
            economies,
            plot_lines,
            conflicts,
            themes,
            motifs,
            overlays,
            promises,
            arcs,
            terms,
            relationships,
            events,
            scenes,
            canonical_facts,
            knowledge_facts,
            character_states,
            voice_profiles,
            world_states,
        })
    }
}
