use super::*;

#[tokio::test]
async fn set_relationship_absolute_overwrites_trust_and_tension() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CreateCharacterInput, CreateRelationshipInput,
    };

    let (_tmp, repo) = fresh_repo().await;
    let (project, branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "RelAbs".into(),
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

    async fn make_char(repo: &Repository, project_id: &str, name: &str) -> Character {
        let (character, _, _, _) = repo
            .create_character(&CreateCharacterInput {
                aliases: Vec::new(),
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
        character
    }
    let a = make_char(&repo, &project.id, "Aira").await;
    let b = make_char(&repo, &project.id, "Bren").await;

    // Seed with arbitrary starting numbers via create_relationship.
    repo.create_relationship(
        &branch.id,
        &CreateRelationshipInput {
            character_a_id: a.id.clone(),
            character_b_id: b.id.clone(),
            relationship_type: "ally".into(),
            initial_trust: 10,
            initial_tension: 90,
            dynamics: vec!["initial".into()],
        },
    )
    .await
    .unwrap();

    // Overwrite with absolutes (NOT deltas).
    let updated = repo
        .set_relationship_absolute(
            &branch.id,
            &a.id,
            &b.id,
            75,
            15,
            Some("import-canonical".into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        updated.trust, 75,
        "trust must be the absolute new value, not 10+75"
    );
    assert_eq!(updated.tension, 15);
    assert_eq!(updated.reason.as_deref(), Some("import-canonical"));

    // Reversed orientation must still match the same row.
    let updated_reversed = repo
        .set_relationship_absolute(&branch.id, &b.id, &a.id, 50, 50, None, None)
        .await
        .unwrap();
    assert_eq!(updated_reversed.trust, 50);
    assert_eq!(updated_reversed.tension, 50);
}
