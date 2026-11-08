//! P1's consumer contract. P2 implements extraction; nothing here interprets prose.
use super::PacketItem;
use crate::models::{
    AnnotatedBeatInput, CanonicalFactEntry, CharacterStatePatchEntry, ContentRating,
    RelationshipUpdateEntry,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DraftReceiptBinding {
    pub generation_id: String,
    pub agent_id: String,
    /// Existing receipt hash: CRLF normalized and outer whitespace trimmed.
    pub normalized_output_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcceptedDraftV1 {
    pub artifact_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: Option<String>,
    pub packet_hash: String,
    pub text: String,
    /// SHA-256 over exact UTF-8 bytes, including whitespace and line endings.
    pub text_sha256: String,
    pub rating: ContentRating,
    pub receipt: DraftReceiptBinding,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractionEntityRef {
    pub id: String,
    pub kind: String,
    pub display_name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcceptedTextExtractionRequestV1 {
    /// Stable across retries; includes the accepted hash, never a candidate's hash.
    pub task_id: String,
    pub accepted: AcceptedDraftV1,
    pub selected_canon: Vec<PacketItem>,
    pub known_entities: Vec<ExtractionEntityRef>,
}
/// Proposed compatibility metadata; application uses existing save/commit gates.
/// Extraction cannot replace the accepted prose or its generation identity.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompatibilityExtractionV1 {
    pub accepted_text_sha256: String,
    pub summary: String,
    pub tone: Option<String>,
    pub character_states: Vec<CharacterStatePatchEntry>,
    pub canonical_facts: Vec<CanonicalFactEntry>,
    pub relationship_updates: Vec<RelationshipUpdateEntry>,
    pub beats: Vec<AnnotatedBeatInput>,
    pub continuity_notes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractionFailure {
    pub code: String,
    pub retryable: bool,
}
/// The caller persists accepted text before invoking this contract. Implementors
/// must honor task id/hash idempotency and rating clearance, preserve the receipt,
/// and never apply canon. P2 owns schema semantics, validation and dispatch.
pub trait AcceptedTextExtractor: Send + Sync {
    fn extract<'a>(
        &'a self,
        request: &'a AcceptedTextExtractionRequestV1,
    ) -> Pin<
        Box<dyn Future<Output = Result<CompatibilityExtractionV1, ExtractionFailure>> + Send + 'a>,
    >;
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlainTextDraftArtifactV1 {
    pub prepared_packet: Option<super::PreparedDraftingPacket>,
    pub source_packet_sha256: String,
    pub schema_version: String,
    pub artifact_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: Option<String>,
    pub packet_hash: String,
    pub text: String,
    pub text_sha256: String,
    pub rating: ContentRating,
    pub truncated: bool,
    pub receipt: DraftReceiptBinding,
    pub accepted_text_sha256: Option<String>,
    pub extraction_request: Option<AcceptedTextExtractionRequestV1>,
    pub extraction_attempts: u32,
    pub extraction_failure: Option<ExtractionFailure>,
    pub extraction: Option<CompatibilityExtractionV1>,
}
