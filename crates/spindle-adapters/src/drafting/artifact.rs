//! Accepted-text lifecycle and local artifacts; no canon extraction or writes.
use super::*;
use spindle_core::drafting::extraction::*;
use std::{io::Write, path::Path};

pub(crate) struct ArtifactLock(std::path::PathBuf);
impl Drop for ArtifactLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub(crate) fn lock_artifact(path: &Path) -> anyhow::Result<ArtifactLock> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut lock = path.as_os_str().to_os_string();
    lock.push(".lock");
    let lock = std::path::PathBuf::from(lock);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|e| anyhow::anyhow!("artifact busy or awaiting crash recovery: {e}"))?;
    Ok(ArtifactLock(lock))
}

fn sha(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn normalized_sha(text: &str) -> String {
    sha(text.replace("\r\n", "\n").trim())
}

pub fn capture_plain_text(
    packet: &DraftingPacketV1,
    packet_hash: &str,
    text: String,
    truncated: bool,
    receipt: DraftReceiptBinding,
) -> anyhow::Result<PlainTextDraftArtifactV1> {
    let source_packet_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(packet)?));
    let artifact = PlainTextDraftArtifactV1 {
        prepared_packet: None,
        source_packet_sha256: source_packet_sha256.clone(),
        schema_version: "plain_text_draft_v1".into(),
        artifact_id: format!("{}:{}", source_packet_sha256, sha(&text)),
        project_id: packet.project_id.clone(),
        branch_id: packet.branch_id.clone(),
        scene_id: packet.scene_id.clone(),
        packet_hash: packet_hash.into(),
        text_sha256: sha(&text),
        text,
        rating: packet.scene_card.rating.clone(),
        truncated,
        receipt,
        accepted_text_sha256: None,
        extraction_request: None,
        extraction_attempts: 0,
        extraction_failure: None,
        extraction: None,
    };
    validate_artifact(&artifact)?;
    Ok(artifact)
}
pub fn validate_artifact(artifact: &PlainTextDraftArtifactV1) -> anyhow::Result<()> {
    anyhow::ensure!(
        artifact.schema_version == "plain_text_draft_v1",
        "unknown artifact version"
    );
    anyhow::ensure!(!artifact.packet_hash.is_empty(), "packet hash required");
    anyhow::ensure!(
        sha(&artifact.text) == artifact.text_sha256,
        "exact text hash mismatch"
    );
    anyhow::ensure!(
        normalized_sha(&artifact.text) == artifact.receipt.normalized_output_sha256,
        "receipt text hash mismatch"
    );
    anyhow::ensure!(
        !artifact.receipt.generation_id.is_empty() && !artifact.receipt.agent_id.is_empty(),
        "generation receipt required"
    );
    if let Some(prepared) = &artifact.prepared_packet {
        anyhow::ensure!(
            prepared.messages_hash == artifact.packet_hash
                && format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&prepared.messages)?)
                ) == artifact.packet_hash,
            "persisted packet message hash mismatch"
        );
        anyhow::ensure!(
            prepared.packet.project_id == artifact.project_id
                && prepared.packet.branch_id == artifact.branch_id
                && prepared.packet.scene_card.rating == artifact.rating,
            "persisted packet identity mismatch"
        );
    }
    if let Some(accepted) = &artifact.accepted_text_sha256 {
        anyhow::ensure!(
            !artifact.truncated && accepted == &artifact.text_sha256,
            "stale or incomplete acceptance"
        );
    }
    if let Some(extraction) = &artifact.extraction {
        anyhow::ensure!(
            artifact.accepted_text_sha256.as_ref() == Some(&extraction.accepted_text_sha256),
            "stale extraction"
        );
    }
    Ok(())
}
pub fn accept_text(
    artifact: &mut PlainTextDraftArtifactV1,
    expected_hash: &str,
) -> anyhow::Result<()> {
    validate_artifact(artifact)?;
    anyhow::ensure!(!artifact.truncated, "incomplete draft cannot be accepted");
    anyhow::ensure!(
        !artifact.text.trim().is_empty(),
        "empty draft cannot be accepted"
    );
    anyhow::ensure!(
        !artifact.text.trim_start().starts_with("```")
            && serde_json::from_str::<serde_json::Value>(&artifact.text).is_err(),
        "draft must be plain scene prose"
    );
    anyhow::ensure!(
        expected_hash == artifact.text_sha256,
        "stale acceptance hash"
    );
    artifact.accepted_text_sha256 = Some(expected_hash.into());
    Ok(())
}
/// Hash-checked acceptance persisted under the same lock as extraction.
pub fn accept_artifact(
    path: &Path,
    expected_hash: &str,
) -> anyhow::Result<PlainTextDraftArtifactV1> {
    let _lock = lock_artifact(path)?;
    let mut artifact = load_artifact(path)?;
    accept_text(&mut artifact, expected_hash)?;
    save_artifact(path, &artifact)?;
    Ok(artifact)
}

/// Atomic snapshots in the existing local artifact layer, not a canon/event store.
pub fn save_artifact(path: &Path, artifact: &PlainTextDraftArtifactV1) -> anyhow::Result<()> {
    validate_artifact(artifact)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), artifact)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}
pub fn load_artifact(path: &Path) -> anyhow::Result<PlainTextDraftArtifactV1> {
    let artifact = serde_json::from_reader(std::fs::File::open(path)?)?;
    validate_artifact(&artifact)?;
    Ok(artifact)
}
pub async fn extract_accepted(
    path: &Path,
    extractor: &dyn AcceptedTextExtractor,
    selected_canon: Vec<PacketItem>,
    known_entities: Vec<ExtractionEntityRef>,
) -> anyhow::Result<PlainTextDraftArtifactV1> {
    let _lock = lock_artifact(path)?;
    let mut artifact = load_artifact(path)?;
    anyhow::ensure!(
        artifact.accepted_text_sha256.as_ref() == Some(&artifact.text_sha256),
        "only accepted text may be extracted"
    );
    let accepted = AcceptedDraftV1 {
        artifact_id: artifact.artifact_id.clone(),
        project_id: artifact.project_id.clone(),
        branch_id: artifact.branch_id.clone(),
        scene_id: artifact.scene_id.clone(),
        packet_hash: artifact.packet_hash.clone(),
        text: artifact.text.clone(),
        text_sha256: artifact.text_sha256.clone(),
        rating: artifact.rating.clone(),
        receipt: artifact.receipt.clone(),
    };
    let request = AcceptedTextExtractionRequestV1 {
        task_id: format!("extract:{}", artifact.artifact_id),
        accepted,
        selected_canon,
        known_entities,
    };
    if let Some(prior) = &artifact.extraction_request {
        anyhow::ensure!(
            serde_json::to_vec(prior)? == serde_json::to_vec(&request)?,
            "extraction retry inputs changed"
        );
    }
    if artifact.extraction.is_some()
        || artifact
            .extraction_failure
            .as_ref()
            .is_some_and(|f| !f.retryable)
    {
        return Ok(artifact);
    }
    artifact.extraction_request = Some(request.clone());
    artifact.extraction_attempts = artifact
        .extraction_attempts
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("extraction attempt overflow"))?;
    save_artifact(path, &artifact)?;
    match extractor.extract(&request).await {
        Ok(metadata)
            if metadata.accepted_text_sha256 == artifact.text_sha256
                && !metadata.summary.trim().is_empty() =>
        {
            artifact.extraction = Some(metadata);
            artifact.extraction_failure = None;
        }
        Ok(_) => {
            artifact.extraction_failure = Some(ExtractionFailure {
                code: "invalid_accepted_text_binding".into(),
                retryable: false,
            });
        }
        Err(error) => {
            artifact.extraction_failure = Some(error);
        }
    }
    save_artifact(path, &artifact)?;
    Ok(artifact)
}
