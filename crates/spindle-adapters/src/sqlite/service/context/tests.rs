use super::*;

#[tokio::test]
async fn get_scene_context_assembles_layers_from_real_scene() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        ContentRating, ContextFormat, CreateCharacterInput, CreateLocationInput,
        GetSceneContextInput, SaveSceneDraftInput, WorldStateInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "SceneCtx".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Oathbound wardens hold the line.".into(),
                style_notes: vec!["grim".into()],
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Oathbound warden of the Ash Gate.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: Some("grim".into()),
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
                goals: Some(vec!["hold the gate".into()]),
                status: Some(vec!["watching".into()]),
                notes: None,
                source_summary: None,
            }),
        })
        .await
        .unwrap();

    let gate = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A blackened wall holding back the dark.".into(),
            initial_state: WorldStateInput {
                controlling_faction: None,
                status: Some("tense".into()),
                prosperity: None,
                stability: Some("fragile".into()),
                threat_level: Some("high".into()),
                sensory_details: vec!["smell of ash".into()],
            },
        })
        .await
        .unwrap();

    // Save one prior scene so the agency check has history to inspect.
    let _ = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate, watching the dark.".into(),
            summary: "Mara holds the gate".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            character_ids: vec![mara.character_id.clone()],
            max_character_count: None,
            location_id: gate.location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(4000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    assert_eq!(ctx.scene.location.location_id, gate.location_id);
    assert_eq!(ctx.scene.location.name, "Ash Gate");
    assert_eq!(ctx.scene.characters.len(), 1);
    assert_eq!(ctx.scene.characters[0].character_id, mara.character_id);
    assert_eq!(
        ctx.scene.world_state.status.as_deref(),
        Some("tense"),
        "world state should be derived from the paired world_state row"
    );
    assert_eq!(
        ctx.novel.reader_contract.promise,
        "Oathbound wardens hold the line."
    );
    assert_eq!(ctx.budget.token_budget, Some(4000));
    let digest = ctx
        .compact_shelf_digest
        .as_ref()
        .expect("writing packet must include compact_shelf_digest");
    assert_eq!(digest.shelves.len(), 12);
    assert!(
        digest
            .shelves
            .iter()
            .any(|shelf| shelf.id == "solitary_fade"
                && shelf.severity == spindle_core::style::antislop::Severity::Soft
                && shelf.enabled)
    );
    assert_eq!(digest.catalog_uri, "bible://references/anti-slop");
    assert!(digest.rewrite_max_passes <= 2);

    let envelope = svc
        .get_scene_context_envelope(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            character_ids: vec![mara.character_id.clone()],
            max_character_count: None,
            location_id: gate.location_id.clone(),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(4000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    assert_eq!(envelope.scene.location.location_id, gate.location_id);
    assert!(envelope.standards.contains("scene-writer"));
    assert!(
        envelope
            .context_markdown
            .as_deref()
            .is_some_and(|markdown| markdown.contains("# Scene context")
                && markdown.contains("compact_shelf_digest")
                && markdown.contains("solitary_fade")),
        "public envelope should include markdown when requested"
    );
    assert!(
        envelope.standards.contains("compact_shelf_digest"),
        "guidance/standards must carry the shelf digest"
    );
    assert!(envelope.compact_shelf_digest.is_some());
}

#[tokio::test]
async fn get_scene_context_hooks_voice_samples_and_scene_negatives_from_style_profile() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, ContextFormat,
        CreateCharacterInput, CreateLocationInput, GetSceneContextInput, WorldStateInput,
    };
    use spindle_core::style::{
        StyleCorpusMetrics, StyleCorpusSummary, StyleProfileCard, StyleProfileGuidance,
        StyleProfileSourcePolicy, StyleProfileStatus,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "PacketHooks".into(),
            project_type: "novel".into(),
            genre: "comedy".into(),
            reader_contract: ReaderContract {
                promise: "Keep it funny.".into(),
                style_notes: vec!["disable fishing_ending".into()],
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "A warden who tells jokes.".into(),
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
            initial_state: None,
        })
        .await
        .unwrap();
    let gate = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A wall.".into(),
            initial_state: WorldStateInput {
                controlling_faction: None,
                status: None,
                prosperity: None,
                stability: None,
                threat_level: None,
                sensory_details: Vec::new(),
            },
        })
        .await
        .unwrap();

    let guidance = StyleProfileGuidance {
        do_rules: vec!["Name the tool, not the mood.".into()],
        avoid_rules: vec!["a mix of relief and dread".into()],
        prompt_snippet: "Short clauses. Concrete work.".into(),
        ..Default::default()
    };
    let profile_id = "style_profile:packet-hooks".to_string();
    let card = StyleProfileCard {
        profile_id: profile_id.clone(),
        project_id: proj.project_id.clone(),
        name: "Packet hooks".into(),
        status: StyleProfileStatus::Ready,
        created_at: "2026-09-13T00:00:00Z".into(),
        updated_at: "2026-09-13T00:00:00Z".into(),
        corpus: StyleCorpusSummary {
            source_count: 0,
            analyzed_source_count: 0,
            skipped_source_count: 0,
            total_words: 0,
            total_characters: 0,
            chunk_count: 0,
            source_refs: Vec::new(),
            warnings: Vec::new(),
        },
        metrics: StyleCorpusMetrics {
            average_sentence_words: 0.0,
            median_sentence_words: 0.0,
            p90_sentence_words: 0.0,
            average_paragraph_words: 0.0,
            median_paragraph_words: 0.0,
            dialogue_line_ratio: 0.0,
            dialogue_word_ratio: 0.0,
            question_mark_rate_per_1k_words: 0.0,
            exclamation_rate_per_1k_words: 0.0,
            semicolon_rate_per_1k_words: 0.0,
            em_dash_rate_per_1k_words: 0.0,
            ellipsis_rate_per_1k_words: 0.0,
            first_person_pronoun_rate_per_1k_words: 0.0,
            third_person_pronoun_rate_per_1k_words: 0.0,
            top_functional_markers: Vec::new(),
        },
        guidance,
        source_policy: StyleProfileSourcePolicy {
            local_user_provided: true,
            source_text_persisted: false,
            max_excerpt_words: 0,
            allowed_roots: Vec::new(),
            metrics_only: true,
            source_sample_word_budget: None,
            source_paths: Vec::new(),
            recursive: None,
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
        },
        model_receipt: None,
        quality: Default::default(),
        archived_at: None,
        parent_profile_id: None,
        refreshed_from_profile_id: None,
        version_number: None,
        refreshed_at: None,
    };
    svc.repository().insert_style_profile(&card).await.unwrap();
    svc.repository()
        .set_active_style_profile_id(&proj.project_id, Some(profile_id.clone()))
        .await
        .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![mara.character_id.clone()],
            max_character_count: None,
            location_id: gate.location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(4000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    let digest = ctx
        .compact_shelf_digest
        .as_ref()
        .expect("digest present with profile overlay");
    let fishing = digest
        .shelves
        .iter()
        .find(|shelf| shelf.id == "fishing_ending")
        .expect("fishing_ending");
    assert!(!fishing.enabled);
    assert_eq!(
        fishing.profile_overlay,
        Some(spindle_core::style::antislop::ProfileOverlay::Disabled)
    );
    assert!(
        ctx.voice_samples
            .iter()
            .any(|sample| sample.excerpt.contains("Name the tool"))
    );
    assert!(
        ctx.scene_negatives
            .iter()
            .any(|neg| neg.shelf_id.as_deref() == Some("emotion_cocktail"))
    );
}

#[tokio::test]
async fn get_scene_context_expands_budget_for_large_hard_constraints() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        ContextFormat, CreateCharacterInput, CreateLocationInput, CreateWorldRuleInput,
        GetSceneContextInput, WorldStateInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Large Canon".into(),
            project_type: "novel".into(),
            genre: "thriller".into(),
            reader_contract: ReaderContract {
                promise: "Book-scale canon stays available.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let character = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Keeps continuity pressure high.".into(),
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
    let location = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A place with a long canon dossier.".into(),
            initial_state: WorldStateInput {
                controlling_faction: None,
                status: None,
                prosperity: None,
                stability: None,
                threat_level: None,
                sensory_details: Vec::new(),
            },
        })
        .await
        .unwrap();
    let long_rule =
        "Mara must preserve every book-scale continuity detail about the Ash Gate. ".repeat(900);
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: proj.project_id.clone(),
        rule_name: "Ash Gate dossier".into(),
        rule_type: "lore".into(),
        description: long_rule.clone(),
        scan_pattern: None,
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character.character_id.clone()],
            max_character_count: None,
            location_id: location.location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(100),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    assert!(
        ctx.hard_constraints
            .iter()
            .any(|constraint| constraint.id == "Ash Gate dossier"
                && constraint.statement == long_rule),
        "oversized world rules are mandatory hard constraints"
    );
    assert!(
        ctx.budget.token_budget.is_some_and(|budget| budget > 100),
        "scene context should expand beyond the preferred budget when canon requires it"
    );
}

#[tokio::test]
async fn get_scene_context_carries_previous_scene_tail_same_chapter() {
    use spindle_core::models::{
        ContentRating, ContextFormat, GetSceneContextInput, SaveSceneDraftInput,
    };

    let (_tmp, svc, project_id, character_id, location_id) = previous_scene_tail_fixture().await;

    // Build prose long enough that the tail must be truncated to <=1200
    // chars. The final ~1400 chars are all multibyte (em-dash + CJK, 3 bytes
    // each), so the byte offset a naive `&s[s.len()-1200..]` byte slice would
    // pick lands *inside* a codepoint (it would panic / split); a
    // char-boundary-safe walk from the END must not.
    let head = "A".repeat(200);
    // 350 * 4 = 1400 multibyte chars, well past the 1200-char tail window.
    let multibyte_run = "—世界—".repeat(350);
    let tail = "闇が迫った——Mara did not flinch.";
    let scene_one_prose = format!("{head}{multibyte_run}{tail}");

    // Guard: a naive `&s[s.len() - 1200..]` byte slice (treating the 1200
    // char budget as a byte count) would land mid-codepoint here, so this
    // test genuinely exercises the char-boundary-safe path rather than
    // passing by accident on ASCII prose.
    let naive_byte_start = scene_one_prose.len() - 1200;
    assert!(
        !scene_one_prose.is_char_boundary(naive_byte_start),
        "test fixture must place the naive byte-slice boundary mid-codepoint"
    );

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: scene_one_prose.clone(),
        summary: "Mara holds the gate".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let scene_one_id = svc
        .repository
        .find_scene_by_natural_key(
            &project_id,
            &svc.repository
                .get_active_branch(&project_id)
                .await
                .unwrap()
                .id,
            1,
            1,
            1,
        )
        .await
        .unwrap()
        .unwrap()
        .id;

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    let tail_dto = ctx
        .novel
        .previous_scene_tail
        .as_ref()
        .expect("scene 2 should carry the tail of scene 1");
    assert_eq!(tail_dto.scene_id, scene_one_id);
    assert_eq!(tail_dto.chapter_number, 1);
    assert_eq!(tail_dto.scene_order, 1);
    assert!(
        tail_dto.excerpt.chars().count() <= 1200,
        "excerpt must be at most 1200 chars, got {}",
        tail_dto.excerpt.chars().count()
    );
    assert!(
        scene_one_prose.ends_with(&tail_dto.excerpt),
        "excerpt must be the exact final characters of scene 1"
    );
    assert!(
        tail_dto.excerpt.ends_with(tail),
        "excerpt must end with the final characters of scene 1's prose"
    );

    let envelope = svc
        .get_scene_context_envelope(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    assert!(
        envelope
            .context_markdown
            .as_deref()
            .is_some_and(|markdown| markdown.contains("PREVIOUS SCENE (closing)")),
        "markdown should carry the previous-scene closing block"
    );
}

#[tokio::test]
async fn get_scene_context_previous_scene_tail_crosses_chapter_boundary() {
    use spindle_core::models::{
        ContentRating, ContextFormat, GetSceneContextInput, SaveSceneDraftInput,
    };

    let (_tmp, svc, project_id, character_id, location_id) = previous_scene_tail_fixture().await;

    // Chapter 1 has two scenes; the last (highest order) is scene 2.
    for (order, text) in [
        (1, "Chapter one opens."),
        (2, "Chapter one closes at the gate."),
    ] {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: order,
            full_text: text.into(),
            summary: "seed".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    }

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 2,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    let tail_dto = ctx
        .novel
        .previous_scene_tail
        .as_ref()
        .expect("ch2 scene1 should pull the last scene of ch1");
    assert_eq!(tail_dto.chapter_number, 1);
    assert_eq!(tail_dto.scene_order, 2);
    assert_eq!(tail_dto.excerpt, "Chapter one closes at the gate.");
}

#[tokio::test]
async fn get_scene_context_previous_scene_tail_absent_at_book_start() {
    use spindle_core::models::{
        ContentRating, ContextFormat, GetSceneContextInput, SaveSceneDraftInput,
    };

    let (_tmp, svc, project_id, character_id, location_id) = previous_scene_tail_fixture().await;

    // Save chapter 1 scene 1 itself; requesting context for it must yield
    // no previous scene tail (book start).
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "The very first scene.".into(),
        summary: "opening".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    assert!(
        ctx.novel.previous_scene_tail.is_none(),
        "book-start scene must have no previous-scene tail"
    );

    let envelope = svc
        .get_scene_context_envelope(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    assert!(
        envelope
            .context_markdown
            .as_deref()
            .is_some_and(|markdown| !markdown.contains("PREVIOUS SCENE (closing)")),
        "book-start markdown must omit the previous-scene block"
    );
}

#[tokio::test]
async fn get_scene_context_previous_scene_tail_none_when_prose_empty() {
    use spindle_core::models::{
        ContentRating, ContextFormat, GetSceneContextInput, SaveSceneDraftInput,
    };

    let (_tmp, svc, project_id, character_id, location_id) = previous_scene_tail_fixture().await;

    // Previous scene saved with whitespace-only prose.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "   \n\t  ".into(),
        summary: "blank".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    assert!(
        ctx.novel.previous_scene_tail.is_none(),
        "a whitespace-only previous scene must yield no tail (not an empty excerpt)"
    );
}

#[tokio::test]
async fn get_scene_context_previous_scene_tail_omitted_when_section_disabled() {
    use spindle_core::models::{
        ContentRating, ContextFormat, GetSceneContextInput, SaveSceneDraftInput,
    };

    let (_tmp, svc, project_id, character_id, location_id) = previous_scene_tail_fixture().await;

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "A prior scene with real prose.".into(),
        summary: "seed".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // Request a sections list that omits `previous_scene_tail` while still
    // asking for other novel sections.
    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: Some(vec!["reader_contract".into(), "location".into()]),
        })
        .await
        .unwrap();

    assert!(
        ctx.novel.previous_scene_tail.is_none(),
        "previous_scene_tail must be absent when the sections filter omits it"
    );
}

/// A General-rated scene that follows an Explicit one must NOT receive the
/// explicit neighbour's prose: that context is assembled into a draft prompt
/// dispatched to the general-rated (non-explicit-capable) agent. The tail is
/// replaced by the neighbour's stored summary and marked elided.
#[tokio::test]
async fn previous_scene_tail_elides_explicit_prose_for_general_scene() {
    use spindle_core::models::ContentRating;

    let tail = explicit_neighbor_tail_for_planned_scene_two(Some(ContentRating::General))
        .await
        .expect("a general scene should still get a handoff, just not the prose");

    assert!(
        !tail.excerpt.contains("inside her"),
        "explicit neighbour prose leaked into a general-rated scene's context: {}",
        tail.excerpt
    );
    assert!(
        tail.excerpt.contains("finally give in"),
        "elided tail should fall back to the neighbour's summary, got: {}",
        tail.excerpt
    );
    assert!(
        tail.elided_reason.is_some(),
        "an elided tail must say so rather than look like an absent one"
    );
}

/// The mirror case: an Explicit scene following an Explicit one routes to
/// the explicit-capable agent, so the real closing prose must still come
/// through. Elision is a rating boundary, not a blanket filter.
#[tokio::test]
async fn previous_scene_tail_keeps_explicit_prose_for_explicit_scene() {
    use spindle_core::models::ContentRating;

    let tail = explicit_neighbor_tail_for_planned_scene_two(Some(ContentRating::Explicit))
        .await
        .expect("an explicit scene should get the real tail");

    assert!(
        tail.excerpt.contains("inside her"),
        "an explicit scene must still receive its explicit neighbour's prose"
    );
    assert!(
        tail.elided_reason.is_none(),
        "same-clearance handoff must not be marked elided"
    );
}

/// Fail-closed: when the target scene has no planned rating (no plan row,
/// no drafted scene), spindle cannot prove the receiving agent is cleared,
/// so the explicit neighbour is elided rather than gambled.
#[tokio::test]
async fn previous_scene_tail_elides_explicit_prose_when_target_rating_unknown() {
    let tail = explicit_neighbor_tail_for_planned_scene_two(None)
        .await
        .expect("unknown-rating target should still get the summary handoff");

    assert!(
        !tail.excerpt.contains("inside her"),
        "unknown target rating must fail closed, not leak explicit prose"
    );
    assert!(tail.elided_reason.is_some());
}

#[tokio::test]
async fn get_chapter_briefing_assembles_from_real_scene_seed() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        ContentRating, ContextFormat, CreateCharacterInput, CreateLocationInput,
        GetChapterBriefingInput, SaveSceneDraftInput, WorldStateInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Briefing".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Wardens hold the line.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden of the Ash Gate.".into(),
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
    let gate = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "Wall against the dark.".into(),
            initial_state: WorldStateInput {
                controlling_faction: None,
                status: Some("tense".into()),
                prosperity: None,
                stability: None,
                threat_level: None,
                sensory_details: Vec::new(),
            },
        })
        .await
        .unwrap();
    let _ = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara held the line.".into(),
            summary: "Mara holds.".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let briefing = svc
        .get_chapter_briefing(GetChapterBriefingInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            character_ids: vec![mara.character_id.clone()],
            location_id: Some(gate.location_id.clone()),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(6000),
            recent_chapter_limit: None,
            token_budget: None,
        })
        .await
        .unwrap();

    assert_eq!(briefing.scene_seed.scene_order, Some(1));
    assert_eq!(
        briefing.scene_seed.character_ids,
        vec![mara.character_id.clone()]
    );
    assert_eq!(
        briefing.scene_seed.location_id.as_deref(),
        Some(gate.location_id.as_str())
    );
    assert!(briefing.scene_seed.scene_context_available);
    assert!(briefing.briefing_markdown.contains("# Chapter Briefing"));
    assert!(
        briefing.scene_context.is_some(),
        "scene_context should be folded in when scene_order + character_ids + location_id are pinned"
    );
    let digest = briefing
        .compact_shelf_digest
        .as_ref()
        .expect("chapter briefing must carry compact_shelf_digest");
    assert_eq!(digest.shelves.len(), 12);
    assert!(briefing.briefing_markdown.contains("compact_shelf_digest"));
}

#[tokio::test]
async fn briefing_lists_immediately_preceding_chapter_summary() {
    // Defect item 9: briefing chapter 12 right after chapter 11's summary
    // was persisted on the same branch reported "Recent chapter summaries:
    // None recorded before this chapter". The preceding chapter's summary
    // must appear in both the structured output and the markdown.
    use spindle_core::models::{
        ContentRating, ContextFormat, CreateChapterInput, GetChapterBriefingInput,
        SaveSceneDraftInput, SaveSummaryInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Briefing Recency".into(),
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
    for chapter_number in [11, 12] {
        svc.create_chapter(CreateChapterInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(chapter_number),
            title: None,
        })
        .await
        .unwrap();
    }
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 11,
        chapter_id: None,
        scene_order: 1,
        full_text: "Chapter eleven prose.".into(),
        summary: "ch11 scene".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.save_summary(SaveSummaryInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 11,
        entity_type: None,
        entity_id: None,
        summary: "Chapter 11: the drawer stayed shut.".into(),
        key_events: vec!["drawer held".into()],
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    })
    .await
    .unwrap();

    let briefing = svc
        .get_chapter_briefing(GetChapterBriefingInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 12,
            scene_order: None,
            character_ids: Vec::new(),
            location_id: None,
            format: Some(ContextFormat::Markdown),
            budget_tokens: None,
            recent_chapter_limit: None,
            token_budget: None,
        })
        .await
        .unwrap();

    assert!(
        briefing
            .recent_chapter_summaries
            .iter()
            .any(|summary| summary.book_number == 1 && summary.chapter_number == 11),
        "chapter 11's summary must be listed for chapter 12's briefing: {:?}",
        briefing.recent_chapter_summaries
    );
    assert!(
        briefing
            .briefing_markdown
            .contains("Book 1 Chapter 11: Chapter 11: the drawer stayed shut."),
        "markdown must render the preceding summary, got:\n{}",
        briefing.briefing_markdown
    );
    assert!(
        !briefing
            .briefing_markdown
            .contains("None recorded before this chapter"),
        "markdown must not claim no summaries exist:\n{}",
        briefing.briefing_markdown
    );
}

#[tokio::test]
async fn scene_context_surfaces_in_world_time_hard_constraint() {
    use spindle_core::models::{
        CalendarDef, ContentRating, ContextFormat, GetSceneContextInput, SaveSceneDraftInput,
        StoryClock,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "ctx-clock".into(),
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
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "The market opened.".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let repo = svc.repository();
    let branch_id = repo.get_scene(&scene.scene_id).await.unwrap().branch_id;
    let calendar = CalendarDef {
        days_per_week: 7,
        hours_per_day: 24,
        week_day_names: Vec::new(),
        months: Vec::new(),
        days_per_year: 365,
        epoch_label: Some("Founding".into()),
        leap_rule: Default::default(),
    };
    repo.upsert_project_calendar(&proj.project_id, &calendar)
        .await
        .unwrap();
    repo.upsert_scene_clock(
        &scene.scene_id,
        &proj.project_id,
        &branch_id,
        &StoryClock {
            day_index: Some(42),
            ..Default::default()
        },
        None,
        Some("main"),
    )
    .await
    .unwrap();

    let location = svc
        .create_location(spindle_core::models::CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Market".into(),
            kind: "city".into(),
            realm: None,
            summary: "A market square.".into(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: Vec::new(),
            max_character_count: None,
            location_id: location.location_id,
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    assert!(
        ctx.hard_constraints
            .iter()
            .any(|c| c.statement.contains("IN-WORLD TIME") || c.id.contains("IN-WORLD TIME")),
        "scene context should surface the in-world time hard constraint: {:?}",
        ctx.hard_constraints
    );
}

/// Drafting the next scene must surface the PREVIOUS scene's end clock plus
/// the grounding instruction to anchor when this scene takes place and to
/// signal any elapsed time — the pre-draft half of temporal grounding.
#[tokio::test]
async fn scene_context_surfaces_prior_scene_temporal_anchor() {
    use spindle_core::models::{
        CalendarDef, ContentRating, ContextFormat, GetChapterBriefingInput, GetSceneContextInput,
        SaveSceneDraftInput, StoryClock,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "anchor".into(),
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
    let draft = |order: i32| SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: order,
        full_text: "Placeholder prose.".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    };
    let scene1 = svc.save_scene_draft(draft(1)).await.unwrap();
    let _scene2 = svc.save_scene_draft(draft(2)).await.unwrap();

    let repo = svc.repository();
    let branch_id = repo.get_scene(&scene1.scene_id).await.unwrap().branch_id;
    let calendar = CalendarDef {
        days_per_week: 7,
        hours_per_day: 24,
        week_day_names: Vec::new(),
        months: Vec::new(),
        days_per_year: 365,
        epoch_label: None,
        leap_rule: Default::default(),
    };
    repo.upsert_project_calendar(&proj.project_id, &calendar)
        .await
        .unwrap();
    // Scene 1 ends on day 5, evening (18:00). Scene 2 is not yet clocked.
    repo.upsert_scene_clock(
        &scene1.scene_id,
        &proj.project_id,
        &branch_id,
        &StoryClock {
            day_index: Some(5),
            time_of_day: Some(18 * 60),
            ..Default::default()
        },
        None,
        Some("main"),
    )
    .await
    .unwrap();

    let location = svc
        .create_location(spindle_core::models::CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Quay".into(),
            kind: "city".into(),
            realm: None,
            summary: "A harbour quay.".into(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            character_ids: Vec::new(),
            max_character_count: None,
            location_id: location.location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    let anchor = ctx
        .hard_constraints
        .iter()
        .find(|c| c.id.contains("IN-WORLD TIME"))
        .expect("temporal anchor hard constraint must be present");
    let lower = anchor.statement.to_lowercase();
    assert!(
        lower.contains("previous scene") && lower.contains("day 5"),
        "anchor must name the previous scene's end clock: {}",
        anchor.statement
    );
    assert!(
        lower.contains("transition") || lower.contains("scene break"),
        "anchor must instruct the writer to signal elapsed time: {}",
        anchor.statement
    );

    // The same anchor must reach get_chapter_briefing (which the brief /
    // Grok path pulls), inherited from the bundled scene context.
    let briefing = svc
        .get_chapter_briefing(GetChapterBriefingInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(2),
            character_ids: Vec::new(),
            location_id: Some(location.location_id),
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            recent_chapter_limit: None,
            token_budget: None,
        })
        .await
        .unwrap();
    assert!(
        briefing
            .hard_constraints
            .iter()
            .any(|c| c.id.contains("IN-WORLD TIME")
                && c.statement.to_lowercase().contains("previous scene")),
        "chapter briefing must inherit the temporal anchor: {:?}",
        briefing.hard_constraints
    );
}

#[tokio::test]
async fn scene_context_surfaces_economy_price_fact() {
    use spindle_core::models::{ContextFormat, GetSceneContextInput};

    let (_tmp, svc) = fresh_service().await;
    let (project_id, location_id) =
        project_with_economy_price_fact(&svc, "ctx-economy-price").await;

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: Vec::new(),
            max_character_count: None,
            location_id,
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    // The economy-scoped price fact must ride the never-trimmed hard-constraint
    // path, so the drafting model can't contradict the price it set last chapter.
    assert!(
        ctx.hard_constraints
            .iter()
            .any(|c| c.id == "bread_price" && c.statement.contains("silver")),
        "scene context should surface the economy price fact as a hard constraint: {:?}",
        ctx.hard_constraints
    );
}

#[tokio::test]
async fn scene_context_surfaces_economy_lore() {
    use spindle_core::models::{ContextFormat, GetSceneContextInput};

    let (_tmp, svc) = fresh_service().await;
    let (project_id, location_id) = project_with_economy_price_fact(&svc, "ctx-economy-lore").await;

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: Vec::new(),
            max_character_count: None,
            location_id,
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    // The Economy entity (static lore) is surfaced so the drafting model knows
    // what trade system / currency is in play.
    assert!(
        ctx.novel
            .economy_briefing
            .iter()
            .any(|e| e.name == "River Mark" && e.currency.as_deref() == Some("silver marks")),
        "scene context should surface the economy lore in economy_briefing: {:?}",
        ctx.novel.economy_briefing
    );
}

#[tokio::test]
async fn writer_state_surfaces_economy_price_fact() {
    use spindle_core::models::GetWriterStateInput;

    let (_tmp, svc) = fresh_service().await;
    let (project_id, _location_id) =
        project_with_economy_price_fact(&svc, "ws-economy-price").await;

    let ws = svc
        .get_writer_state(GetWriterStateInput {
            project_id: project_id.clone(),
            branch_id: None,
            at_scene_id: None,
            format: None,
            budget_tokens: None,
            include_subjects: None,
            include_recent_activity: None,
            recent_activity_limit: None,
        })
        .await
        .unwrap();

    // The re-anchor packet should surface current named prices so the writer
    // (and the authoring loop) stay consistent with the economy.
    assert!(
        ws.hard_constraints
            .iter()
            .any(|c| c.id == "bread_price" && c.statement.contains("silver")),
        "writer state should surface the economy price fact as a hard constraint: {:?}",
        ws.hard_constraints
    );
}

#[tokio::test]
async fn scene_context_surfaces_quantity_state_hard_constraint() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, CommitQuantityStateInput,
        ContextFormat, CreateCharacterInput, CreateLocationInput, GetSceneContextInput,
        QuantityState,
    };

    let (_tmp, svc) = fresh_service().await;
    let project_id = project_with_wealth_scheme(&svc, "qty-ctx").await;

    let character = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project_id.clone(),
            name: "Hero".into(),
            summary: "the hero".into(),
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
            initial_state: None,
        })
        .await
        .unwrap();

    svc.commit_quantity_state(CommitQuantityStateInput {
        project_id: project_id.clone(),
        subject_table: "character".into(),
        subject_id: character.character_id.clone(),
        measure: "wealth".into(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
        scene_id: None,
        state: QuantityState {
            band: Some("comfortable".into()),
            ..Default::default()
        },
    })
    .await
    .unwrap();

    let location = svc
        .create_location(CreateLocationInput {
            project_id: project_id.clone(),
            name: "Market".into(),
            kind: "city".into(),
            realm: None,
            summary: "A market square.".into(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character.character_id.clone()],
            max_character_count: None,
            location_id: location.location_id,
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    assert!(
        ctx.hard_constraints
            .iter()
            .any(|c| c.id == "[WEALTH/STATE]" && c.statement.contains("comfortable")),
        "scene context should surface the quantity-state hard constraint: {:?}",
        ctx.hard_constraints
    );
}

#[tokio::test]
async fn scene_context_quantity_constraint_shows_book_trajectory() {
    use spindle_core::models::{
        CommitQuantityStateInput, ContextFormat, CreateLocationInput, GetSceneContextInput,
        QuantityState,
    };

    let (_tmp, svc) = fresh_service().await;
    let project_id = project_with_wealth_scheme(&svc, "qty-trajectory").await;
    let hero = seed_character(&svc, &project_id, "Hero").await;

    let commit = |chapter: i32, band: &str| CommitQuantityStateInput {
        project_id: project_id.clone(),
        subject_table: "character".into(),
        subject_id: hero.clone(),
        measure: "wealth".into(),
        book_number: 1,
        chapter_number: chapter,
        scene_order: 1,
        scene_id: None,
        state: QuantityState {
            band: Some(band.into()),
            ..Default::default()
        },
    };
    // Climbs one tier (destitute -> comfortable): a clean arc, no drift.
    svc.commit_quantity_state(commit(1, "destitute"))
        .await
        .unwrap();
    svc.commit_quantity_state(commit(3, "comfortable"))
        .await
        .unwrap();

    let location = svc
        .create_location(CreateLocationInput {
            project_id: project_id.clone(),
            name: "Market".into(),
            kind: "city".into(),
            realm: None,
            summary: "A market.".into(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 5,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![hero.clone()],
            max_character_count: None,
            location_id: location.location_id,
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    let wealth = ctx
        .hard_constraints
        .iter()
        .find(|c| c.id == "[WEALTH/STATE]")
        .expect("wealth/state hard constraint present");
    assert!(
        wealth.statement.contains("destitute → comfortable"),
        "constraint should show the per-book trajectory: {}",
        wealth.statement
    );
}

#[tokio::test]
async fn scene_context_surfaces_story_so_far_digest() {
    use spindle_core::models::{
        ContentRating, ContextFormat, GetSceneContextInput, SaveSceneDraftInput, SaveSummaryInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "sofar".into(),
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
    // A scene establishes chapter 1; a summary builds the book digest.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "x".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.repository()
        .save_summary(&SaveSummaryInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            entity_type: None,
            entity_id: None,
            summary: "The kingdom fell to the Ashlords".into(),
            key_events: Vec::new(),
            character_changes: Vec::new(),
            relationship_shifts: Vec::new(),
            arc_advances: Vec::new(),
            promise_events: Vec::new(),
        })
        .await
        .unwrap();
    let location = svc
        .create_location(spindle_core::models::CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Keep".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A keep.".into(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 2,
            chapter_id: None,
            scene_order: 1,
            character_ids: Vec::new(),
            max_character_count: None,
            location_id: location.location_id,
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    assert!(
        ctx.hard_constraints
            .iter()
            .any(|c| c.statement.contains("STORY SO FAR") && c.statement.contains("Ashlords")),
        "scene context should surface the story-so-far digest: {:?}",
        ctx.hard_constraints
    );
}

#[tokio::test]
async fn scene_context_surfaces_semantic_references() {
    use crate::ai::SearchDocument;
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, ContextFormat,
        CreateCharacterInput, CreateLocationInput, GetSceneContextInput, WorldStateInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "semrefs".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Oathbound wardens hold the line.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Oathbound warden who holds the Ash Gate against the dark.".into(),
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
            initial_state: None,
        })
        .await
        .unwrap();

    let gate = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A blackened wall holding back the dark.".into(),
            initial_state: WorldStateInput::default(),
        })
        .await
        .unwrap();

    let branch = svc
        .repository()
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();

    // Seed three embeddings on the active branch:
    //  - Mara herself (must be EXCLUDED — she's a present-scene entity)
    //  - a topically-overlapping faction (must SURFACE — distant but related)
    //  - an unrelated scribe (allowed, but ranked far)
    let faction_id = "world_rule:oath-wardens";
    let scribe_id = "character:aldric-scribe";
    for (id, doc) in [
        (
            mara.character_id.as_str(),
            SearchDocument {
                entity_table: "character".into(),
                title: "Mara".into(),
                excerpt: "oath warden".into(),
                content: "Mara, oathbound warden, holds the Ash Gate against the dark.".into(),
            },
        ),
        (
            faction_id,
            SearchDocument {
                entity_table: "world_rule".into(),
                title: "Oath Wardens".into(),
                excerpt: "the order that holds the gate".into(),
                content: "The Oath Wardens are oathbound to hold the Ash Gate against the dark."
                    .into(),
            },
        ),
        (
            scribe_id,
            SearchDocument {
                entity_table: "character".into(),
                title: "Aldric".into(),
                excerpt: "scribe".into(),
                content: "Aldric copies old maps by candlelight; ink stains his cuffs.".into(),
            },
        ),
    ] {
        svc.repository()
            .upsert_search_embedding_document(&proj.project_id, &branch.id, id, &doc)
            .await
            .unwrap();
    }

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![mara.character_id.clone()],
            max_character_count: None,
            location_id: gate.location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: Some(vec!["semantic_references".into()]),
        })
        .await
        .unwrap();

    let refs = &ctx.novel.semantic_references;
    assert!(
        refs.iter().any(|r| r.entity_id == faction_id),
        "topically-related distant canon should surface: {refs:?}"
    );
    assert!(
        !refs.iter().any(|r| r.entity_id == mara.character_id),
        "a present-scene character must be excluded from recall: {refs:?}"
    );
    assert!(
        !refs.iter().any(|r| r.entity_id == gate.location_id),
        "the present location must be excluded from recall: {refs:?}"
    );
    // The overlapping faction must outrank the unrelated scribe.
    let faction_score = refs
        .iter()
        .find(|r| r.entity_id == faction_id)
        .map(|r| r.score)
        .expect("faction must be present");
    if let Some(scribe) = refs.iter().find(|r| r.entity_id == scribe_id) {
        assert!(
            faction_score > scribe.score,
            "related canon must outrank unrelated canon: {refs:?}"
        );
    }
}

#[tokio::test]
async fn scene_context_renders_open_threads_line() {
    use spindle_core::models::{
        ContentRating, ContextFormat, CreateNarrativePromiseInput, GetSceneContextInput,
        SaveSceneDraftInput, SaveSummaryInput, StoryPlacement,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "threadctx".into(),
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

    svc.create_narrative_promise(CreateNarrativePromiseInput {
        project_id: proj.project_id.clone(),
        promise_type: "setup".into(),
        description: "the oath sworn at the gate must be kept".into(),
        planted_at: StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(0),
            note: None,
        },
        planned_payoff: None,
        notes: Vec::new(),
    })
    .await
    .unwrap();

    // Book 1 scene + summary → builds the book-1 digest with open threads.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "x".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.repository()
        .save_summary(&SaveSummaryInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            entity_type: None,
            entity_id: None,
            summary: "The oath is sworn".into(),
            key_events: Vec::new(),
            character_changes: Vec::new(),
            relationship_shifts: Vec::new(),
            arc_advances: Vec::new(),
            promise_events: Vec::new(),
        })
        .await
        .unwrap();
    let location = svc
        .create_location(spindle_core::models::CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Keep".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A keep.".into(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();

    // Ask for context in book 2 → prior book-1 digest (with threads) surfaces.
    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 2,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: Vec::new(),
            max_character_count: None,
            location_id: location.location_id,
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    assert!(
        ctx.hard_constraints.iter().any(|c| {
            c.statement.contains("STORY SO FAR")
                && c.statement.contains("Open threads:")
                && c.statement
                    .contains("the oath sworn at the gate must be kept")
        }),
        "story-so-far should carry the open-threads line naming the promise: {:?}",
        ctx.hard_constraints
    );
}

#[tokio::test]
async fn scene_context_omits_open_threads_when_all_resolved() {
    use spindle_core::models::{
        ContentRating, ContextFormat, CreateNarrativePromiseInput, GetSceneContextInput,
        SaveSceneDraftInput, SaveSummaryInput, StoryPlacement, UpdatePromiseStatusInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "noThreads".into(),
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

    let promise = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: proj.project_id.clone(),
            promise_type: "setup".into(),
            description: "a promise that gets paid off".into(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(0),
                note: None,
            },
            planned_payoff: None,
            notes: Vec::new(),
        })
        .await
        .unwrap();
    svc.update_promise_status(UpdatePromiseStatusInput {
        narrative_promise_id: promise.narrative_promise_id,
        status: "paid_off".into(),
        note: None,
        ..Default::default()
    })
    .await
    .unwrap();

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "x".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    let repo = svc.repository();
    let branch = repo.get_active_branch(&proj.project_id).await.unwrap();
    repo.save_summary(&SaveSummaryInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        entity_type: None,
        entity_id: None,
        summary: "Everything resolves".into(),
        key_events: Vec::new(),
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    })
    .await
    .unwrap();

    let digests = repo
        .list_book_digests_by_project_and_branch(&proj.project_id, &branch.id)
        .await
        .unwrap();
    assert_eq!(digests.len(), 1);
    assert!(
        digests[0].open_threads.is_empty(),
        "no open threads when all resolved: {:?}",
        digests[0].open_threads
    );

    let location = svc
        .create_location(spindle_core::models::CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Keep".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A keep.".into(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();
    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 2,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: Vec::new(),
            max_character_count: None,
            location_id: location.location_id,
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    assert!(
        !ctx.hard_constraints
            .iter()
            .any(|c| c.statement.contains("Open threads:")),
        "no open-threads line when nothing is open: {:?}",
        ctx.hard_constraints
    );
}

#[tokio::test]
async fn scene_context_open_threads_respects_per_book_cap_with_multibyte() {
    use spindle_core::models::{
        ContentRating, ContextFormat, CreateNarrativePromiseInput, GetSceneContextInput,
        SaveSceneDraftInput, SaveSummaryInput, StoryPlacement,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "capThreads".into(),
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

    // 15 promises with long multibyte (em-dash / CJK) descriptions.
    for n in 0..15 {
        svc.create_narrative_promise(CreateNarrativePromiseInput {
            project_id: proj.project_id.clone(),
            promise_type: "setup".into(),
            description: format!("約束{n:02}—a long thread {}", "字".repeat(60)),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(0),
                note: None,
            },
            planned_payoff: None,
            notes: Vec::new(),
        })
        .await
        .unwrap();
    }

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "x".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.repository()
        .save_summary(&SaveSummaryInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            entity_type: None,
            entity_id: None,
            summary: "多くの約束".into(),
            key_events: Vec::new(),
            character_changes: Vec::new(),
            relationship_shifts: Vec::new(),
            arc_advances: Vec::new(),
            promise_events: Vec::new(),
        })
        .await
        .unwrap();
    let location = svc
        .create_location(spindle_core::models::CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Keep".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A keep.".into(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 2,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: Vec::new(),
            max_character_count: None,
            location_id: location.location_id,
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    let sofar = ctx
        .hard_constraints
        .iter()
        .find(|c| c.statement.contains("STORY SO FAR"))
        .expect("story-so-far present");
    // The rendered per-book section must stay within the 700-char cap. The
    // statement wraps one book section, so its non-header body is bounded.
    // Assert the whole statement is valid UTF-8 (guaranteed) and that the
    // book-1 section is inside the per-book budget.
    let body = sofar.statement.as_str();
    assert!(body.is_char_boundary(body.len()), "valid UTF-8 boundary");
    // Book section header + synopsis + threads must not exceed per-book cap.
    // Locate the "Book 1" section and measure to end (single book here).
    let section = body
        .find("Book 1")
        .map(|i| &body[i..])
        .expect("book 1 section present");
    assert!(
        section.len() <= 700 + "Book 1 (through ch 1): ".len(),
        "per-book section within cap: {} bytes",
        section.len()
    );
}

/// T-109 test 1: three annotated prior chapters (0.6/0.45/0.3) feed forward a
/// falling-intensity trend directive into a chapter-4 scene context.
#[tokio::test]
async fn scene_context_feeds_forward_falling_intensity_trend() {
    let (_tmp, svc, project_id, character_id, location_id) = pacing_feed_forward_fixture().await;
    for (chapter, intensity) in [(1, 0.6_f64), (2, 0.45), (3, 0.3)] {
        seed_annotated_chapter(&svc, &project_id, chapter, Some(intensity)).await;
    }

    let ctx = scene_context_for_chapter(&svc, &project_id, &character_id, &location_id, 4).await;

    let trend = ctx
        .novel
        .realized_intensity_trend
        .as_ref()
        .expect("chapter-4 scene context should carry a realized-intensity trend directive");
    assert!(
        trend.contains("0.60 → 0.45 → 0.30"),
        "trend directive must state the last-3-chapter sequence: {trend}"
    );
    assert!(
        trend.contains("falling"),
        "trend directive must name the falling direction: {trend}"
    );
}

/// T-109 test 2: no prior annotations → no trend directive at all.
#[tokio::test]
async fn scene_context_omits_trend_when_no_prior_annotations() {
    let (_tmp, svc, project_id, character_id, location_id) = pacing_feed_forward_fixture().await;
    // Chapters exist but carry no intensity annotations at all.
    for chapter in 1..=3 {
        seed_annotated_chapter(&svc, &project_id, chapter, None).await;
    }

    let ctx = scene_context_for_chapter(&svc, &project_id, &character_id, &location_id, 4).await;

    assert!(
        ctx.novel.realized_intensity_trend.is_none(),
        "no annotated prior chapters must yield no trend directive: {:?}",
        ctx.novel.realized_intensity_trend
    );
}

/// T-109 test 3: exactly one annotated prior chapter → single-value directive
/// with NO rising/falling/flat wording.
#[tokio::test]
async fn scene_context_single_prior_chapter_states_value_without_direction() {
    let (_tmp, svc, project_id, character_id, location_id) = pacing_feed_forward_fixture().await;
    seed_annotated_chapter(&svc, &project_id, 1, Some(0.42)).await;

    let ctx = scene_context_for_chapter(&svc, &project_id, &character_id, &location_id, 2).await;

    let trend = ctx
        .novel
        .realized_intensity_trend
        .as_ref()
        .expect("a single annotated prior chapter should still surface its mean");
    assert!(
        trend.contains("0.42"),
        "single-chapter directive must state the mean: {trend}"
    );
    assert!(
        !trend.contains("rising") && !trend.contains("falling") && !trend.contains("flat"),
        "single-chapter directive must make no trend claim: {trend}"
    );
}

/// T-109 test 4: a chapter whose only annotation has NULL intensity is treated
/// as unannotated and excluded from the trend window. Representable because
/// `scene_beat_annotation.intensity` is nullable (V0019).
#[tokio::test]
async fn scene_context_excludes_all_null_intensity_chapter_from_window() {
    let (_tmp, svc, project_id, character_id, location_id) = pacing_feed_forward_fixture().await;
    // Chapter 1 annotated (0.6); chapter 2 annotated but intensity NULL;
    // chapter 3 annotated (0.3). The window for chapter 4 must skip chapter 2.
    seed_annotated_chapter(&svc, &project_id, 1, Some(0.6)).await;
    seed_annotated_chapter(&svc, &project_id, 2, None).await;
    seed_annotated_chapter(&svc, &project_id, 3, Some(0.3)).await;

    let ctx = scene_context_for_chapter(&svc, &project_id, &character_id, &location_id, 4).await;

    let trend = ctx
        .novel
        .realized_intensity_trend
        .as_ref()
        .expect("chapter-4 scene context should carry a trend directive");
    // Only the two annotated chapters (0.60 and 0.30) form the sequence; the
    // NULL chapter contributes no value.
    assert!(
        trend.contains("0.60 → 0.30"),
        "all-NULL chapter must be excluded from the window: {trend}"
    );
    assert!(
        !trend.contains("0.45") && !trend.contains("0.00"),
        "the NULL chapter must not appear as a value: {trend}"
    );
}

#[tokio::test]
async fn scene_context_hydrates_plan_targeted_active_threads() {
    use spindle_core::models::{
        ContextFormat, CreateConflictInput, CreateMotifInput, CreatePlotLineInput,
        CreateThemeInput, GetSceneContextInput, PlanChapterInput, PlanChapterSceneInput,
        StatedConsequence,
    };

    let (_tmp, svc) = fresh_service().await;
    let (project_id, character_id, location_id) =
        active_threads_base(&svc, "Active Threads Scene").await;

    let theme = svc
        .create_theme(CreateThemeInput {
            project_id: project_id.clone(),
            theme_statement: "Duty devours the self.".into(),
            thesis_antithesis: "duty vs freedom".into(),
            introduction_point: None,
            resolution_point: None,
        })
        .await
        .unwrap();
    let conflict = svc
        .create_conflict(CreateConflictInput {
            project_id: project_id.clone(),
            name: "Mara vs Wraith".into(),
            conflict_type: "person-vs-supernatural".into(),
            stakes: "The village burns if she fails.".into(),
            escalation_stages: Vec::new(),
            expected_total_cycles: None,
            try_fail_cycles: Vec::new(),
            stated_consequences: vec![StatedConsequence {
                description: "The ward will shatter by dawn.".into(),
                stated_at: None,
                must_demonstrate_by: None,
                delivered: false,
            }],
        })
        .await
        .unwrap();
    let plot_line = svc
        .create_plot_line(CreatePlotLineInput {
            project_id: project_id.clone(),
            name: "The Sundering".into(),
            plot_type: "main".into(),
            summary: "The dark returns to reclaim the world.".into(),
            status: Some("rising".into()),
            convergence_points: Vec::new(),
            connected_conflict_ids: Vec::new(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();
    let motif = svc
        .create_motif(CreateMotifInput {
            project_id: project_id.clone(),
            name: "Cold iron".into(),
            description: "Iron that never warms.".into(),
            max_uses_per_chapter: Some(2),
            connected_theme_ids: vec![theme.theme_id.clone()],
        })
        .await
        .unwrap();

    svc.plan_chapter(PlanChapterInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: Some(character_id.clone()),
        synopsis: "Mara holds the gate.".into(),
        target_theme_ids: vec![theme.theme_id.clone()],
        target_conflict_ids: vec![conflict.conflict_id.clone()],
        target_plot_line_ids: vec![plot_line.plot_line_id.clone()],
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "Mara takes the watch".into(),
            beat_structure: vec!["arrival".into()],
            character_ids: vec![character_id.clone()],
            purpose: "establishing".into(),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    let threads = &ctx.novel.active_threads;
    assert_eq!(threads.len(), 4, "all four targeted threads hydrate");
    let by_kind = |kind: &str| threads.iter().find(|t| t.kind == kind).unwrap();
    assert_eq!(by_kind("theme").id, theme.theme_id);
    assert_eq!(by_kind("theme").name, "Duty devours the self.");
    assert_eq!(by_kind("theme").statement, "Duty devours the self.");
    let conflict_thread = by_kind("conflict");
    assert_eq!(conflict_thread.name, "Mara vs Wraith");
    assert_eq!(conflict_thread.statement, "The village burns if she fails.");
    assert_eq!(conflict_thread.status, "person-vs-supernatural");
    assert_eq!(
        conflict_thread.next_expectation.as_deref(),
        Some("The ward will shatter by dawn."),
        "undelivered stated consequence surfaces as next expectation"
    );
    let plot_thread = by_kind("plot_line");
    assert_eq!(plot_thread.name, "The Sundering");
    assert_eq!(
        plot_thread.statement,
        "The dark returns to reclaim the world."
    );
    assert_eq!(plot_thread.status, "rising");
    let motif_thread = by_kind("motif");
    assert_eq!(motif_thread.id, motif.motif_id);
    assert_eq!(motif_thread.name, "Cold iron");
    assert_eq!(motif_thread.statement, "Iron that never warms.");
    assert_eq!(
        motif_thread.next_expectation.as_deref(),
        Some("use sparingly: ≤2 per chapter")
    );

    // Deterministic ordering: theme -> conflict -> plot_line -> motif.
    let kinds: Vec<&str> = threads.iter().map(|t| t.kind.as_str()).collect();
    assert_eq!(kinds, vec!["theme", "conflict", "plot_line", "motif"]);

    let envelope = svc
        .get_scene_context_envelope(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    assert!(
        envelope
            .context_markdown
            .as_deref()
            .is_some_and(|md| md.contains("ACTIVE THREADS")),
        "rendered scene-context markdown contains an ACTIVE THREADS block"
    );
}

#[tokio::test]
async fn chapter_briefing_hydrates_plan_targeted_active_threads() {
    use spindle_core::models::{
        ContextFormat, CreateThemeInput, GetChapterBriefingInput, PlanChapterInput,
        PlanChapterSceneInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let (project_id, character_id, location_id) =
        active_threads_base(&svc, "Active Threads Briefing").await;

    let theme = svc
        .create_theme(CreateThemeInput {
            project_id: project_id.clone(),
            theme_statement: "The line must hold.".into(),
            thesis_antithesis: "order vs chaos".into(),
            introduction_point: None,
            resolution_point: None,
        })
        .await
        .unwrap();

    svc.plan_chapter(PlanChapterInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: Some(character_id.clone()),
        synopsis: "Mara holds the gate.".into(),
        target_theme_ids: vec![theme.theme_id.clone()],
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "Mara takes the watch".into(),
            beat_structure: vec!["arrival".into()],
            character_ids: vec![character_id.clone()],
            purpose: "establishing".into(),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    let briefing = svc
        .get_chapter_briefing(GetChapterBriefingInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            character_ids: vec![character_id.clone()],
            location_id: Some(location_id.clone()),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(8000),
            recent_chapter_limit: None,
            token_budget: None,
        })
        .await
        .unwrap();

    assert_eq!(briefing.active_threads.len(), 1);
    assert_eq!(briefing.active_threads[0].kind, "theme");
    assert_eq!(briefing.active_threads[0].id, theme.theme_id);
    assert!(
        briefing.briefing_markdown.contains("ACTIVE THREADS"),
        "briefing markdown contains an ACTIVE THREADS block"
    );
}

#[tokio::test]
async fn scene_context_skips_stale_active_thread_ids() {
    use spindle_core::models::{
        ContextFormat, CreatePlotLineInput, GetSceneContextInput, PlanChapterInput,
        PlanChapterSceneInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let (project_id, character_id, location_id) = active_threads_base(&svc, "Stale Thread").await;

    let plot_line = svc
        .create_plot_line(CreatePlotLineInput {
            project_id: project_id.clone(),
            name: "Live plot".into(),
            plot_type: "main".into(),
            summary: "This one exists.".into(),
            status: Some("rising".into()),
            convergence_points: Vec::new(),
            connected_conflict_ids: Vec::new(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();

    svc.plan_chapter(PlanChapterInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: Some(character_id.clone()),
        synopsis: "Mara holds the gate.".into(),
        // A theme id that resolves to nothing plus a real plot line.
        target_theme_ids: vec!["theme:does-not-exist".into()],
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: vec![plot_line.plot_line_id.clone()],
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "Mara takes the watch".into(),
            beat_structure: vec!["arrival".into()],
            character_ids: vec![character_id.clone()],
            purpose: "establishing".into(),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    assert_eq!(
        ctx.novel.active_threads.len(),
        1,
        "stale theme id is skipped, real plot line survives"
    );
    assert_eq!(ctx.novel.active_threads[0].kind, "plot_line");
    assert_eq!(ctx.novel.active_threads[0].id, plot_line.plot_line_id);
}

#[tokio::test]
async fn scene_context_omits_active_threads_when_no_targets() {
    use spindle_core::models::{
        ContextFormat, GetSceneContextInput, PlanChapterInput, PlanChapterSceneInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let (project_id, character_id, location_id) = active_threads_base(&svc, "No Targets").await;

    svc.plan_chapter(PlanChapterInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: Some(character_id.clone()),
        synopsis: "Mara holds the gate.".into(),
        target_theme_ids: Vec::new(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "Mara takes the watch".into(),
            beat_structure: vec!["arrival".into()],
            character_ids: vec![character_id.clone()],
            purpose: "establishing".into(),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    assert!(ctx.novel.active_threads.is_empty());

    let envelope = svc
        .get_scene_context_envelope(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    assert!(
        envelope
            .context_markdown
            .as_deref()
            .is_some_and(|md| !md.contains("ACTIVE THREADS")),
        "no ACTIVE THREADS header when the vec is empty"
    );
}

#[tokio::test]
async fn scene_context_active_threads_budget_and_ordering() {
    use spindle_core::models::{
        ContextFormat, CreateThemeInput, GetSceneContextInput, PlanChapterInput,
        PlanChapterSceneInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let (project_id, character_id, location_id) = active_threads_base(&svc, "Budget Threads").await;

    // One theme with a very long statement including multibyte chars near
    // the 240-char boundary, to prove char-boundary-safe truncation.
    let long_statement = format!("{}é{}", "a".repeat(239), "b".repeat(4760));
    let long_theme = svc
        .create_theme(CreateThemeInput {
            project_id: project_id.clone(),
            theme_statement: long_statement.clone(),
            thesis_antithesis: "x vs y".into(),
            introduction_point: None,
            resolution_point: None,
        })
        .await
        .unwrap();

    // Seven targeted themes total (the long one + six more) to prove the
    // cap of five and id-ascending order within kind.
    let mut theme_ids = vec![long_theme.theme_id.clone()];
    for i in 0..6 {
        let t = svc
            .create_theme(CreateThemeInput {
                project_id: project_id.clone(),
                theme_statement: format!("Filler theme {i}."),
                thesis_antithesis: "a vs b".into(),
                introduction_point: None,
                resolution_point: None,
            })
            .await
            .unwrap();
        theme_ids.push(t.theme_id);
    }

    svc.plan_chapter(PlanChapterInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: Some(character_id.clone()),
        synopsis: "Mara holds the gate.".into(),
        target_theme_ids: theme_ids.clone(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "Mara takes the watch".into(),
            beat_structure: vec!["arrival".into()],
            character_ids: vec![character_id.clone()],
            purpose: "establishing".into(),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![character_id.clone()],
            max_character_count: None,
            location_id: location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(16000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    let threads = &ctx.novel.active_threads;
    assert_eq!(threads.len(), 5, "cap of five per kind is enforced");

    // Ordered by id ascending within the theme kind.
    let mut expected_ids = theme_ids.clone();
    expected_ids.sort();
    expected_ids.truncate(5);
    let got_ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
    assert_eq!(got_ids, expected_ids, "threads sorted by id ascending");

    // Whichever surviving thread is the long theme must be char-safe and
    // truncated to at most 240 chars.
    if let Some(long) = threads.iter().find(|t| t.id == long_theme.theme_id) {
        assert!(
            long.statement.chars().count() <= 240,
            "statement truncated to 240 chars, got {}",
            long.statement.chars().count()
        );
        // Char-boundary safety: indexing the whole string must succeed.
        assert!(long.statement.is_char_boundary(long.statement.len()));
    }
}

// =========================================================================
// Secret-knowledge gating — Part A (design docs/secret-knowledge-gating-design.md).
// =========================================================================
mod secret_knowledge_gating {
    use super::*;
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, ContentRating, ContextFormat,
        CreateCharacterInput, CreateLocationInput, GetSceneContextInput, RecordKnowledgeInput,
        RegisterCanonicalFactInput, SaveSceneDraftInput, SecrecyScope, WorldStateInput,
    };

    /// Build a project + one character + a location + a saved scene,
    /// returning (project_id, character_id, scene_id, location_id) used
    /// across the secret-gating tests.
    async fn scaffold(svc: &SqliteSpindleService) -> (String, String, String, String) {
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Secrets".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Nothing stays hidden forever.".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let mara = svc
            .create_character(CreateCharacterInput {
                aliases: Vec::new(),
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
                summary: "A reincarnated warden who has told no one.".into(),
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
                initial_state: None,
            })
            .await
            .unwrap();
        let gate = svc
            .create_location(CreateLocationInput {
                project_id: proj.project_id.clone(),
                name: "Ash Gate".into(),
                kind: "fortress".into(),
                realm: None,
                summary: "A blackened wall holding back the dark.".into(),
                initial_state: WorldStateInput {
                    controlling_faction: None,
                    status: Some("tense".into()),
                    prosperity: None,
                    stability: Some("fragile".into()),
                    threat_level: Some("high".into()),
                    sensory_details: vec!["smell of ash".into()],
                },
            })
            .await
            .unwrap();
        let scene = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara kept her own counsel.".into(),
                summary: "Mara alone.".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
                ..Default::default()
            })
            .await
            .unwrap();
        (
            proj.project_id,
            mara.character_id,
            scene.scene_id,
            gate.location_id,
        )
    }

    /// A1 back-compat/plumbing: a fact registered WITHOUT secrecy reads back
    /// `secret = false` / `concealment_note = None`, and a knowledge row
    /// recorded without a link reads `secret_of_fact_id = None`. This pins
    /// the additive/defaulted schema: pre-V0023 shape is the default shape.
    /// A1 migration back-compat: a row inserted the *pre-V0023 way* (the
    /// INSERT omits `secret` / `concealment_note` / `secret_of_fact_id`
    /// entirely, exactly as a pre-migration binary would) reads back through
    /// the record `TryFrom` with `secret = false`, `concealment_note = None`,
    /// and `secret_of_fact_id = None` — proving the additive migration's
    /// DEFAULT 0 / NULL semantics hold for old rows.
    #[tokio::test]
    async fn pre_v0023_rows_default_to_non_secret() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, character_id, scene_id, _location_id) = scaffold(&svc).await;
        let branch = svc
            .repository()
            .get_active_branch(&project_id)
            .await
            .unwrap();
        let branch_id = branch.id.clone();

        let fact_id = "canonical_fact:pre_v0023".to_string();
        let knowledge_id = "knowledge_fact:pre_v0023".to_string();
        {
            let fact_id = fact_id.clone();
            let knowledge_id = knowledge_id.clone();
            let project_id = project_id.clone();
            let branch_id = branch_id.clone();
            let scene_id = scene_id.clone();
            let character_id = character_id.clone();
            svc.repository()
                .pool()
                .write(move |conn| {
                    // Pre-V0023 canonical_fact INSERT: no secret columns.
                    conn.execute(
                        "INSERT INTO canonical_fact (id, project_id, branch_id, scene_id, \
                             book_number, chapter_number, subject_table, predicate, value_kind, \
                             value_text, aliases, scope, created_at, updated_at) \
                             VALUES (?1, ?2, ?3, ?4, 1, 1, 'project', 'motto', 'string', \
                                     'hold the line', '[]', 'invariant', 0, 0)",
                        rusqlite::params![&fact_id, &project_id, &branch_id, &scene_id],
                    )?;
                    // Pre-V0023 knowledge_fact INSERT: no secret_of_fact_id.
                    conn.execute(
                        "INSERT INTO knowledge_fact (id, project_id, branch_id, character_id, \
                             fact, normalized_fact, source_summary, tags, reader_visible, \
                             created_at, updated_at) \
                             VALUES (?1, ?2, ?3, ?4, 'old fact', 'old fact', 'legacy', '[]', 1, \
                                     0, 0)",
                        rusqlite::params![&knowledge_id, &project_id, &branch_id, &character_id],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
        }

        let fact = svc.repository().get_canonical_fact(&fact_id).await.unwrap();
        assert!(!fact.secret, "pre-V0023 fact defaults to secret=false");
        assert_eq!(fact.concealment_note, None);

        let knowledge = svc
            .repository()
            .get_knowledge_fact(&knowledge_id)
            .await
            .unwrap();
        assert_eq!(
            knowledge.secret_of_fact_id, None,
            "pre-V0023 knowledge row has no secret link"
        );
    }

    #[tokio::test]
    async fn plain_fact_and_knowledge_default_to_non_secret() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, character_id, scene_id, _location_id) = scaffold(&svc).await;

        let out = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project_id.clone(),
                scene_id: Some(scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".into()),
                subject_id: Some(character_id.clone()),
                predicate: Some("eye_color".into()),
                value_kind: Some("string".into()),
                value_text: Some("grey".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
                secrecy: None,
            })
            .await
            .unwrap();
        let fact = svc
            .repository()
            .get_canonical_fact(&out.canonical_fact_id)
            .await
            .unwrap();
        assert!(!fact.secret, "a fact without secrecy is public");
        assert_eq!(fact.concealment_note, None);

        svc.record_knowledge(RecordKnowledgeInput {
            project_id: project_id.clone(),
            branch_id: None,
            character_id: character_id.clone(),
            fact: "The sky is grey.".into(),
            source_summary: "observation".into(),
            learned_at: None,
            confidence: None,
            tags: Vec::new(),
            reader_visible: true,
            secret_of_fact_id: None,
        })
        .await
        .unwrap();
        let branch = svc
            .repository()
            .get_active_branch(&project_id)
            .await
            .unwrap();
        let rows = svc
            .repository()
            .list_knowledge_facts_by_project_and_branch(&project_id, &branch.id)
            .await
            .unwrap();
        assert!(
            rows.iter().all(|r| r.secret_of_fact_id.is_none()),
            "ordinary knowledge rows carry no secret link"
        );
    }

    /// A2 declaration: registering with `secrecy` marks the fact secret,
    /// stores the concealment note, and writes one linked holder row per
    /// holder_id with `learned_at = None` (always known) and
    /// `reader_visible = true` (dramatic-irony default).
    #[tokio::test]
    async fn declaring_secrecy_writes_linked_holder_rows() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, character_id, scene_id, _location_id) = scaffold(&svc).await;

        let out = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project_id.clone(),
                scene_id: Some(scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".into()),
                subject_id: Some(character_id.clone()),
                predicate: Some("reincarnation".into()),
                value_kind: Some("string".into()),
                value_text: Some("is a reincarnated warden".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
                secrecy: Some(SecrecyScope {
                    holder_ids: vec![character_id.clone()],
                    concealment_note: Some("she deflects with dry humor".into()),
                }),
            })
            .await
            .unwrap();

        let fact = svc
            .repository()
            .get_canonical_fact(&out.canonical_fact_id)
            .await
            .unwrap();
        assert!(fact.secret, "declared fact is secret");
        assert_eq!(
            fact.concealment_note.as_deref(),
            Some("she deflects with dry humor")
        );

        let branch = svc
            .repository()
            .get_active_branch(&project_id)
            .await
            .unwrap();
        let circle = svc
            .repository()
            .secret_circle_members(&project_id, &branch.id, &out.canonical_fact_id)
            .await
            .unwrap();
        assert_eq!(circle.len(), 1, "one holder row written");
        assert_eq!(circle[0].0, character_id);
        assert!(
            circle[0].1.is_none(),
            "declaration-time holder is known from the start (learned_at None)"
        );

        let rows = svc
            .repository()
            .list_knowledge_facts_by_project_and_branch(&project_id, &branch.id)
            .await
            .unwrap();
        let linked = rows
            .iter()
            .find(|r| r.secret_of_fact_id.as_deref() == Some(out.canonical_fact_id.as_str()))
            .expect("linked holder row exists");
        assert!(
            linked.reader_visible,
            "dramatic-irony default is reader-visible"
        );
        assert_eq!(linked.source_summary, "secret declared");
    }

    /// A2 reveal path: `record_knowledge` with a valid `secret_of_fact_id`
    /// expands the circle (the new holder appears with a placement-stamped
    /// learned_at).
    #[tokio::test]
    async fn reveal_expands_circle_with_valid_link() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, character_id, scene_id, _location_id) = scaffold(&svc).await;
        let confidant = svc
            .create_character(CreateCharacterInput {
                aliases: Vec::new(),
                project_id: project_id.clone(),
                name: "Bran".into(),
                summary: "The one she finally tells.".into(),
                role: "supporting".into(),
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
                initial_state: None,
            })
            .await
            .unwrap();

        let out = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project_id.clone(),
                scene_id: Some(scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".into()),
                subject_id: Some(character_id.clone()),
                predicate: Some("reincarnation".into()),
                value_kind: Some("string".into()),
                value_text: Some("is a reincarnated warden".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
                secrecy: Some(SecrecyScope {
                    holder_ids: vec![character_id.clone()],
                    concealment_note: None,
                }),
            })
            .await
            .unwrap();

        svc.record_knowledge(RecordKnowledgeInput {
            project_id: project_id.clone(),
            branch_id: None,
            character_id: confidant.character_id.clone(),
            fact: "Mara is a reincarnated warden.".into(),
            source_summary: "she finally told him".into(),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 12,
                scene_order: Some(1),
                note: None,
            }),
            confidence: None,
            tags: Vec::new(),
            reader_visible: true,
            secret_of_fact_id: Some(out.canonical_fact_id.clone()),
        })
        .await
        .unwrap();

        let branch = svc
            .repository()
            .get_active_branch(&project_id)
            .await
            .unwrap();
        let mut circle = svc
            .repository()
            .secret_circle_members(&project_id, &branch.id, &out.canonical_fact_id)
            .await
            .unwrap();
        circle.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(circle.len(), 2, "circle expanded to two members");
        let confidant_entry = circle
            .iter()
            .find(|(id, _)| id == &confidant.character_id)
            .expect("confidant in circle");
        assert_eq!(
            confidant_entry.1.as_ref().map(|p| p.chapter_number),
            Some(12),
            "the reveal is placement-stamped at chapter 12"
        );
    }

    /// A2 reveal validation: a `secret_of_fact_id` pointing at a NON-secret
    /// fact is rejected with a domain error (the reveal would grant
    /// membership in a non-secret).
    #[tokio::test]
    async fn reveal_link_to_non_secret_fact_errors() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, character_id, scene_id, _location_id) = scaffold(&svc).await;

        let public = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project_id.clone(),
                scene_id: Some(scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".into()),
                subject_id: Some(character_id.clone()),
                predicate: Some("eye_color".into()),
                value_kind: Some("string".into()),
                value_text: Some("grey".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
                secrecy: None,
            })
            .await
            .unwrap();

        let err = svc
            .record_knowledge(RecordKnowledgeInput {
                project_id: project_id.clone(),
                branch_id: None,
                character_id: character_id.clone(),
                fact: "grey eyes".into(),
                source_summary: "bogus reveal".into(),
                learned_at: None,
                confidence: None,
                tags: Vec::new(),
                reader_visible: true,
                secret_of_fact_id: Some(public.canonical_fact_id.clone()),
            })
            .await
            .expect_err("linking a reveal to a non-secret fact must error");
        assert!(
            err.to_string().contains("not marked secret"),
            "error names the non-secret cause: {err}"
        );

        let missing = svc
            .record_knowledge(RecordKnowledgeInput {
                project_id: project_id.clone(),
                branch_id: None,
                character_id: character_id.clone(),
                fact: "phantom".into(),
                source_summary: "bogus reveal".into(),
                learned_at: None,
                confidence: None,
                tags: Vec::new(),
                reader_visible: true,
                secret_of_fact_id: Some("canonical_fact:does-not-exist".into()),
            })
            .await
            .expect_err("linking a reveal to a missing fact must error");
        assert!(
            missing
                .to_string()
                .contains("does not reference an existing"),
            "error names the missing cause: {missing}"
        );
    }

    /// A2 retrofit: `canonical_fact.secret` and `.concealment_note` are in
    /// the update_entity allowlist, so an existing public fact can be
    /// retrofitted into a secret without a data migration.
    #[tokio::test]
    async fn update_entity_retrofits_secret_flag() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, character_id, scene_id, _location_id) = scaffold(&svc).await;
        let out = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project_id.clone(),
                scene_id: Some(scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".into()),
                subject_id: Some(character_id.clone()),
                predicate: Some("origin".into()),
                value_kind: Some("string".into()),
                value_text: Some("came from the drowned coast".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
                secrecy: None,
            })
            .await
            .unwrap();

        svc.repository()
            .update_entity_field(
                "canonical_fact",
                &out.canonical_fact_id,
                "secret",
                serde_json::json!(1),
            )
            .await
            .unwrap();
        svc.repository()
            .update_entity_field(
                "canonical_fact",
                &out.canonical_fact_id,
                "concealment_note",
                serde_json::json!("never volunteered"),
            )
            .await
            .unwrap();

        let fact = svc
            .repository()
            .get_canonical_fact(&out.canonical_fact_id)
            .await
            .unwrap();
        assert!(fact.secret, "retrofit set the secret flag");
        assert_eq!(fact.concealment_note.as_deref(), Some("never volunteered"));
    }

    /// A4 no-secrets regression pin (design §5 invariant): a project with
    /// ZERO secret facts produces get_scene_context markdown with no
    /// "[SECRETS" substring, and registering a plain (non-secret) canonical
    /// fact does not change that. Part A does not wire the context gate, so
    /// this passes trivially — it exists to pin the invariant Part B must
    /// inherit (byte-identical no-secrets behavior).
    #[tokio::test]
    async fn no_secrets_context_markdown_has_no_secrets_block() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, character_id, scene_id, location_id) = scaffold(&svc).await;

        // A plain, non-secret canonical fact — the ordinary case.
        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project_id.clone(),
            scene_id: Some(scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(character_id.clone()),
            predicate: Some("oath".into()),
            value_kind: Some("string".into()),
            value_text: Some("ash gate warden".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();

        let envelope = svc
            .get_scene_context_envelope(GetSceneContextInput {
                project_id: project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 2,
                character_ids: vec![character_id.clone()],
                max_character_count: None,
                location_id: location_id.clone(),
                format: Some(ContextFormat::Markdown),
                budget_tokens: Some(4000),
                token_budget: None,
                sections: None,
            })
            .await
            .unwrap();
        let markdown = envelope
            .context_markdown
            .as_deref()
            .expect("markdown requested");
        assert!(
            !markdown.contains("[SECRETS"),
            "a project with no secret facts must not emit a secrets block"
        );
        assert!(
            markdown.contains("ash gate warden"),
            "the plain canonical fact still renders unchanged"
        );
    }

    // ─────────────────────────────────────────────────────────────────
    // B1: context-gate wiring in get_scene_context (design §2.2).
    // ─────────────────────────────────────────────────────────────────

    use spindle_core::models::{PlanChapterInput, PlanChapterSceneInput, StoryPlacement};

    /// Register `is a reincarnated warden` as a secret held by `holder`.
    /// Returns the canonical_fact_id.
    async fn declare_reincarnation_secret(
        svc: &SqliteSpindleService,
        project_id: &str,
        scene_id: &str,
        subject_id: &str,
        holder: &str,
        concealment_note: Option<&str>,
    ) -> String {
        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project_id.to_string(),
            scene_id: Some(scene_id.to_string()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(subject_id.to_string()),
            predicate: Some("reincarnation".into()),
            value_kind: Some("string".into()),
            value_text: Some("is a reincarnated warden".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: Some(SecrecyScope {
                holder_ids: vec![holder.to_string()],
                concealment_note: concealment_note.map(str::to_string),
            }),
        })
        .await
        .unwrap()
        .canonical_fact_id
    }

    /// Create a second character (returns the character id).
    async fn make_character(svc: &SqliteSpindleService, project_id: &str, name: &str) -> String {
        svc.create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project_id.to_string(),
            name: name.to_string(),
            summary: format!("{name} is present."),
            role: "supporting".into(),
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
            initial_state: None,
        })
        .await
        .unwrap()
        .character_id
    }

    /// Fetch the chapter-1 scene-2 context envelope for the given cast.
    async fn context_for(
        svc: &SqliteSpindleService,
        project_id: &str,
        location_id: &str,
        character_ids: Vec<String>,
    ) -> spindle_core::models::SceneContextEnvelope {
        svc.get_scene_context_envelope(GetSceneContextInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            character_ids,
            max_character_count: None,
            location_id: location_id.to_string(),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap()
    }

    /// B1 envelope: a non-POV circle member present with an out-of-circle
    /// character in the room → the secret renders in a `[SECRETS IN PLAY]`
    /// hard-constraint block naming the insider (Known ONLY to) and the
    /// unaware character, and it does NOT appear as an ordinary canonical
    /// fact hard-constraint line or read-model entry.
    #[tokio::test]
    async fn envelope_block_renders_and_secret_absent_from_normal_carriers() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, mara, scene_id, location_id) = scaffold(&svc).await;
        let bran = make_character(&svc, &project_id, "Bran").await;
        let fact_id = declare_reincarnation_secret(
            &svc,
            &project_id,
            &scene_id,
            &mara,
            &mara,
            Some("she deflects with dry humor"),
        )
        .await;

        let envelope = context_for(
            &svc,
            &project_id,
            &location_id,
            vec![mara.clone(), bran.clone()],
        )
        .await;
        let markdown = envelope.context_markdown.as_deref().unwrap();

        assert!(
            markdown.contains("[SECRETS IN PLAY]"),
            "an insider-present secret must render the envelope block: {markdown}"
        );
        assert!(
            markdown.contains("Known ONLY to: Mara"),
            "the block names the insider by name: {markdown}"
        );
        assert!(
            markdown.contains("Present and NOT in the know: Bran"),
            "the block names the unaware present character: {markdown}"
        );
        assert!(
            markdown.contains("must not reference, imply, or react"),
            "the block carries the must-not-reference instruction: {markdown}"
        );
        assert!(
            markdown.contains("she deflects with dry humor"),
            "the concealment note rides the envelope: {markdown}"
        );

        // The secret must NOT appear as an ordinary reincarnation fact line
        // outside the envelope block. The value string appears exactly once
        // (inside the block), never on a plain "**reincarnation**:" line.
        assert!(
            !markdown.contains("**reincarnation**: is a reincarnated warden"),
            "the secret must not render as a normal canonical-fact hard constraint: {markdown}"
        );

        // Structured read models must not carry the secret fact.
        assert!(
            envelope
                .novel
                .subjects
                .iter()
                .flat_map(|s| s.canonical_facts())
                .all(|f| !f.fact.contains("is a reincarnated warden")),
            "subject snapshots must not surface the withheld secret fact"
        );
        let _ = fact_id;
    }

    /// B1 withhold: no circle member present (and no POV insider) → the
    /// secret is stripped from EVERY carrier — markdown, subject snapshots,
    /// and there is no `[SECRETS IN PLAY]` block at all.
    #[tokio::test]
    async fn no_insider_present_withholds_from_all_carriers() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, mara, scene_id, location_id) = scaffold(&svc).await;
        let bran = make_character(&svc, &project_id, "Bran").await;
        declare_reincarnation_secret(&svc, &project_id, &scene_id, &mara, &mara, None).await;

        // Only Bran (out of circle) is present; Mara is not in the cast and
        // there is no chapter plan POV, so no insider is present.
        let envelope = context_for(&svc, &project_id, &location_id, vec![bran.clone()]).await;
        let markdown = envelope.context_markdown.as_deref().unwrap();

        assert!(
            !markdown.contains("[SECRETS IN PLAY]"),
            "no insider present → no envelope block: {markdown}"
        );
        assert!(
            !markdown.contains("is a reincarnated warden"),
            "the secret text must be absent from the markdown entirely: {markdown}"
        );
        assert!(
            envelope
                .novel
                .subjects
                .iter()
                .flat_map(|s| s.canonical_facts())
                .all(|f| !f.fact.contains("is a reincarnated warden")),
            "the withheld secret must not surface in any subject snapshot"
        );
    }

    /// B1 knowledge-briefing withhold: when a holder's own knowledge row for
    /// the secret would otherwise brief into context but no insider is
    /// present, the briefing entry linked to the secret is stripped too.
    #[tokio::test]
    async fn no_insider_present_withholds_knowledge_briefing_entry() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, mara, scene_id, location_id) = scaffold(&svc).await;
        let bran = make_character(&svc, &project_id, "Bran").await;
        let fact_id =
            declare_reincarnation_secret(&svc, &project_id, &scene_id, &mara, &mara, None).await;

        // Reveal to Bran at ch 12 — a placement-stamped circle row that also
        // creates a knowledge briefing entry linked to the secret.
        svc.record_knowledge(RecordKnowledgeInput {
            project_id: project_id.clone(),
            branch_id: None,
            character_id: bran.clone(),
            fact: "Mara is a reincarnated warden.".into(),
            source_summary: "she finally told him".into(),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 12,
                scene_order: Some(1),
                note: None,
            }),
            confidence: None,
            tags: Vec::new(),
            reader_visible: true,
            secret_of_fact_id: Some(fact_id.clone()),
        })
        .await
        .unwrap();

        // Scene is ch 1 scene 2 (cursor before the ch-12 reveal), Bran alone:
        // Bran is not yet in the circle at this cursor, so no insider present.
        let envelope = context_for(&svc, &project_id, &location_id, vec![bran.clone()]).await;
        assert!(
            envelope
                .novel
                .knowledge_briefing
                .iter()
                .all(|item| !item.fact.contains("reincarnated warden")),
            "the secret-linked briefing entry must be withheld when no insider is present"
        );
        let markdown = envelope.context_markdown.as_deref().unwrap();
        assert!(
            !markdown.contains("[SECRETS IN PLAY]"),
            "cursor precedes the reveal → Bran is not an insider → no block: {markdown}"
        );
    }

    /// B1 POV-only: the POV character is the only present circle member →
    /// the envelope carries the private-awareness narration line.
    #[tokio::test]
    async fn pov_only_insider_renders_narration_line() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, mara, scene_id, location_id) = scaffold(&svc).await;
        let bran = make_character(&svc, &project_id, "Bran").await;
        declare_reincarnation_secret(&svc, &project_id, &scene_id, &mara, &mara, None).await;

        // Chapter plan sets Mara as POV; scene cast has Mara + Bran, so Mara
        // is the only present circle member and is the POV → PovEnvelope.
        svc.plan_chapter(PlanChapterInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            pov_character_id: Some(mara.clone()),
            synopsis: "Mara keeps her secret.".into(),
            target_theme_ids: Vec::new(),
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes: vec![PlanChapterSceneInput {
                scene_order: 2,
                summary: "Mara and Bran talk".into(),
                beat_structure: vec!["tension".into()],
                character_ids: vec![mara.clone(), bran.clone()],
                purpose: "development".into(),
                ..Default::default()
            }],
        })
        .await
        .unwrap();

        let envelope = context_for(
            &svc,
            &project_id,
            &location_id,
            vec![mara.clone(), bran.clone()],
        )
        .await;
        let markdown = envelope.context_markdown.as_deref().unwrap();
        assert!(
            markdown.contains("[SECRETS IN PLAY]"),
            "the POV insider still gets an envelope: {markdown}"
        );
        assert!(
            markdown.contains("Narration may carry")
                && markdown.contains("private awareness")
                && markdown.contains("dialogue and other characters' behavior must not"),
            "the POV-only variant carries the private-awareness narration line: {markdown}"
        );
    }

    /// B1 flashback cursor: a reveal placed at ch 12 does not make the
    /// recipient an insider for a ch-9-cursor scene drafted later. With the
    /// original holder absent and the recipient present, the recipient is
    /// listed as unaware (or the secret is withheld) — never leaked.
    #[tokio::test]
    async fn flashback_cursor_keeps_future_recipient_unaware() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, mara, scene_id, location_id) = scaffold(&svc).await;
        let bran = make_character(&svc, &project_id, "Bran").await;
        let fact_id =
            declare_reincarnation_secret(&svc, &project_id, &scene_id, &mara, &mara, None).await;

        // Reveal to Bran at ch 12.
        svc.record_knowledge(RecordKnowledgeInput {
            project_id: project_id.clone(),
            branch_id: None,
            character_id: bran.clone(),
            fact: "Mara is a reincarnated warden.".into(),
            source_summary: "later reveal".into(),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 12,
                scene_order: Some(1),
                note: None,
            }),
            confidence: None,
            tags: Vec::new(),
            reader_visible: true,
            secret_of_fact_id: Some(fact_id.clone()),
        })
        .await
        .unwrap();

        // A ch-9 scene: cursor precedes the ch-12 reveal. Mara (holder) is
        // present too, so the envelope renders — but Bran must be UNAWARE.
        let envelope = svc
            .get_scene_context_envelope(GetSceneContextInput {
                project_id: project_id.clone(),
                book_number: 1,
                chapter_number: 9,
                chapter_id: None,
                scene_order: 1,
                character_ids: vec![mara.clone(), bran.clone()],
                max_character_count: None,
                location_id: location_id.clone(),
                format: Some(ContextFormat::Markdown),
                budget_tokens: Some(8000),
                token_budget: None,
                sections: None,
            })
            .await
            .unwrap();
        let markdown = envelope.context_markdown.as_deref().unwrap();
        assert!(
            markdown.contains("[SECRETS IN PLAY]"),
            "the holder Mara is present at ch 9 → envelope renders: {markdown}"
        );
        assert!(
            markdown.contains("Present and NOT in the know: Bran"),
            "a ch-12 reveal must not leak backward: Bran is unaware at ch 9: {markdown}"
        );
        assert!(
            !markdown.contains("Known ONLY to: Bran")
                && !markdown.contains("Known ONLY to: Mara, Bran"),
            "Bran must not be listed as an insider before the reveal cursor: {markdown}"
        );
    }

    // ── Item 6: reveal linking in the save_scene_draft continuity package ──
    // The design's path 2 of §2.3: a draft-time on-page reveal flagged in the
    // save's `knowledge_learned` package becomes a knowledge_fact row linked
    // to the secret via `secret_of_fact_id`, learned_at = this scene's
    // placement — expanding the circle from that scene forward.
    use spindle_core::models::KnowledgeLearnedEntry;

    /// Save a scene at (chapter, scene_order) carrying a knowledge_learned
    /// package. Returns the SaveSceneDraftOutput result.
    async fn save_with_knowledge(
        svc: &SqliteSpindleService,
        project_id: &str,
        chapter: i32,
        scene_order: i32,
        knowledge_learned: Vec<KnowledgeLearnedEntry>,
    ) -> anyhow::Result<spindle_core::models::SaveSceneDraftOutput> {
        if chapter > 1 {
            let _ = svc
                .create_chapter(spindle_core::models::CreateChapterInput {
                    project_id: project_id.to_string(),
                    book_id: None,
                    book_number: Some(1),
                    chapter_number: Some(chapter),
                    title: None,
                })
                .await;
        }
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: chapter,
            chapter_id: None,
            scene_order,
            full_text: "Mara finally told Bran the truth.".into(),
            summary: "the reveal".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            knowledge_learned,
            ..Default::default()
        })
        .await
    }

    async fn context_at(
        svc: &SqliteSpindleService,
        project_id: &str,
        location_id: &str,
        chapter: i32,
        scene_order: i32,
        character_ids: Vec<String>,
    ) -> spindle_core::models::SceneContextEnvelope {
        svc.get_scene_context_envelope(GetSceneContextInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: chapter,
            chapter_id: None,
            scene_order,
            character_ids,
            max_character_count: None,
            location_id: location_id.to_string(),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap()
    }

    /// A reveal flagged in the save package expands the circle: at a later
    /// cursor with the revealed-to character present, the envelope lists them
    /// among the insiders (Known ONLY to), not the unaware.
    #[tokio::test]
    async fn reveal_via_package_expands_circle() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, mara, scene_id, location_id) = scaffold(&svc).await;
        let bran = make_character(&svc, &project_id, "Bran").await;
        let fact_id =
            declare_reincarnation_secret(&svc, &project_id, &scene_id, &mara, &mara, None).await;

        // Reveal to Bran on-page at chapter 5.
        save_with_knowledge(
            &svc,
            &project_id,
            5,
            1,
            vec![KnowledgeLearnedEntry {
                character_id: bran.clone(),
                fact: "Mara is a reincarnated warden.".into(),
                source_summary: Some("she finally told him".into()),
                secret_of_fact_id: Some(fact_id.clone()),
                reader_visible: Some(true),
            }],
        )
        .await
        .unwrap();

        // Later cursor (ch 6): Bran is now inside the circle.
        let envelope = context_at(
            &svc,
            &project_id,
            &location_id,
            6,
            1,
            vec![mara.clone(), bran.clone()],
        )
        .await;
        let markdown = envelope.context_markdown.as_deref().unwrap();
        assert!(
            markdown.contains("[SECRETS IN PLAY]"),
            "the holder is present → the envelope renders: {markdown}"
        );
        assert!(
            markdown.contains("Known ONLY to: Bran, Mara")
                || markdown.contains("Known ONLY to: Mara, Bran"),
            "the package reveal put Bran inside the circle at a later cursor: {markdown}"
        );
        assert!(
            !markdown.contains("Present and NOT in the know: Bran"),
            "Bran must no longer be listed as unaware after the reveal: {markdown}"
        );
    }

    /// Flashback-safety: learned_at is stamped with the reveal scene's
    /// placement, so a context at an EARLIER cursor still withholds Bran.
    #[tokio::test]
    async fn package_reveal_is_placement_stamped_no_backward_leak() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, mara, scene_id, location_id) = scaffold(&svc).await;
        let bran = make_character(&svc, &project_id, "Bran").await;
        let fact_id =
            declare_reincarnation_secret(&svc, &project_id, &scene_id, &mara, &mara, None).await;

        save_with_knowledge(
            &svc,
            &project_id,
            5,
            1,
            vec![KnowledgeLearnedEntry {
                character_id: bran.clone(),
                fact: "Mara is a reincarnated warden.".into(),
                source_summary: Some("she finally told him".into()),
                secret_of_fact_id: Some(fact_id.clone()),
                reader_visible: Some(true),
            }],
        )
        .await
        .unwrap();

        // Earlier cursor (ch 2): Bran is NOT yet in the circle.
        let envelope = context_at(
            &svc,
            &project_id,
            &location_id,
            2,
            1,
            vec![mara.clone(), bran.clone()],
        )
        .await;
        let markdown = envelope.context_markdown.as_deref().unwrap();
        assert!(
            markdown.contains("Present and NOT in the know: Bran"),
            "a ch-5 reveal must not leak backward into a ch-2 context: {markdown}"
        );
        assert!(
            !markdown.contains("Known ONLY to: Bran")
                && !markdown.contains("Known ONLY to: Mara, Bran")
                && !markdown.contains("Known ONLY to: Bran, Mara"),
            "Bran must not be an insider before the reveal cursor: {markdown}"
        );
    }

    /// An invalid secret link (pointing at a NON-secret fact) fails the save
    /// with a domain error atomically: no scene row, no knowledge rows.
    #[tokio::test]
    async fn invalid_secret_link_fails_save_atomically() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, mara, scene_id, _location_id) = scaffold(&svc).await;
        let bran = make_character(&svc, &project_id, "Bran").await;

        // Register a PLAIN (non-secret) fact.
        let plain = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project_id.clone(),
                scene_id: Some(scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".into()),
                subject_id: Some(mara.clone()),
                predicate: Some("hair_color".into()),
                value_kind: Some("string".into()),
                value_text: Some("black".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
                secrecy: None,
            })
            .await
            .unwrap()
            .canonical_fact_id;

        let branch = svc
            .repository()
            .get_active_branch(&project_id)
            .await
            .unwrap();
        let scenes_before = svc
            .repository()
            .list_scenes_by_project_and_branch(&project_id, &branch.id)
            .await
            .unwrap()
            .len();
        let knowledge_before = svc
            .repository()
            .list_knowledge_facts_by_project_and_branch(&project_id, &branch.id)
            .await
            .unwrap()
            .iter()
            .filter(|r| r.character_id == bran)
            .count();

        let result = save_with_knowledge(
            &svc,
            &project_id,
            5,
            1,
            vec![KnowledgeLearnedEntry {
                character_id: bran.clone(),
                fact: "Mara is a reincarnated warden.".into(),
                source_summary: Some("bogus".into()),
                secret_of_fact_id: Some(plain.clone()),
                reader_visible: Some(true),
            }],
        )
        .await;
        assert!(result.is_err(), "an invalid secret link must fail the save");

        let scenes_after = svc
            .repository()
            .list_scenes_by_project_and_branch(&project_id, &branch.id)
            .await
            .unwrap()
            .len();
        let knowledge_after = svc
            .repository()
            .list_knowledge_facts_by_project_and_branch(&project_id, &branch.id)
            .await
            .unwrap()
            .iter()
            .filter(|r| r.character_id == bran)
            .count();
        assert_eq!(
            scenes_after, scenes_before,
            "no scene row may be written when the package validation fails"
        );
        assert_eq!(
            knowledge_after, knowledge_before,
            "no knowledge row may be written when the package validation fails"
        );
    }

    /// A plain knowledge_learned entry with no secret link works as ordinary
    /// per-character knowledge (writes a knowledge_fact row, no circle math).
    #[tokio::test]
    async fn plain_knowledge_learned_entry_records_ordinary_knowledge() {
        let (_tmp, svc) = fresh_service().await;
        let (project_id, _mara, _scene_id, _location_id) = scaffold(&svc).await;
        let bran = make_character(&svc, &project_id, "Bran").await;

        save_with_knowledge(
            &svc,
            &project_id,
            3,
            1,
            vec![KnowledgeLearnedEntry {
                character_id: bran.clone(),
                fact: "The bridge is out past the mill.".into(),
                source_summary: Some("he saw it".into()),
                secret_of_fact_id: None,
                reader_visible: Some(true),
            }],
        )
        .await
        .unwrap();

        let branch = svc
            .repository()
            .get_active_branch(&project_id)
            .await
            .unwrap();
        let rows = svc
            .repository()
            .list_knowledge_facts_by_project_and_branch(&project_id, &branch.id)
            .await
            .unwrap();
        assert!(
            rows.iter()
                .any(|r| r.character_id == bran && r.fact.contains("bridge is out")),
            "an ordinary knowledge_learned entry writes a knowledge_fact row: {rows:?}"
        );
    }
}
