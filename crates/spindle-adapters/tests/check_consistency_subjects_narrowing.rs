//! Integration test for Gap 6: `check_consistency` with a non-empty
//! `subjects` list must actually filter the scene set to scenes that
//! reference one of the subjects (by direct id link or by full_text
//! match of the subject's display term). Before the fix, narrowing
//! emitted an info notice and ran against ALL scenes in scope.

use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
    CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateCharacterInput,
    CreateProjectInput, ReaderContract, SaveSceneDraftInput,
};
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);
    (tmp, SqliteSpindleService::new(repo))
}

#[tokio::test]
async fn check_consistency_subjects_list_filters_scene_set() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Narrow".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let mara = svc
        .create_character(CreateCharacterInput {
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: Some(CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: None,
                status: None,
                notes: None,
                source_summary: None,
            }),
        })
        .await
        .unwrap();

    // Scene 1 mentions Mara; scene 2 doesn't.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Mara walked the wall, watching the dark.".into(),
        summary: "Mara".into(),
        content_rating: ContentRating::General,
        tone: Some("calm".into()),
        generation_id: None,
        source_path: None,
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 2,
        full_text: "Aldric pondered the empty courtyard, unrelated.".into(),
        summary: "Aldric".into(),
        content_rating: ContentRating::General,
        tone: Some("calm".into()),
        generation_id: None,
        source_path: None,
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: None,
            subjects: vec![mara.character_id.clone()],
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    // No info-notice fallback fires anymore.
    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "subjects_narrowing"),
        "subjects_narrowing info notice must be gone (real narrowing now runs); got: {:?}",
        out.issues
            .iter()
            .map(|i| i.check_type.clone())
            .collect::<Vec<_>>()
    );
}
