use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use spindle_core::models::{
    AnnotateSceneBeatsOutput, AnnotatedBeatInput, CanonicalFactEntry, CharacterStatePatchEntry,
    CommitSceneChangesOutput, CreateSavePointOutput, RelationshipUpdateEntry, ResearchClaim,
    ResearchNote, ResearchSource, SaveSceneDraftOutput, SaveSummaryOutput,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SceneGenerationArtifact {
    pub version: u32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub route_name: String,
    pub agent_id: String,
    #[serde(default)]
    pub rating: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub completion_fragments: Vec<String>,
    #[serde(default)]
    pub adapter_kind: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub generation_id: Option<String>,
    #[serde(default)]
    pub generation_agent_id: Option<String>,
    #[serde(default)]
    pub generation_output_sha256: Option<String>,
    #[serde(default)]
    pub last_parse_error: Option<String>,
    #[serde(default)]
    pub package: Option<GeneratedScenePackage>,
    #[serde(default)]
    pub save_draft_output: Option<SaveSceneDraftOutput>,
    #[serde(default)]
    pub commit_output: Option<CommitSceneChangesOutput>,
    #[serde(default)]
    pub beat_annotation_output: Option<AnnotateSceneBeatsOutput>,
    #[serde(default)]
    pub research_source_ids: Vec<String>,
    #[serde(default)]
    pub research_note_ids: Vec<String>,
    #[serde(default)]
    pub research_claim_ids: Vec<String>,
    #[serde(default)]
    pub research_query_pack_input: Option<String>,
    #[serde(default)]
    pub research_context_hash: Option<String>,
    #[serde(default)]
    pub research_sources: Vec<ResearchSource>,
    #[serde(default)]
    pub research_notes: Vec<ResearchNote>,
    #[serde(default)]
    pub research_claims: Vec<ResearchClaim>,
}

impl SceneGenerationArtifact {
    pub fn new(
        chapter_number: i32,
        scene_order: i32,
        route_name: String,
        agent_id: String,
        rating: Option<String>,
        prompt: String,
    ) -> Self {
        Self {
            version: 1,
            chapter_number,
            scene_order,
            route_name,
            agent_id,
            rating,
            prompt,
            completion_fragments: Vec::new(),
            adapter_kind: None,
            model_name: None,
            truncated: true,
            generation_id: None,
            generation_agent_id: None,
            generation_output_sha256: None,
            last_parse_error: None,
            package: None,
            save_draft_output: None,
            commit_output: None,
            beat_annotation_output: None,
            research_source_ids: Vec::new(),
            research_note_ids: Vec::new(),
            research_claim_ids: Vec::new(),
            research_query_pack_input: None,
            research_context_hash: None,
            research_sources: Vec::new(),
            research_notes: Vec::new(),
            research_claims: Vec::new(),
        }
    }

    pub fn combined_output(&self) -> String {
        self.completion_fragments.concat()
    }

    pub fn is_ready(&self) -> bool {
        self.package.is_some()
    }

    /// Discard the stored generation so the next `ensure_scene_package_ready`
    /// re-dispatches a fresh draft instead of re-parsing the cached (poisoned)
    /// completion forever (BUG 3). Resets the generation receipt and restores the
    /// `truncated == true` sentinel a freshly-created artifact carries, so the
    /// scheduler treats the scene as pending-draft. Preserves the prompt and the
    /// research context (source/note/claim ids and hash) — those are still valid
    /// for the re-dispatch. Never touches `save_draft_output`; a scene that
    /// already saved a draft is past this path.
    pub fn clear_generation(&mut self) {
        self.completion_fragments.clear();
        self.truncated = true;
        self.adapter_kind = None;
        self.model_name = None;
        self.generation_id = None;
        self.generation_agent_id = None;
        self.generation_output_sha256 = None;
        self.package = None;
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GeneratedScenePackage {
    pub full_text: String,
    pub summary: String,
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub character_states: Vec<CharacterStatePatchEntry>,
    #[serde(default)]
    pub canonical_facts: Vec<CanonicalFactEntry>,
    #[serde(default)]
    pub relationship_updates: Vec<RelationshipUpdateEntry>,
    #[serde(default)]
    pub beats: Vec<AnnotatedBeatInput>,
    #[serde(default)]
    pub continuity_notes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChapterSummaryArtifact {
    pub version: u32,
    pub chapter_number: i32,
    pub route_name: String,
    pub agent_id: String,
    pub prompt: String,
    #[serde(default)]
    pub completion_fragments: Vec<String>,
    #[serde(default)]
    pub adapter_kind: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub last_parse_error: Option<String>,
    #[serde(default)]
    pub package: Option<GeneratedChapterSummaryPackage>,
    #[serde(default)]
    pub save_summary_output: Option<SaveSummaryOutput>,
    /// The authoring run that produced this artifact. An artifact found on disk
    /// with a DIFFERENT run id is residue from an earlier pass — its package and
    /// save_summary_output must not be honored as idempotency proof for the
    /// current run (defect item 2). `None` on artifacts written before stamping
    /// existed (those fall back to the persisted-row existence check).
    #[serde(default)]
    pub run_id: Option<String>,
}

impl ChapterSummaryArtifact {
    pub fn new(chapter_number: i32, route_name: String, agent_id: String, prompt: String) -> Self {
        Self {
            version: 1,
            chapter_number,
            route_name,
            agent_id,
            prompt,
            completion_fragments: Vec::new(),
            adapter_kind: None,
            model_name: None,
            truncated: true,
            last_parse_error: None,
            package: None,
            save_summary_output: None,
            run_id: None,
        }
    }

    pub fn combined_output(&self) -> String {
        self.completion_fragments.concat()
    }

    pub fn is_ready(&self) -> bool {
        self.package.is_some()
    }

    /// Discard the stored generation so the next `ensure_summary_package_ready`
    /// re-dispatches a fresh summary instead of re-parsing the cached (poisoned)
    /// completion forever (BUG 3, mirror of the scene path).
    pub fn clear_generation(&mut self) {
        self.completion_fragments.clear();
        self.truncated = true;
        self.adapter_kind = None;
        self.model_name = None;
        self.package = None;
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GeneratedChapterSummaryPackage {
    pub summary: String,
    #[serde(default)]
    pub key_events: Vec<String>,
    #[serde(default)]
    pub character_changes: Vec<String>,
    #[serde(default)]
    pub relationship_shifts: Vec<String>,
    #[serde(default)]
    pub arc_advances: Vec<String>,
    #[serde(default)]
    pub promise_events: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointReportArtifact {
    pub version: u32,
    pub start_chapter: i32,
    pub end_chapter: i32,
    pub save_point: CreateSavePointOutput,
    pub consistency: serde_json::Value,
    #[serde(default)]
    pub deep_consistency: Option<serde_json::Value>,
    #[serde(default)]
    pub deep_consistency_status: String,
    #[serde(default)]
    pub deep_consistency_instruction: String,
    #[serde(default)]
    pub sampled_reviews: Vec<serde_json::Value>,
    #[serde(default)]
    pub sampled_review_status: String,
    #[serde(default)]
    pub sampled_review_instruction: String,
    pub pacing_overview: serde_json::Value,
    pub chapter_summaries: serde_json::Value,
    pub narrative_promises: serde_json::Value,
    #[serde(default)]
    pub sampled_scene_ids: Vec<String>,
    /// Cumulative reader-simulation section (evolution §3.6, R3). Additive,
    /// serde-default so a pre-reader-sim report round-trips unchanged. Populated
    /// by the auto-checkpoint automation after the sampled reviews and before the
    /// verdict; report-only (reader-sim concerns never fold into the verdict
    /// counts, matching the sampled-review outcomes). `None` on a report that
    /// never ran the pass (manual policy, or an older report).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader_sim: Option<CheckpointReaderSimSection>,
}

/// The reader-simulation section of a checkpoint report (evolution §3.6). One
/// per-chapter entry in checkpoint-range order plus the path to the run's
/// rolling reader-sim notes artifact, so an operator can read the reader's full
/// cumulative memory. Enums/ids/concern-text only — never committed prose (I8).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointReaderSimSection {
    #[serde(default)]
    pub chapters: Vec<CheckpointReaderSimChapter>,
    /// Path (relative to the run's artifacts dir) to `reader-sim-notes.json`.
    pub notes_artifact_path: String,
}

/// One chapter's reader-sim result inside a checkpoint report (evolution §3.6).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointReaderSimChapter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<spindle_core::serial::ReaderMemoryTrace>,
    pub chapter: i32,
    /// `high` | `steady` | `dipping` | `unparsed` | `skipped`.
    pub engagement: String,
    #[serde(default)]
    pub concerns: Vec<ReaderSimConcernEntry>,
    /// Present only when this chapter's pass was skipped; names the route+rating
    /// that was uncleared or the transport failure. Never carries prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

/// A reader-sim concern as recorded in a checkpoint report (evolution §3.6).
/// `severity` is `info` | `warning`; report-only, never a gate.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ReaderSimConcernEntry {
    pub severity: String,
    pub description: String,
}

/// The run's rolling reader-simulation notes artifact (evolution §3.6, R1):
/// one `reader-sim-notes.json` per run in the run's artifacts dir. Carries the
/// reader's cumulative craft memory (the model's own notes, NOT committed
/// prose) plus a per-range history. `updated_through_chapter` is the highest
/// chapter whose read landed into `notes` (`0` before any read). The notes are
/// tail-truncated char-safe to [`READER_SIM_NOTES_CAP`] before being fed back
/// into the next chapter's prompt — the cap governs what WE include, not what
/// the model wrote.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ReaderSimNotesArtifact {
    /// Highest chapter number whose reader-sim read has landed into `notes`.
    /// `0` before any read.
    #[serde(default)]
    pub updated_through_chapter: i32,
    /// The reader's cumulative, self-contained notes (the model's own memory).
    #[serde(default)]
    pub notes: String,
    /// Per-range history entries, appended as each checkpoint runs.
    #[serde(default)]
    pub history: Vec<ReaderSimHistoryEntry>,
}

/// One history entry in the rolling reader-sim notes artifact (evolution §3.6).
/// `range` is a compact `"c..d"` chapter span; `engagement` is the reader's
/// verdict for that chapter (`high`/`steady`/`dipping`/`unparsed`/`skipped`);
/// `concerns_count` is how many concerns the reader raised. Enums/counts only.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReaderSimHistoryEntry {
    pub range: String,
    pub engagement: String,
    pub concerns_count: usize,
}

/// Char cap on the reader-sim notes block WE include in the next chapter's
/// prompt (evolution §3.6, R1). The cap is on our inclusion, not on what the
/// model may write; implementation is a char-safe tail truncation (keep the
/// newest content). 4000 chars keeps the prior-notes block bounded without
/// splitting a multibyte character.
pub const READER_SIM_NOTES_CAP: usize = 4000;

/// The run-relative path to the rolling reader-sim notes artifact (evolution
/// §3.6, R1). One per run, alongside the other run artifacts.
pub const READER_SIM_NOTES_PATH: &str = "reader-sim-notes.json";

/// Tail-truncate `notes` char-safe to at most [`READER_SIM_NOTES_CAP`] chars,
/// keeping the NEWEST content (evolution §3.6, R1). Never splits a multibyte
/// character: truncation happens on a char boundary, so the returned string is
/// always valid UTF-8. A shorter input is returned unchanged.
pub fn cap_reader_sim_notes(notes: &str) -> String {
    let char_count = notes.chars().count();
    if char_count <= READER_SIM_NOTES_CAP {
        return notes.to_string();
    }
    // Keep the last READER_SIM_NOTES_CAP characters (the newest content). Skip
    // by char, not byte, so the boundary always lands on a char boundary.
    notes
        .chars()
        .skip(char_count - READER_SIM_NOTES_CAP)
        .collect()
}

#[derive(Debug, Clone)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn scene_relative_path(chapter_number: i32, scene_order: i32) -> String {
        format!("scenes/chapter-{chapter_number:04}/scene-{scene_order:03}.json")
    }

    pub fn summary_relative_path(chapter_number: i32) -> String {
        format!("summaries/chapter-{chapter_number:04}.json")
    }

    pub fn checkpoint_relative_path(start_chapter: i32, end_chapter: i32) -> String {
        format!("checkpoints/chapter-{start_chapter:04}-{end_chapter:04}.json")
    }

    pub fn load_json<T>(&self, relative_path: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let full_path = self.root.join(relative_path);
        let raw = fs::read_to_string(&full_path)
            .with_context(|| format!("failed to read artifact {}", full_path.display()))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse artifact {}", full_path.display()))
    }

    pub fn save_json<T>(&self, relative_path: &str, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let full_path = self.root.join(relative_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create artifact dir {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(value)?;
        fs::write(&full_path, json)
            .with_context(|| format!("failed to write artifact {}", full_path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod reader_sim_tests {
    use super::*;

    #[test]
    fn cap_leaves_short_notes_untouched() {
        let notes = "short cumulative notes";
        assert_eq!(cap_reader_sim_notes(notes), notes);
    }

    #[test]
    fn cap_at_boundary_keeps_exactly_cap_chars() {
        let notes: String = "a".repeat(READER_SIM_NOTES_CAP);
        let capped = cap_reader_sim_notes(&notes);
        assert_eq!(capped.chars().count(), READER_SIM_NOTES_CAP);
        assert_eq!(capped, notes);
    }

    // Test 6: prior notes over the cap → the included block is ≤ cap chars and
    // char-safe even when a multibyte character straddles the truncation
    // boundary. We build a string whose leading run of ASCII pushes the cut
    // point onto a multibyte char; a byte-truncation would panic / corrupt, a
    // char-safe truncation must not.
    #[test]
    fn cap_over_limit_is_char_safe_at_multibyte_boundary() {
        // One multibyte char (é = 2 bytes) at the front, then enough ASCII to
        // exceed the cap by a handful of chars. The kept tail (newest content)
        // is all ASCII, and the whole result stays valid UTF-8 and ≤ cap.
        let mut notes = String::from("é");
        notes.push_str(&"x".repeat(READER_SIM_NOTES_CAP + 25));
        let capped = cap_reader_sim_notes(&notes);
        assert!(
            capped.chars().count() <= READER_SIM_NOTES_CAP,
            "capped notes must be ≤ cap chars, got {}",
            capped.chars().count()
        );
        assert_eq!(
            capped.chars().count(),
            READER_SIM_NOTES_CAP,
            "an over-cap input is truncated to exactly the cap"
        );
        // Valid UTF-8 by construction (String), and the dropped-oldest content
        // means the leading multibyte é is gone — the tail is the newest run.
        assert!(!capped.contains('é'), "oldest content (é) must be dropped");
        assert!(capped.chars().all(|c| c == 'x'));
    }

    // Test 6 (multibyte at the KEPT boundary): the truncation boundary lands
    // right where a multibyte char begins, so a naive byte slice would split
    // it. The char-safe skip must keep the whole char and never split.
    #[test]
    fn cap_never_splits_a_multibyte_char_at_the_kept_boundary() {
        // Fill the tail with multibyte chars so the kept region begins on one.
        let notes: String = "λ".repeat(READER_SIM_NOTES_CAP + 10);
        let capped = cap_reader_sim_notes(&notes);
        assert_eq!(capped.chars().count(), READER_SIM_NOTES_CAP);
        // Every retained char is the intact multibyte λ — none split.
        assert!(capped.chars().all(|c| c == 'λ'));
    }

    #[test]
    fn notes_artifact_round_trips_with_history() {
        let artifact = ReaderSimNotesArtifact {
            updated_through_chapter: 2,
            notes: "The reader is engaged through chapter 2.".to_string(),
            history: vec![
                ReaderSimHistoryEntry {
                    range: "1..1".to_string(),
                    engagement: "high".to_string(),
                    concerns_count: 0,
                },
                ReaderSimHistoryEntry {
                    range: "2..2".to_string(),
                    engagement: "steady".to_string(),
                    concerns_count: 1,
                },
            ],
        };
        let json = serde_json::to_string(&artifact).unwrap();
        let back: ReaderSimNotesArtifact = serde_json::from_str(&json).unwrap();
        assert_eq!(back.updated_through_chapter, 2);
        assert_eq!(back.history.len(), 2);
        assert_eq!(back.history[1].engagement, "steady");
        assert_eq!(back.history[1].concerns_count, 1);
    }

    #[test]
    fn report_round_trips_without_reader_sim_section() {
        // A pre-reader-sim report (no `reader_sim` key) must still deserialize —
        // the field is serde-default None.
        let raw = r#"{
            "version": 1,
            "start_chapter": 1,
            "end_chapter": 1,
            "save_point": {"save_point_id": "sp1", "branch_id": "b1"},
            "consistency": {},
            "pacing_overview": {},
            "chapter_summaries": {},
            "narrative_promises": {}
        }"#;
        let report: CheckpointReportArtifact = serde_json::from_str(raw).unwrap();
        assert!(report.reader_sim.is_none());
    }
}

#[cfg(test)]
mod clear_generation_tests {
    use super::*;

    #[test]
    fn scene_clear_generation_resets_to_pending_draft_but_keeps_context() {
        let mut artifact = SceneGenerationArtifact::new(
            1,
            1,
            "draft".to_string(),
            "agent:grok".to_string(),
            Some("teen".to_string()),
            "the prompt".to_string(),
        );
        // Simulate a completed-but-unparseable dispatch.
        artifact.completion_fragments = vec!["narration then bad json".to_string()];
        artifact.truncated = false;
        artifact.adapter_kind = Some("grok".to_string());
        artifact.model_name = Some("grok-4.5".to_string());
        artifact.generation_id = Some("gen:1".to_string());
        artifact.generation_agent_id = Some("agent:grok".to_string());
        artifact.generation_output_sha256 = Some("deadbeef".to_string());
        artifact.last_parse_error = Some("model output was not valid JSON".to_string());
        artifact.research_source_ids = vec!["source:1".to_string()];
        artifact.research_context_hash = Some("ctx".to_string());

        artifact.clear_generation();

        // Poisoned generation is gone; the scheduler re-dispatches fresh.
        assert!(artifact.completion_fragments.is_empty());
        assert!(
            artifact.truncated,
            "must restore the pending-draft sentinel"
        );
        assert!(artifact.generation_id.is_none());
        assert!(artifact.generation_agent_id.is_none());
        assert!(artifact.generation_output_sha256.is_none());
        assert!(artifact.adapter_kind.is_none());
        assert!(artifact.model_name.is_none());
        assert!(artifact.package.is_none());
        // Prompt and research context are still valid for the re-dispatch.
        assert_eq!(artifact.prompt, "the prompt");
        assert_eq!(artifact.research_source_ids, vec!["source:1".to_string()]);
        assert_eq!(artifact.research_context_hash.as_deref(), Some("ctx"));
    }

    #[test]
    fn summary_clear_generation_resets_to_pending() {
        let mut artifact = ChapterSummaryArtifact::new(
            1,
            "summary".to_string(),
            "agent:grok".to_string(),
            "the prompt".to_string(),
        );
        artifact.completion_fragments = vec!["bad".to_string()];
        artifact.truncated = false;
        artifact.adapter_kind = Some("grok".to_string());
        artifact.model_name = Some("grok-4.5".to_string());

        artifact.clear_generation();

        assert!(artifact.completion_fragments.is_empty());
        assert!(artifact.truncated);
        assert!(artifact.adapter_kind.is_none());
        assert!(artifact.model_name.is_none());
        assert_eq!(artifact.prompt, "the prompt");
    }
}
