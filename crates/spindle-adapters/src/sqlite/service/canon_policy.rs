//! Read-side canon cutover helpers: provisional constraint findings and
//! pending-work reporting (P2 PR E).

use super::*;
use spindle_core::canon::*;
use spindle_core::models::ConsistencyIssue;

impl SqliteSpindleService {
    /// Provisional canon claims are visible as labeled warnings/info while the
    /// project is in dual-write/shadow-read/canon-read. Legacy-unverified rows
    /// never produce an error block; they need corroboration first.
    pub async fn canon_consistency_issues(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<ConsistencyIssue>> {
        if self.repository.canon_cutover_mode(project_id).await? == CanonCutoverModeV1::Legacy {
            return Ok(Vec::new());
        }
        let claims = self
            .repository
            .list_canon_claims(project_id, branch_id, None, None)
            .await?;
        let mut issues = Vec::new();
        for claim in claims
            .iter()
            .filter(|claim| claim.status == CanonClaimStatus::Provisional)
        {
            let corroborated = claim.provenance != CanonProvenanceKind::LegacyUnverified;
            let finding = classify_legacy_finding(LegacyCanonFindingV1 {
                severity: if can_trigger_error_finding(claim.provenance, corroborated) {
                    "warning".into()
                } else {
                    "info".into()
                },
                check_type: "canon_consistency".into(),
                message: format!(
                    "provisional canon claim '{}' ({:?}) awaits author ratification",
                    claim.claim.proposed.claim_id, claim.provenance
                ),
                provenance: claim.provenance,
                corroborated,
            });
            issues.push(ConsistencyIssue {
                severity: finding.severity,
                check_type: finding.check_type,
                message: finding.message,
                entity_ids: claim
                    .scene_id
                    .iter()
                    .map(|scene_id| format!("scene:{scene_id}"))
                    .collect(),
                suggested_action: Some(
                    "Review the evidence-bound claim and ratify or reject it; do not treat a provisional claim as world truth.".into(),
                ),
            });
        }
        Ok(issues)
    }

    /// Counts eligible claims without a final policy decision. A checkpoint or
    /// continuity report must not say "clean" while this returns some.
    pub async fn canon_pending_work(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<CanonPendingWorkV1> {
        let claims = self
            .repository
            .list_canon_claims(project_id, branch_id, None, None)
            .await?;
        let pending_scene_count = claims
            .iter()
            .filter(|claim| claim.status == CanonClaimStatus::Provisional)
            .filter_map(|claim| claim.scene_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let disputed_high_impact_prerequisites = claims
            .iter()
            .filter(|claim| claim.status == CanonClaimStatus::Provisional)
            .filter(|claim| {
                classify_canon_claim(
                    &claim.claim,
                    None,
                    false,
                    &std::collections::BTreeSet::new(),
                )
                .is_v1_high_impact()
            })
            .count();
        Ok(evaluate_canon_pending_work(&CanonPendingWorkInputV1 {
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            pending_scene_count,
            disputed_high_impact_prerequisites,
            failed_extraction_scene_count: 0,
            retries_remaining: 2,
        }))
    }
}
