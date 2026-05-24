//! Integration test for Gap 3: `Repository::set_relationship_absolute`
//! writes absolute trust/tension values onto an existing relationship
//! row, distinct from `update_relationship` which applies deltas.
//! Before the fix, `import_hydrate_bible` silently skipped existing
//! relationships because applying its absolutes as deltas would have
//! corrupted canon.

use spindle_adapters::sqlite::{Repository, SqlitePool};
use spindle_core::models::{
    CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
    CreateCharacterInput, CreateProjectInput, CreateRelationshipInput, ReaderContract,
};
use tempfile::TempDir;

async fn fresh_repo() -> (TempDir, Repository) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("repo.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);
    (tmp, repo)
}

async fn seed_character(repo: &Repository, project_id: &str, name: &str) -> String {
    let (character, _, _, _) = repo
        .create_character(&CreateCharacterInput {
            project_id: project_id.to_string(),
            name: name.to_string(),
            summary: "x".into(),
            role: "x".into(),
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
    character.id
}

#[tokio::test]
async fn set_relationship_absolute_overwrites_existing_trust_and_tension() {
    let (_tmp, repo) = fresh_repo().await;
    let (project, branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "RelAbsIntegration".into(),
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
    let a = seed_character(&repo, &project.id, "Aira").await;
    let b = seed_character(&repo, &project.id, "Bren").await;

    // Seed a baseline (trust=10, tension=90).
    repo.create_relationship(
        &branch.id,
        &CreateRelationshipInput {
            character_a_id: a.clone(),
            character_b_id: b.clone(),
            relationship_type: "ally".into(),
            initial_trust: 10,
            initial_tension: 90,
            dynamics: vec!["initial".into()],
        },
    )
    .await
    .unwrap();

    // Apply absolutes (75, 15). NOT 10+75 = 85; NOT 90+15 = 105.
    let updated = repo
        .set_relationship_absolute(
            &branch.id,
            &a,
            &b,
            75,
            15,
            Some("import-canonical".into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(updated.trust, 75, "absolute trust, not 10+75 delta");
    assert_eq!(updated.tension, 15, "absolute tension, not 90+15 delta");
    assert_eq!(updated.reason.as_deref(), Some("import-canonical"));

    // Reversed orientation must hit the same row.
    let again = repo
        .set_relationship_absolute(&branch.id, &b, &a, 50, 50, None, None)
        .await
        .unwrap();
    assert_eq!(again.trust, 50);
    assert_eq!(again.tension, 50);
}
