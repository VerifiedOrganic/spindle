use super::*;

#[tokio::test]
async fn assemble_subject_snapshot_character_round_trip() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CreateCharacterInput, FlexRange, StoryPlacement,
    };
    use spindle_core::subject::{Subject, SubjectTable};
    use spindle_core::subject_snapshot::SubjectKindSpecific;

    let (_tmp, repo) = fresh_repo().await;
    let (project, branch, _book, _chapter) = repo
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

    let (character, voice, _emotional, _state) = repo
        .create_character(&CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.id.clone(),
            name: "Mara".into(),
            summary: "An oathbound warden.".into(),
            role: "protagonist".into(),
            realm: Some("Marches".into()),
            voice_profile: CharacterVoiceProfileData {
                tone: Some("grim".into()),
                vocabulary: vec!["oath".into()],
                sentence_structure: vec!["clipped".into()],
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: vec!["I know the cost.".into()],
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: Some(FlexRange {
                    low: None,
                    high: None,
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

    let placement = StoryPlacement {
        book_number: 1,
        chapter_number: 1,
        scene_order: Some(0),
        note: None,
    };
    let subject = Subject::new(SubjectTable::Character, character.id.clone()).unwrap();

    let snapshot = repo
        .assemble_subject_snapshot(&project.id, &branch.id, &subject, &placement)
        .await
        .unwrap();

    assert_eq!(snapshot.display_name(), "Mara");
    assert_eq!(snapshot.at_placement().book_number, 1);
    match snapshot.kind_specific() {
        SubjectKindSpecific::Character(details) => {
            assert_eq!(details.role, "protagonist");
            assert_eq!(details.summary, "An oathbound warden.");
            assert_eq!(details.realm.as_deref(), Some("Marches"));
        }
        other => panic!("expected Character kind, got {other:?}"),
    }

    let voice_summary = snapshot
        .voice_profile()
        .expect("voice profile should be present");
    assert_eq!(voice_summary.tone.as_deref(), Some("grim"));
    assert_eq!(voice_summary.vocabulary, vec!["oath".to_string()]);
    assert_eq!(voice.character_id, character.id);

    let state_summary = snapshot
        .current_state()
        .expect("character state should be present");
    assert_eq!(state_summary.goals, vec!["keep the gate".to_string()]);
    assert_eq!(state_summary.status, vec!["wary".to_string()]);

    // Batch path round-trips identically.
    let batched = repo
        .assemble_subject_snapshots(
            &project.id,
            &branch.id,
            std::slice::from_ref(&subject),
            &placement,
        )
        .await
        .unwrap();
    assert_eq!(batched.len(), 1);
    assert_eq!(batched[0].display_name(), "Mara");
}
