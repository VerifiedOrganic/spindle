use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use spindle_core::models::{AgencyWarning, ContentRating};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterRange {
    pub start_chapter: i32,
    pub end_chapter: i32,
}

impl ChapterRange {
    pub fn contains(&self, chapter_number: i32) -> bool {
        chapter_number >= self.start_chapter && chapter_number <= self.end_chapter
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessSeed {
    pub project_id: String,
    pub book_number: i32,
    pub range: ChapterRange,
    pub checkpoint_interval: usize,
    #[serde(default)]
    pub editorial_directives: Vec<String>,
    #[serde(default)]
    pub chapters: Vec<ChapterSeed>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterSeed {
    pub chapter_number: i32,
    pub synopsis: String,
    #[serde(default)]
    pub pov_character_id: Option<String>,
    #[serde(default)]
    pub scenes: Vec<SceneSeed>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneSeed {
    pub scene_order: i32,
    #[serde(default)]
    pub character_ids: Vec<String>,
    pub location_id: String,
    pub content_rating: ContentRating,
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessState {
    pub project_id: String,
    pub active_branch_id: String,
    pub book_number: i32,
    pub range: ChapterRange,
    pub checkpoint_interval: usize,
    pub last_checkpoint_end_chapter: i32,
    #[serde(default = "default_artifacts_dir")]
    pub artifacts_dir: String,
    #[serde(default)]
    pub editorial_directives: Vec<String>,
    #[serde(default)]
    pub chapters: Vec<ChapterState>,
    #[serde(default)]
    pub checkpoint_history: Vec<CheckpointRecord>,
}

impl HarnessState {
    pub fn from_seed(seed: HarnessSeed, active_branch_id: String) -> Self {
        let mut state = Self {
            project_id: seed.project_id,
            active_branch_id,
            book_number: seed.book_number,
            range: seed.range.clone(),
            checkpoint_interval: seed.checkpoint_interval,
            last_checkpoint_end_chapter: seed.range.start_chapter - 1,
            artifacts_dir: default_artifacts_dir(),
            editorial_directives: seed.editorial_directives,
            chapters: seed
                .chapters
                .into_iter()
                .map(|chapter| ChapterState {
                    chapter_number: chapter.chapter_number,
                    planned: true,
                    synopsis: chapter.synopsis,
                    pov_character_id: chapter.pov_character_id,
                    status: ChapterStatus::Pending,
                    scenes: chapter
                        .scenes
                        .into_iter()
                        .map(|scene| SceneState {
                            scene_order: scene.scene_order,
                            character_ids: scene.character_ids,
                            location_id: scene.location_id,
                            content_rating: scene.content_rating,
                            tone: scene.tone,
                            source_path: scene.source_path,
                            phase: ScenePhase::Pending,
                            scene_id: None,
                            scene_artifact_path: None,
                            draft_diagnostics: None,
                            blocked_reason: None,
                        })
                        .collect(),
                    summary_saved: false,
                    summary_artifact_path: None,
                })
                .collect(),
            checkpoint_history: Vec::new(),
        };
        state.normalize();
        state
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read harness state {}", path.display()))?;
        let mut state: Self = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse harness state {}", path.display()))?;
        state.normalize();
        Ok(state)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut normalized = self.clone();
        normalized.normalize();
        let json = serde_json::to_string_pretty(&normalized)?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create parent directory {}", parent.display())
            })?;
        }
        fs::write(path, json)
            .with_context(|| format!("failed to write harness state {}", path.display()))?;
        Ok(())
    }

    pub fn normalize(&mut self) {
        if self.artifacts_dir.trim().is_empty() {
            self.artifacts_dir = default_artifacts_dir();
        }
        self.chapters.sort_by_key(|chapter| chapter.chapter_number);
        for chapter in &mut self.chapters {
            chapter.scenes.sort_by_key(|scene| scene.scene_order);
            chapter.recompute_status();
        }
        self.checkpoint_history
            .sort_by_key(|checkpoint| (checkpoint.start_chapter, checkpoint.end_chapter));
    }

    pub fn completed_chapter_count(&self) -> usize {
        self.chapters
            .iter()
            .filter(|chapter| chapter.summary_saved)
            .count()
    }

    #[cfg(test)]
    pub fn chapter(&self, chapter_number: i32) -> Option<&ChapterState> {
        self.chapters
            .iter()
            .find(|chapter| chapter.chapter_number == chapter_number)
    }

    #[cfg(test)]
    pub fn chapter_mut(&mut self, chapter_number: i32) -> Option<&mut ChapterState> {
        self.chapters
            .iter_mut()
            .find(|chapter| chapter.chapter_number == chapter_number)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterState {
    pub chapter_number: i32,
    pub planned: bool,
    pub synopsis: String,
    #[serde(default)]
    pub pov_character_id: Option<String>,
    pub status: ChapterStatus,
    #[serde(default)]
    pub scenes: Vec<SceneState>,
    #[serde(default)]
    pub summary_saved: bool,
    #[serde(default)]
    pub summary_artifact_path: Option<String>,
}

impl ChapterState {
    pub fn recompute_status(&mut self) {
        self.status = if self.summary_saved {
            ChapterStatus::Complete
        } else if self
            .scenes
            .iter()
            .any(|scene| scene.phase != ScenePhase::Pending)
        {
            ChapterStatus::InProgress
        } else {
            ChapterStatus::Pending
        };
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChapterStatus {
    Pending,
    InProgress,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneState {
    pub scene_order: i32,
    #[serde(default)]
    pub character_ids: Vec<String>,
    pub location_id: String,
    pub content_rating: ContentRating,
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    pub phase: ScenePhase,
    #[serde(default)]
    pub scene_id: Option<String>,
    #[serde(default)]
    pub scene_artifact_path: Option<String>,
    #[serde(default)]
    pub draft_diagnostics: Option<SceneDraftDiagnostics>,
    #[serde(default)]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ScenePhase {
    Pending,
    DraftSaved,
    ChangesCommitted,
    BeatsAnnotated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub start_chapter: i32,
    pub end_chapter: i32,
    pub save_point_id: String,
    pub status: CheckpointStatus,
    #[serde(default)]
    pub report_artifact_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointStatus {
    PendingReview,
    Reviewed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneDraftDiagnostics {
    #[serde(default)]
    pub pacing_warnings: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_agency_warning_compat")]
    pub agency_warning: Option<AgencyWarning>,
    #[serde(default)]
    pub tone_deviation: bool,
    #[serde(default)]
    pub content_rating_valid: bool,
    #[serde(default)]
    pub content_rating_warnings: Vec<String>,
}

pub fn load_seed(path: &Path) -> Result<HarnessSeed> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read harness seed {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse harness seed {}", path.display()))
}

fn default_artifacts_dir() -> String {
    "spindle-harness-artifacts".to_string()
}

fn deserialize_agency_warning_compat<'de, D>(
    deserializer: D,
) -> Result<Option<AgencyWarning>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum AgencyWarningRepr {
        Typed(AgencyWarning),
        LegacyMessage(String),
        Null,
    }

    match AgencyWarningRepr::deserialize(deserializer)? {
        AgencyWarningRepr::Typed(warning) => Ok(Some(warning)),
        AgencyWarningRepr::LegacyMessage(message) => Ok(Some(AgencyWarning {
            kind: spindle_core::models::AgencyWarningKind::Passive,
            message,
            character_id: None,
            character_name: None,
            evidence: Vec::new(),
            suggestion: String::new(),
        })),
        AgencyWarningRepr::Null => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn load_supports_legacy_string_agency_warning_shape() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "spindle-harness-state-legacy-agency-warning-{}-{}.json",
            std::process::id(),
            unique
        ));

        let state_json = serde_json::json!({
            "project_id": "project:p1",
            "active_branch_id": "bible_branch:feature",
            "book_number": 1,
            "range": {
                "start_chapter": 1,
                "end_chapter": 1
            },
            "checkpoint_interval": 1,
            "last_checkpoint_end_chapter": 0,
            "artifacts_dir": "spindle-harness-artifacts",
            "editorial_directives": [],
            "chapters": [{
                "chapter_number": 1,
                "planned": true,
                "synopsis": "Legacy diagnostics fixture.",
                "status": "in_progress",
                "scenes": [{
                    "scene_order": 1,
                    "character_ids": [],
                    "location_id": "location:gate",
                    "content_rating": "teen",
                    "phase": "draft_saved",
                    "scene_id": "scene:s1",
                    "scene_artifact_path": "scenes/ch01-s01.json",
                    "draft_diagnostics": {
                        "pacing_warnings": [],
                        "agency_warning": "Legacy warning string payload",
                        "tone_deviation": false,
                        "content_rating_valid": true,
                        "content_rating_warnings": []
                    },
                    "blocked_reason": null
                }],
                "summary_saved": false,
                "summary_artifact_path": null
            }],
            "checkpoint_history": []
        });
        fs::write(
            &path,
            serde_json::to_string_pretty(&state_json).expect("serialize fixture"),
        )
        .expect("write fixture");

        let loaded = HarnessState::load(&path).expect("legacy fixture should deserialize");
        let warning = loaded.chapters[0].scenes[0]
            .draft_diagnostics
            .as_ref()
            .and_then(|diagnostics| diagnostics.agency_warning.as_ref())
            .expect("legacy warning should map into typed warning");
        assert_eq!(
            warning.kind,
            spindle_core::models::AgencyWarningKind::Passive
        );
        assert_eq!(warning.message, "Legacy warning string payload");
        assert_eq!(warning.character_id, None);
        assert!(warning.evidence.is_empty());

        let _ = fs::remove_file(&path);
    }
}
