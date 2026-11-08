//! Compatibility projection of an existing drafter package, without model calls.
use super::GeneratedScenePackage;
use sha2::{Digest, Sha256};
use spindle_core::drafting::extraction::{
    AcceptedTextExtractionRequestV1, AcceptedTextExtractor, CompatibilityExtractionV1,
    ExtractionFailure,
};
use std::{future::Future, pin::Pin};

/// One immutable legacy task. The caller still persists accepted text and the
/// result through the shared artifact lifecycle. No independent audit is claimed.
pub struct LegacyPackageExtractor {
    request_sha256: String,
    result: CompatibilityExtractionV1,
}
fn fingerprint(request: &AcceptedTextExtractionRequestV1) -> anyhow::Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(request)?)
    ))
}
impl LegacyPackageExtractor {
    pub fn new(
        request: &AcceptedTextExtractionRequestV1,
        package: &GeneratedScenePackage,
    ) -> anyhow::Result<Self> {
        let accepted = &request.accepted;
        anyhow::ensure!(
            !request.task_id.trim().is_empty()
                && !accepted.receipt.generation_id.trim().is_empty()
                && !accepted.receipt.agent_id.trim().is_empty()
                && !package.summary.trim().is_empty()
                && package.full_text == accepted.text
                && format!("{:x}", Sha256::digest(accepted.text.as_bytes()))
                    == accepted.text_sha256
                && format!(
                    "{:x}",
                    Sha256::digest(accepted.text.replace("\r\n", "\n").trim().as_bytes())
                ) == accepted.receipt.normalized_output_sha256,
            "legacy package is not bound to accepted text and receipt"
        );
        Ok(Self {
            request_sha256: fingerprint(request)?,
            result: CompatibilityExtractionV1 {
                canon_audit: None,
                accepted_text_sha256: accepted.text_sha256.clone(),
                summary: package.summary.clone(),
                tone: package.tone.clone(),
                character_states: package.character_states.clone(),
                canonical_facts: package.canonical_facts.clone(),
                relationship_updates: package.relationship_updates.clone(),
                beats: package.beats.clone(),
                continuity_notes: package.continuity_notes.clone(),
            },
        })
    }
}
impl AcceptedTextExtractor for LegacyPackageExtractor {
    fn extract<'a>(
        &'a self,
        request: &'a AcceptedTextExtractionRequestV1,
    ) -> Pin<
        Box<dyn Future<Output = Result<CompatibilityExtractionV1, ExtractionFailure>> + Send + 'a>,
    > {
        Box::pin(async move {
            if fingerprint(request).ok().as_ref() != Some(&self.request_sha256) {
                return Err(ExtractionFailure {
                    code: "legacy_task_identity_changed".into(),
                    retryable: false,
                });
            }
            Ok(self.result.clone())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spindle_core::drafting::extraction::PlainTextDraftArtifactV1;
    fn fixture() -> (AcceptedTextExtractionRequestV1, GeneratedScenePackage) {
        let text = "  Veyra returned.\r\n";
        let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
        let normalized = format!(
            "{:x}",
            Sha256::digest(text.replace("\r\n", "\n").trim().as_bytes())
        );
        let request = serde_json::from_value(serde_json::json!({
            "task_id":"extract:draft1", "accepted":{
                "placement":{"book_number":1,"chapter_number":2,"scene_order":3,"note":null},
                "artifact_id":"draft1", "project_id":"project:one", "branch_id":"branch:one", "scene_id":null,
                "packet_hash":"packet1", "text":text,"text_sha256":hash,"rating":"general",
                "receipt":{"generation_id":"generation1","agent_id":"agent1","normalized_output_sha256":normalized}
            },"selected_canon":[],"known_entities":[]
        })).unwrap();
        let package = serde_json::from_value(serde_json::json!({
            "full_text":text, "summary":"Veyra returns.","tone":"tense",
            "character_states":[{"character_id":"character:veyra","changes":{"status":["alive"]}}],
            "canonical_facts":[{"subject_id":"character:veyra","predicate":"location","value_text":"home"}],
            "relationship_updates":[{"character_a_id":"character:veyra","character_b_id":"character:other","trust_delta":2,"tension_delta":-1,"reason":"Returned."}],
            "beats":[{"beat_type":"turn","summary":"The return."}],"continuity_notes":["Existing drafter note."]
        })).unwrap();
        (request, package)
    }
    #[tokio::test]
    async fn legacy_port_round_trips_the_current_package_without_new_semantics() {
        let (request, package) = fixture();
        let adapter = LegacyPackageExtractor::new(&request, &package).unwrap();
        let result = adapter.extract(&request).await.unwrap();
        assert!(result.canon_audit.is_none());
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::to_value(adapter.extract(&request).await.unwrap()).unwrap()
        );
        let accepted = &request.accepted;
        let artifact = PlainTextDraftArtifactV1 {
            placement: accepted.placement.clone(),
            prepared_packet: None,
            source_packet_sha256: "source".into(),
            schema_version: "plain_text_draft_v1".into(),
            artifact_id: accepted.artifact_id.clone(),
            project_id: accepted.project_id.clone(),
            branch_id: accepted.branch_id.clone(),
            scene_id: accepted.scene_id.clone(),
            packet_hash: accepted.packet_hash.clone(),
            text: accepted.text.clone(),
            text_sha256: accepted.text_sha256.clone(),
            rating: accepted.rating.clone(),
            truncated: false,
            receipt: accepted.receipt.clone(),
            accepted_text_sha256: Some(accepted.text_sha256.clone()),
            extraction_request: Some(request.clone()),
            extraction_attempts: 1,
            extraction_failure: None,
            extraction: Some(result),
        };
        assert_eq!(
            serde_json::to_value(GeneratedScenePackage::from_accepted_packet(&artifact).unwrap())
                .unwrap(),
            serde_json::to_value(package).unwrap()
        );
    }
    #[tokio::test]
    async fn legacy_port_rejects_changed_identity_and_mismatched_prose() {
        let (request, mut package) = fixture();
        let adapter = LegacyPackageExtractor::new(&request, &package).unwrap();
        let mut changed = request.clone();
        changed.accepted.rating = spindle_core::models::ContentRating::Explicit;
        assert_eq!(
            adapter.extract(&changed).await.unwrap_err().code,
            "legacy_task_identity_changed"
        );
        changed = request.clone();
        changed.accepted.receipt.generation_id.push('2');
        assert!(!adapter.extract(&changed).await.unwrap_err().retryable);
        package.full_text.push(' ');
        assert!(LegacyPackageExtractor::new(&request, &package).is_err());
    }
}
