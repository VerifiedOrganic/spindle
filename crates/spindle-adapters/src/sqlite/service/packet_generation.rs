use super::*;
use crate::drafting::{
    DraftTokenizer, PacketBudget, artifact,
    critique::{self, CRITIC_RUBRIC_VERSION, CriticBudget},
};
use spindle_core::drafting::{
    CriticFindingInput, DraftingPacketV1, EditorialPolicy, SHADOW_CRITIQUE_ARTIFACT_SCHEMA_VERSION,
    ShadowCritiqueArtifactV1,
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
            let prepared = captured.prepared_packet.as_ref().ok_or_else(|| {
                anyhow::anyhow!("captured packet/report missing; manual recovery required")
            })?;
            let recorded_seed = prepared
                .packet
                .assembly
                .as_ref()
                .and_then(|assembly| assembly.generation_seed);
            anyhow::ensure!(
                recorded_seed == budget.generation_seed,
                "artifact generation seed mismatch"
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

    /// Execute a shadow-only scene critic pass against a persisted
    /// `PlainTextDraftArtifactV1` and save the resulting `ShadowCritiqueArtifactV1`
    /// without mutating the draft artifact, its prose, or any scene/canon tables.
    #[allow(clippy::too_many_arguments)]
    pub async fn critique_artifact_shadow(
        &self,
        policy: EditorialPolicy,
        draft_artifact_path: &std::path::Path,
        critique_artifact_path: &std::path::Path,
        findings: Vec<CriticFindingInput>,
        budget: &CriticBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        max_calls: u32,
    ) -> Result<ShadowCritiqueArtifactV1> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "shadow critique requires critique_v1 policy"
        );
        let draft_bytes_before = std::fs::read(draft_artifact_path)?;
        let draft = artifact::load_artifact(draft_artifact_path)?;
        anyhow::ensure!(
            !draft.truncated,
            "incomplete/truncated draft artifact cannot be critiqued"
        );
        let prepared = draft.prepared_packet.as_ref().ok_or_else(|| {
            anyhow::anyhow!("draft artifact missing prepared_packet; cannot run shadow critique")
        })?;
        let branch = self.repository.get_active_branch(&draft.project_id).await?;
        anyhow::ensure!(
            branch.id == draft.branch_id,
            "draft artifact branch is no longer active"
        );

        let expected_ingested = critique::ingest_and_dedup_findings(
            findings.clone(),
            &draft.text,
            &draft.text_sha256,
            draft.scene_id.as_deref(),
        )?;
        if critique_artifact_path.exists() {
            let existing = critique::load_shadow_critique_artifact(critique_artifact_path)?;
            anyhow::ensure!(
                existing.draft_artifact_id == draft.artifact_id
                    && existing.base_text_hash == draft.text_sha256
                    && existing.packet_hash == draft.packet_hash
                    && existing.project_id == draft.project_id
                    && existing.branch_id == draft.branch_id
                    && existing.ingested_findings == expected_ingested.0,
                "existing shadow critique artifact belongs to different draft or findings snapshot"
            );
            let draft_bytes_after = std::fs::read(draft_artifact_path)?;
            anyhow::ensure!(
                draft_bytes_after == draft_bytes_before,
                "shadow critique violated draft artifact immutability"
            );
            return Ok(existing);
        }

        let run_result = self
            .repository
            .model_router()
            .critique_draft_shadow(
                policy,
                &prepared.packet,
                &draft.packet_hash,
                &draft.text,
                &draft.text_sha256,
                findings,
                budget,
                tokenizer,
                max_calls,
            )
            .await?;

        let shadow_artifact = ShadowCritiqueArtifactV1 {
            schema_version: SHADOW_CRITIQUE_ARTIFACT_SCHEMA_VERSION.into(),
            artifact_id: format!("shadow_critique:{}", draft.artifact_id),
            draft_artifact_id: draft.artifact_id.clone(),
            project_id: draft.project_id.clone(),
            branch_id: draft.branch_id.clone(),
            scene_id: draft.scene_id.clone(),
            base_text_hash: draft.text_sha256.clone(),
            packet_hash: draft.packet_hash.clone(),
            rubric_version: CRITIC_RUBRIC_VERSION.into(),
            rating: draft.rating.clone(),
            ingested_findings: run_result.ingested_findings,
            deduplicated_finding_ids: run_result.deduplicated_finding_ids,
            prompt_hashes: run_result.prompt_hashes,
            model_call_ids: run_result.model_call_ids,
            outcome: run_result.outcome,
        };
        critique::save_shadow_critique_artifact(critique_artifact_path, &shadow_artifact)?;

        // Enforce strict shadow-mode immutability: the draft artifact on disk must be byte-for-byte unchanged.
        let draft_bytes_after = std::fs::read(draft_artifact_path)?;
        anyhow::ensure!(
            draft_bytes_after == draft_bytes_before,
            "shadow critique violated draft artifact immutability"
        );
        Ok(shadow_artifact)
    }

    /// Ensure a deterministic `(project_id, branch_id)` pair exists as an active
    /// project/branch in SQLite for evaluation packet runs. Never mutates an
    /// existing project's active branch to a different branch ID.
    pub async fn ensure_eval_project_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<()> {
        let pid = project_id.to_string();
        let bid = branch_id.to_string();
        let now = chrono::Utc::now().timestamp_micros();
        self.repository
            .pool()
            .write(move |conn| {
                let tx = conn.savepoint()?;
                tx.execute(
                    "INSERT INTO project \
                     (id, name, project_type, genre, reader_contract, active_branch_id, notes, created_at, updated_at) \
                     VALUES (?1, ?1, 'novel', 'eval', '{\"promise\":\"eval\",\"style_notes\":[],\"boundaries\":[]}', ?2, NULL, ?3, ?3) \
                     ON CONFLICT(id) DO NOTHING",
                    rusqlite::params![&pid, &bid, now],
                )?;
                tx.execute(
                    "INSERT INTO bible_branch \
                     (id, project_id, parent_branch_id, name, status, branch_type, description, created_from_save_point_id, created_at, updated_at) \
                     VALUES (?1, ?2, NULL, 'main', 'active', NULL, NULL, NULL, ?3, ?3) \
                     ON CONFLICT(id) DO NOTHING",
                    rusqlite::params![&bid, &pid, now],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        let active = self.repository.get_active_branch(project_id).await?;
        anyhow::ensure!(active.id == branch_id, "packet branch is no longer active");
        Ok(())
    }
}
