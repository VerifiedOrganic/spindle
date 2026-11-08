use super::*;
use crate::drafting::{DraftTokenizer, PacketBudget, artifact};
use spindle_core::drafting::{
    DraftingPacketV1, EditorialPolicy,
    extraction::{DraftReceiptBinding, PlainTextDraftArtifactV1},
};

impl SqliteSpindleService {
    /// Internal opt-in seam; authoring runs retain the legacy package workflow.
    /// Caller supplies a private local artifact path and a verified tokenizer.
    pub async fn draft_packet_to_artifact(
        &self,
        policy: EditorialPolicy,
        packet: DraftingPacketV1,
        budget: &PacketBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        path: &std::path::Path,
    ) -> Result<PlainTextDraftArtifactV1> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "packet drafting requires critique_v1 policy"
        );
        let branch = self
            .repository
            .get_active_branch(&packet.project_id)
            .await?;
        anyhow::ensure!(
            branch.id == packet.branch_id,
            "packet branch is no longer active"
        );
        let mut lock = artifact::lock_artifact(path)?;
        if path.exists() {
            let captured = artifact::load_artifact(path)?;
            anyhow::ensure!(
                captured.prepared_packet.is_some(),
                "captured packet/report missing; manual recovery required"
            );
            let source_hash = generation_sha256_hex(&serde_json::to_vec(&packet)?);
            anyhow::ensure!(
                captured.source_packet_sha256 == source_hash
                    && captured.project_id == packet.project_id
                    && captured.branch_id == packet.branch_id,
                "artifact belongs to a different packet snapshot"
            );
            let receipt = self
                .verified_revisable_draft_receipt(Some(&captured.receipt.generation_id))
                .await?
                .ok_or_else(|| anyhow::anyhow!("draft receipt missing"))?;
            anyhow::ensure!(
                receipt.output_sha256 == captured.receipt.normalized_output_sha256
                    && receipt.agent_id == captured.receipt.agent_id
                    && receipt.rating.as_deref() == Some(captured.rating.as_str()),
                "artifact receipt binding mismatch"
            );
            return Ok(captured);
        }
        let (prepared, response) = self
            .repository
            .model_router()
            .draft_packet_before_dispatch(policy, packet.clone(), budget, tokenizer, |prepared| {
                lock.retain_for_recovery(&prepared.messages_hash)
            })
            .await?;
        let receipt = self
            .register_generation_receipt_with_calls(
                "draft",
                Some(packet.scene_card.rating.as_str()),
                &response.model_name,
                &response.output,
                response.call_id.into_iter().collect(),
            )
            .await?;
        let mut captured = artifact::capture_plain_text(
            &packet,
            &prepared.messages_hash,
            response.output,
            response.truncated,
            DraftReceiptBinding {
                generation_id: receipt.id,
                agent_id: receipt.agent_id,
                normalized_output_sha256: receipt.output_sha256,
            },
        )?;
        captured.prepared_packet = Some(prepared);
        artifact::save_artifact(path, &captured)?;
        lock.captured();
        Ok(captured)
    }
}
