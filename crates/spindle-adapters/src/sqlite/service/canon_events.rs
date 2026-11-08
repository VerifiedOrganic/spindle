//! Service-layer orchestration for append-only canon claims, decisions,
//! typed events, checkpoints, projections, and branch merges (P2 PR C).

use super::*;
use spindle_core::canon::*;

impl SqliteSpindleService {
    pub async fn record_canon_claims(
        &self,
        project_id: &str,
        branch_id: &str,
        claims: &[ClaimV1],
        status: CanonClaimStatus,
        tier: CanonTier,
        provenance: CanonProvenanceKind,
    ) -> Result<Vec<CanonClaimRecordV1>> {
        self.repository
            .record_canon_claims(project_id, branch_id, claims, status, tier, provenance)
            .await
    }

    pub async fn list_canon_claims(
        &self,
        project_id: &str,
        branch_id: &str,
        scene_id: Option<&str>,
        revision_id: Option<&str>,
    ) -> Result<Vec<CanonClaimRecordV1>> {
        self.repository
            .list_canon_claims(project_id, branch_id, scene_id, revision_id)
            .await
    }

    pub async fn append_canon_decision(
        &self,
        input: AppendCanonDecisionInputV1,
    ) -> Result<AppendCanonDecisionOutputV1> {
        self.repository.append_canon_decision(input).await
    }

    pub async fn canon_state_at(
        &self,
        project_id: &str,
        branch_id: &str,
        cursor: CanonCursorV1,
        tier: CanonTier,
        decision_watermark: Option<i64>,
    ) -> Result<CanonStateViewV1> {
        self.repository
            .canon_state_at(project_id, branch_id, cursor, tier, decision_watermark)
            .await
    }

    pub async fn list_canon_events_for_lineage(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CanonEventV1>> {
        self.repository
            .list_canon_events_for_lineage(project_id, branch_id)
            .await
    }

    pub async fn rebuild_canon_projections(
        &self,
        project_id: &str,
        branch_id: &str,
        tier: CanonTier,
    ) -> Result<CanonStateViewV1> {
        self.repository
            .rebuild_canon_projections(project_id, branch_id, tier)
            .await
    }

    pub async fn list_canon_projection_rows(
        &self,
        project_id: &str,
        branch_id: &str,
        tier: CanonTier,
    ) -> Result<Vec<CanonProjectionRowV1>> {
        self.repository
            .list_canon_projection_rows(project_id, branch_id, tier)
            .await
    }

    pub async fn create_canon_checkpoint(
        &self,
        project_id: &str,
        branch_id: &str,
        save_point_id: Option<String>,
        tier: CanonTier,
    ) -> Result<CanonSavepointCheckpointV1> {
        self.repository
            .create_canon_checkpoint(project_id, branch_id, save_point_id, tier)
            .await
    }

    pub async fn restore_canon_checkpoint(
        &self,
        project_id: &str,
        branch_id: &str,
        checkpoint_id: &str,
        actor: &str,
        reason: &str,
    ) -> Result<CanonStateViewV1> {
        self.repository
            .restore_canon_checkpoint(project_id, branch_id, checkpoint_id, actor, reason)
            .await
    }

    pub async fn merge_canon_branch_events(
        &self,
        project_id: &str,
        common_ancestor_branch_id: &str,
        target_branch_id: &str,
        source_branch_id: &str,
        actor: &str,
        reason: &str,
    ) -> Result<(CanonMergePlanV1, Option<AppendCanonDecisionOutputV1>)> {
        self.repository
            .merge_canon_branch_events(
                project_id,
                common_ancestor_branch_id,
                target_branch_id,
                source_branch_id,
                actor,
                reason,
            )
            .await
    }

    pub async fn upsert_canon_source_map(
        &self,
        entry: CanonSourceMapEntryV1,
    ) -> Result<CanonSourceMapEntryV1> {
        self.repository.upsert_canon_source_map(entry).await
    }

    pub async fn list_canon_source_maps(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        source_table: Option<&str>,
    ) -> Result<Vec<CanonSourceMapEntryV1>> {
        self.repository
            .list_canon_source_maps(project_id, branch_id, source_table)
            .await
    }

    pub async fn backfill_project_canon(&self, project_id: &str) -> Result<CanonBackfillReportV1> {
        self.repository.backfill_project_canon(project_id).await
    }

    pub async fn backfill_project_canon_with_version(
        &self,
        project_id: &str,
        migration_version: u32,
    ) -> Result<CanonBackfillReportV1> {
        self.repository
            .backfill_project_canon_with_version(project_id, migration_version)
            .await
    }

    pub async fn compare_canon_compatibility_shadow_reads(
        &self,
        project_id: &str,
        branch_id: &str,
        placement: Option<StoryPlacement>,
    ) -> Result<CanonCompatibilityShadowReportV1> {
        self.repository
            .compare_canon_compatibility_shadow_reads(project_id, branch_id, placement)
            .await
    }

    pub async fn evaluate_project_canon_cutover_readiness(
        &self,
        project_id: &str,
    ) -> Result<ProjectCutoverReadinessV1> {
        self.repository
            .evaluate_project_canon_cutover_readiness(project_id)
            .await
    }

    pub async fn evaluate_project_canon_cutover_readiness_with_version(
        &self,
        project_id: &str,
        migration_version: u32,
    ) -> Result<ProjectCutoverReadinessV1> {
        self.repository
            .evaluate_project_canon_cutover_readiness_with_version(project_id, migration_version)
            .await
    }

    pub async fn get_canon_projected_subject_snapshot(
        &self,
        project_id: &str,
        branch_id: &str,
        subject: &spindle_core::subject::Subject,
        placement: Option<&StoryPlacement>,
    ) -> Result<spindle_core::subject_snapshot::SubjectSnapshot> {
        self.repository
            .get_canon_projected_subject_snapshot(project_id, branch_id, subject, placement)
            .await
    }
}
