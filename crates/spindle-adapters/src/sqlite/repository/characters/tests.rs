use super::*;

#[tokio::test]
async fn create_character_inserts_all_four_tables_atomically() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CreateCharacterInput, FlexRange,
    };
    let (_tmp, repo) = fresh_repo().await;
    let (project, _branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "P".into(),
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

    let (character, voice, emotional, state) = repo
        .create_character(&CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.id.clone(),
            name: "Mara".into(),
            summary: "An oathbound warden.".into(),
            role: "protagonist".into(),
            realm: Some("Marches".into()),
            voice_profile: CharacterVoiceProfileData {
                tone: Some("grim".into()),
                vocabulary: vec!["oath".into(), "ash".into()],
                sentence_structure: vec!["clipped".into()],
                tics: vec!["touches scar".into()],
                forbidden_words: Vec::new(),
                example_lines: vec!["I know the cost.".into()],
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: vec!["fear".into()],
                triggers: Vec::new(),
                defense_mechanisms: vec!["silence".into()],
                flex_range: Some(FlexRange {
                    low: Some("withdrawn".into()),
                    high: Some("explosive".into()),
                }),
            },
            initial_state: Some(CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: Some(vec!["keep the gate".into()]),
                status: Some(vec!["wary".into()]),
                notes: None,
                source_summary: Some("introduction".into()),
            }),
        })
        .await
        .unwrap();

    assert!(character.id.starts_with("character:"));
    assert_eq!(character.normalized_name, "mara");
    assert_eq!(voice.character_id, character.id);
    assert_eq!(voice.tone.as_deref(), Some("grim"));
    assert_eq!(emotional.character_id, character.id);
    assert!(emotional.flex_range.is_some());
    assert_eq!(state.character_id, character.id);
    assert_eq!(state.goals, vec!["keep the gate".to_string()]);
    assert_eq!(state.source_summary.as_deref(), Some("introduction"));

    // Verify the indirect access paths see the same rows.
    let voice_again = repo
        .get_character_voice_profile(&character.id)
        .await
        .unwrap();
    assert_eq!(voice_again.id, voice.id);
    let emo_again = repo
        .get_character_emotional_profile(&character.id)
        .await
        .unwrap();
    assert_eq!(emo_again.id, emotional.id);
}
