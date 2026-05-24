use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use spindle_core::models::{
    AnnotateSceneBeatsOutput, AnnotatedBeatInput, CanonicalFactEntry, CharacterStatePatchEntry,
    CommitSceneChangesOutput, CreateSavePointOutput, RelationshipUpdateEntry, SaveSceneDraftOutput,
    SaveSummaryOutput,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SceneGenerationArtifact {
    pub version: u32,
    pub chapter_number: i32,
    pub scene_order: i32,
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
    pub package: Option<GeneratedScenePackage>,
    #[serde(default)]
    pub save_draft_output: Option<SaveSceneDraftOutput>,
    #[serde(default)]
    pub commit_output: Option<CommitSceneChangesOutput>,
    #[serde(default)]
    pub beat_annotation_output: Option<AnnotateSceneBeatsOutput>,
}

impl SceneGenerationArtifact {
    pub fn new(
        chapter_number: i32,
        scene_order: i32,
        route_name: String,
        agent_id: String,
        prompt: String,
    ) -> Self {
        Self {
            version: 1,
            chapter_number,
            scene_order,
            route_name,
            agent_id,
            prompt,
            completion_fragments: Vec::new(),
            adapter_kind: None,
            model_name: None,
            truncated: true,
            last_parse_error: None,
            package: None,
            save_draft_output: None,
            commit_output: None,
            beat_annotation_output: None,
        }
    }

    pub fn combined_output(&self) -> String {
        self.completion_fragments.concat()
    }

    pub fn is_ready(&self) -> bool {
        self.package.is_some()
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
        }
    }

    pub fn combined_output(&self) -> String {
        self.completion_fragments.concat()
    }

    pub fn is_ready(&self) -> bool {
        self.package.is_some()
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
    pub sampled_reviews: Vec<serde_json::Value>,
    pub pacing_overview: serde_json::Value,
    pub chapter_summaries: serde_json::Value,
    pub narrative_promises: serde_json::Value,
    #[serde(default)]
    pub sampled_scene_ids: Vec<String>,
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
