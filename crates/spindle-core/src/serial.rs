use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EditorialStatus {
    Open,
    Accepted,
    Deferred,
    Resolved,
    Dismissed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EditorialDecision {
    pub status: EditorialStatus,
    pub note: String,
    pub reviewed_source_hash: Option<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EditorialItem {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub source_hash: String,
    pub reader_memory_id: String,
    /// Chapter-level evidence, not a claim that each scene contains the issue.
    pub scene_ids: Vec<String>,
    pub severity: String,
    pub description: String,
    pub status: EditorialStatus,
    pub revision: u32,
    pub decisions: Vec<EditorialDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EditorialItemView {
    pub item: EditorialItem,
    /// None means the source chapter no longer exists.
    pub current_source_hash: Option<String>,
    pub source_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetEditorialQueueInput {
    pub project_id: String,
    pub branch_id: Option<String>,
    /// Omit for open, accepted and deferred work.
    pub status: Option<EditorialStatus>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EditorialQueueOutput {
    pub items: Vec<EditorialItemView>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DecideEditorialItemInput {
    pub project_id: String,
    pub item_id: String,
    pub expected_revision: u32,
    /// Current hash from the queue, including after the manuscript was revised.
    pub reviewed_source_hash: Option<String>,
    pub status: EditorialStatus,
    /// Required for accepted revisions and resolutions; records the author's intent.
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PrepareEpisodeReleaseInput {
    pub project_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReleasedScene {
    pub scene_id: String,
    pub scene_order: i32,
    pub text_sha256: String,
    pub content_rating: String,
    pub draft_origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EpisodeSnapshot {
    pub project_id: String,
    pub branch_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub title: String,
    pub markdown: String,
    pub word_count: usize,
    pub scenes: Vec<ReleasedScene>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EpisodeReleasePreview {
    pub snapshot: EpisodeSnapshot,
    pub source_hash: String,
    pub blocking_issues: Vec<String>,
    pub previous_release_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseEpisodeInput {
    pub project_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    /// From prepare_episode_release; rejects any intervening manuscript change.
    pub expected_source_hash: String,
    /// From the preview. Corrections append a revision; old releases are preserved.
    pub previous_release_id: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EpisodeRelease {
    pub id: String,
    pub revision: u32,
    pub source_hash: String,
    pub previous_release_id: Option<String>,
    pub released_at: String,
    pub note: Option<String>,
    pub snapshot: EpisodeSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetEpisodeReleaseInput {
    pub release_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSeriesStatusInput {
    pub project_id: String,
    pub book_number: Option<i32>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EpisodeStatus {
    pub book_number: i32,
    pub chapter_number: i32,
    pub title: String,
    pub word_count: usize,
    pub ready: bool,
    pub blocking_issues: Vec<String>,
    pub latest_release_id: Option<String>,
    pub release_revision: Option<u32>,
    pub changed_since_release: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SeriesStatusOutput {
    pub project_id: String,
    pub branch_id: String,
    /// Contiguous releases in the current chapter spine; gaps stop this cursor.
    pub published_through: Option<crate::models::StoryPlacement>,
    pub released_episodes: usize,
    pub draft_backlog: usize,
    pub ready_backlog: usize,
    pub episodes: Vec<EpisodeStatus>,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadEpisodeInput {
    pub project_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub branch_id: Option<String>,
    /// Re-read even if an unchanged chapter has a cached reading.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReaderMemoryRecord {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub source_hash: String,
    pub chapters_read: Vec<String>,
    pub open_questions: Vec<String>,
    pub rating: String,
    pub outcome: crate::models::ReaderSimChapterOutcome,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ReaderMemoryTrace {
    pub loaded_memory_id: Option<String>,
    pub stored_memory_id: Option<String>,
    pub source_hash: String,
    pub cached: bool,
    pub stale_records_ignored: usize,
    pub unread_prior_chapters: Vec<String>,
    pub persistence_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadEpisodeOutput {
    pub outcome: crate::models::ReaderSimChapterOutcome,
    pub memory: ReaderMemoryTrace,
}

/// Provider-reported tokens for the final response. Missing means unknown,
/// including CLI adapters; estimates and prices are deliberately separate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ModelUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModelCallRecord {
    pub id: String,
    pub project_id: Option<String>,
    pub scene_id: Option<String>,
    pub route: String,
    pub adapter_kind: Option<String>,
    pub model_name: Option<String>,
    pub outcome: String,
    pub usage: Option<ModelUsage>,
    pub elapsed_ms: u64,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct GetModelUsageInput {
    pub project_id: Option<String>,
    /// Most recent calls returned (1–200); aggregate counts cover all matching calls.
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetModelUsageOutput {
    pub calls: Vec<ModelCallRecord>,
    pub total_calls: u64,
    pub calls_with_unknown_tokens: u64,
    pub known_input_tokens: u64,
    pub known_output_tokens: u64,
    pub elapsed_ms: u64,
    /// Calls without a project association, excluded from a project-specific report.
    pub unattributed_calls: u64,
}

/// Coverage of a requested model-backed check, independent of its findings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct AuditCoverage {
    pub check_type: String,
    pub eligible: usize,
    pub evaluated_ids: Vec<String>,
    /// Cap overflow, unavailable routes and malformed verdicts remain here.
    pub not_evaluated_ids: Vec<String>,
    /// Deterministic fallback is useful but is not a completed model review.
    pub heuristic_ids: Vec<String>,
    pub offset: usize,
    pub next_offset: Option<usize>,
}

impl AuditCoverage {
    pub fn window(&mut self, offset: usize, cap: usize) {
        self.offset = offset;
        self.next_offset = offset.checked_add(cap).filter(|next| *next < self.eligible);
    }
    pub fn new(check_type: &str, ids: impl IntoIterator<Item = String>) -> Self {
        let ids: Vec<_> = ids.into_iter().collect();
        Self {
            check_type: check_type.into(),
            eligible: ids.len(),
            not_evaluated_ids: ids,
            ..Default::default()
        }
    }

    pub fn finish(&mut self) {
        let evaluated: std::collections::HashSet<_> = self.evaluated_ids.iter().collect();
        self.not_evaluated_ids.retain(|id| !evaluated.contains(id));
    }
}
