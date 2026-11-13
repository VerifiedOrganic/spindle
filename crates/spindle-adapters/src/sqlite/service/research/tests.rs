use super::*;

#[tokio::test]
async fn test_research_subsystem_workflow() {
    use spindle_core::models::{
        ContentRating, CreateProjectInput, PlanChapterInput, PlanChapterSceneInput, ReaderContract,
        ResearchIngestReportInput, ResearchPlanForSceneInput, ResearchQueryInput,
        SaveSceneDraftInput, SourcePolicy,
    };

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Vegas Mob".into(),
            project_type: "novel".into(),
            genre: "historical-crime".into(),
            reader_contract: ReaderContract {
                promise: "Gritty 70s Vegas".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // 1. Run a structured query that persists mock sources/notes/claims
    let out = svc
        .research_query(ResearchQueryInput {
            project_id: proj.project_id.clone(),
            query: "MOCK_VAL: details on skim operations".into(),
            context_hint: None,
            branch_id: None,
            rating: None,
            tags: Some(vec!["skim".into()]),
            scene_id: None,
            store: true,
            source_policy: Some(SourcePolicy::RequireSources),
        })
        .await
        .unwrap();

    assert_eq!(out.summary, "Mock research summary");
    assert_eq!(out.sources.len(), 1);
    assert_eq!(out.sources[0].title, "Mock Source");
    assert_eq!(out.claims.len(), 1);
    assert_eq!(out.claims[0].claim, "Mock claim");
    assert!(out.tags.iter().any(|tag| tag == "skim"));
    assert!(out.warnings.is_empty());

    // Verify items were persisted in the repository
    let saved_sources = svc
        .repository
        .list_research_sources_by_project(&proj.project_id)
        .await
        .unwrap();
    assert_eq!(saved_sources.len(), 1);
    assert_eq!(saved_sources[0].title, "Mock Source");
    assert!(saved_sources[0].tags.iter().any(|tag| tag == "skim"));

    let saved_claims = svc
        .repository
        .list_research_claims_by_project(&proj.project_id)
        .await
        .unwrap();
    assert_eq!(saved_claims.len(), 1);
    assert_eq!(saved_claims[0].claim, "Mock claim");
    assert!(saved_claims[0].tags.iter().any(|tag| tag == "skim"));

    // 2. Ingest a research report
    let report_out = svc
        .research_ingest_report(ResearchIngestReportInput {
            project_id: proj.project_id.clone(),
            branch_id: None,
            title: "MobSkimming".into(),
            report: "MOCK_VAL: mob skim details".into(),
            tags: vec!["ingest-tag".into()],
            rating: None,
            source_policy: None,
        })
        .await
        .unwrap();

    assert_eq!(report_out.summary, "Mock research summary");
    assert_eq!(report_out.sources.len(), 1);
    assert!(report_out.tags.iter().any(|tag| tag == "ingest-tag"));
    let saved_sources_after_report = svc
        .repository
        .list_research_sources_by_project(&proj.project_id)
        .await
        .unwrap();
    assert!(
        saved_sources_after_report
            .iter()
            .any(|source| source.tags.iter().any(|tag| tag == "ingest-tag"))
    );

    // 3. Plan research for a scene
    // Setup a planned scene
    let active_branch = svc
        .repository
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();
    let _plan = svc
        .plan_chapter(PlanChapterInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            pov_character_id: None,
            synopsis: "The boss gets sketch outline.".into(),
            target_theme_ids: Vec::new(),
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes: vec![PlanChapterSceneInput {
                scene_order: 1,
                summary: "Casino skim meetup.".into(),
                beat_structure: Vec::new(),
                character_ids: Vec::new(),
                purpose: "establish skim stakes".into(),
                research_required: Some(true),
                research_tags: vec!["skim-tag".into()],
                explicit_query: Some("Casino skim operations".into()),
                ..Default::default()
            }],
        })
        .await
        .unwrap();

    // Create the scene in the DB so it exists for scene_id
    let scene_ref = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: "Prose draft...".into(),
            summary: "Casino skim meetup.".into(),
            content_rating: ContentRating::General,
            ..Default::default()
        })
        .await
        .unwrap();

    // Check plan before research is added with "skim-tag": it should block
    let plan_check = svc
        .research_plan_for_scene(ResearchPlanForSceneInput {
            project_id: proj.project_id.clone(),
            scene_id: scene_ref.scene_id.clone(),
            planned_scene_summary: None,
            rating: None,
            tags: None,
        })
        .await
        .unwrap();

    assert!(plan_check.await_research);
    assert!(plan_check.missing_tags.contains(&"skim-tag".to_string()));

    // Add a claim with "skim-tag"
    svc.research_add_claim(spindle_core::models::ResearchAddClaimInput {
        project_id: proj.project_id.clone(),
        source_id: None,
        note_id: None,
        branch_id: Some(active_branch.id.clone()),
        claim: "Casino skimming is verified".into(),
        topic: Some("Skim".into()),
        time_period: None,
        location: None,
        confidence: "verified".into(),
        tags: vec!["skim-tag".into()],
    })
    .await
    .unwrap();

    // Check plan again: it should now NOT block because "skim-tag" is satisfied
    let plan_check_after = svc
        .research_plan_for_scene(ResearchPlanForSceneInput {
            project_id: proj.project_id.clone(),
            scene_id: scene_ref.scene_id.clone(),
            planned_scene_summary: None,
            rating: None,
            tags: None,
        })
        .await
        .unwrap();

    assert!(!plan_check_after.await_research);
    assert!(plan_check_after.missing_tags.is_empty());
}

#[tokio::test]
async fn test_research_query_explicit_requires_explicit_route() {
    use spindle_core::models::{CreateProjectInput, ReaderContract, ResearchQueryInput};

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Vegas Mob Explicit".into(),
            project_type: "novel".into(),
            genre: "historical-crime".into(),
            reader_contract: ReaderContract {
                promise: "Gritty 70s Vegas".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let err = svc
        .research_query(ResearchQueryInput {
            project_id: proj.project_id.clone(),
            query: "MOCK_PROSE: adult entertainment details".into(),
            context_hint: None,
            branch_id: None,
            rating: Some("explicit".into()),
            tags: None,
            scene_id: None,
            store: false,
            source_policy: None,
        })
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("explicit rating requested but no explicit research route configured"),
        "expected explicit research route guard, got: {err}"
    );
}

#[test]
fn research_prompt_instructions_include_fiction_safety_frame() {
    assert!(STRUCTURED_RESEARCH_PROMPT_INSTRUCTIONS.contains("fictional book project"));
    assert!(STRUCTURED_RESEARCH_PROMPT_INSTRUCTIONS.contains("fiction writing"));
    assert!(STRUCTURED_RESEARCH_PROMPT_INSTRUCTIONS.contains("Do not write story prose"));
    assert!(STRUCTURED_RESEARCH_PROMPT_INSTRUCTIONS.contains("operational instructions"));

    assert!(EXPLICIT_RESEARCH_SYSTEM_APPENDIX.contains("fictional book project"));
    assert!(EXPLICIT_RESEARCH_SYSTEM_APPENDIX.contains("consenting adults"));
    assert!(EXPLICIT_RESEARCH_SYSTEM_APPENDIX.contains("sexual-health"));
    assert!(EXPLICIT_RESEARCH_SYSTEM_APPENDIX.contains("pickup tactics"));
    assert!(EXPLICIT_RESEARCH_SYSTEM_APPENDIX.contains("real-world operational instructions"));
}

#[tokio::test]
async fn test_research_query_explicit_warnings() {
    use spindle_core::models::{
        ConfigureAgentsInput, CreateProjectInput, ReaderContract, ResearchQueryInput,
    };

    let (tmp, svc) = fresh_service_local().await;
    let config_path = tmp.path().join("research-routes.toml");
    std::fs::write(
        &config_path,
        r#"
[health_check]
enabled = false

[[agents]]
id = "research-local"
name = "Research Local"
provider = "local"
endpoint = "local"
model = "research-local"
ratings = ["safe", "mature", "explicit"]

[[routing]]
route = "research"
agent = "research-local"

[[routing]]
route = "research"
agent = "research-local"
rating = "explicit"
"#,
    )
    .unwrap();
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.display().to_string()),
    })
    .unwrap();

    let proj = svc
        .create_project(CreateProjectInput {
            name: "Vegas Mob Explicit".into(),
            project_type: "novel".into(),
            genre: "historical-crime".into(),
            reader_contract: ReaderContract {
                promise: "Gritty 70s Vegas".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let out = svc
        .research_query(ResearchQueryInput {
            project_id: proj.project_id.clone(),
            query: "MOCK_PROSE: adult entertainment details".into(),
            context_hint: None,
            branch_id: None,
            rating: Some("explicit".into()),
            tags: None,
            scene_id: None,
            store: false,
            source_policy: None,
        })
        .await
        .unwrap();

    assert!(
        !out.warnings
            .iter()
            .any(|w| w.contains("no explicit research route configured"))
    );
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("adult/explicit query without adult-only boundary language"))
    );
    assert!(out.warnings.iter().any(|w| {
        w.contains("explicit research claim contains no source provenance")
            || w.contains("explicit research result contains no provenance")
    }));
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("prose-like material"))
    );
}

/// Live-run bug 5: on parse failure `research_ingest_report` used to insert a
/// junk "Unparsed Lead" claim carrying the ENTIRE raw model output and return
/// success-shaped output. With no research agent configured, the built-in
/// local research stub errors (bug 6a), so ingest must surface a structured
/// error and write ZERO research rows — never a success-shaped result.
#[tokio::test]
async fn research_ingest_report_errors_on_unparseable_output_and_writes_nothing() {
    use spindle_core::models::{
        CreateProjectInput, ReaderContract, ResearchIngestReportInput, SourcePolicy,
    };

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Ingest Junk".into(),
            project_type: "novel".into(),
            genre: "historical-crime".into(),
            reader_contract: ReaderContract {
                promise: "Gritty".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // No `MOCK_` sentinel: the local research stub returns a structured
    // "no model configured" error rather than an echo. AllowUnsourcedLeads
    // must NOT resurrect the junk "Unparsed Lead" fallback.
    let err = svc
        .research_ingest_report(ResearchIngestReportInput {
            project_id: proj.project_id.clone(),
            branch_id: None,
            title: "Unparseable Report".into(),
            report: "some free-form prose that is not structured research".into(),
            tags: vec!["ingest-tag".into()],
            rating: None,
            source_policy: Some(SourcePolicy::AllowUnsourcedLeads),
        })
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        !msg.contains("Unparsed Lead"),
        "ingest must not create a junk lead claim, got: {msg}"
    );

    // Zero research rows must have been written.
    let claims = svc
        .repository
        .list_research_claims_by_project(&proj.project_id)
        .await
        .unwrap();
    assert!(
        claims.is_empty(),
        "no claims must be written on ingest failure, got: {claims:?}"
    );
    let sources = svc
        .repository
        .list_research_sources_by_project(&proj.project_id)
        .await
        .unwrap();
    assert!(
        sources.is_empty(),
        "no sources must be written on ingest failure, got: {sources:?}"
    );
}

/// Live-run bug 6a: an unconfigured `research` route landed on the local
/// stub, which echoed the prompt back as plausible research output. With no
/// research agent configured (built-in default local stub) and no `MOCK_`
/// sentinel, `research_query` must fail with a structured error and write no
/// echo-derived research rows.
#[tokio::test]
async fn research_query_unconfigured_route_errors_instead_of_echoing() {
    use spindle_core::models::{CreateProjectInput, ReaderContract, ResearchQueryInput};

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Echo Guard".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "epic".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let err = svc
        .research_query(ResearchQueryInput {
            project_id: proj.project_id.clone(),
            query: "what does decompression sickness feel like".into(),
            context_hint: None,
            branch_id: None,
            rating: None,
            tags: None,
            scene_id: None,
            store: true,
            source_policy: None,
        })
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("no model configured for route 'research'"),
        "expected structured no-model error, got: {msg}"
    );
    assert!(
        !msg.contains("Local research adapter investigated"),
        "must not echo the prompt back, got: {msg}"
    );

    let claims = svc
        .repository
        .list_research_claims_by_project(&proj.project_id)
        .await
        .unwrap();
    assert!(claims.is_empty(), "no echo-ingest rows, got: {claims:?}");
}

/// Live-run bug 5: archived research rows must be excluded wherever research
/// is consumed — here `research_pack_for_scene`. A claim added then archived
/// must not appear in the pack.
#[tokio::test]
async fn research_pack_for_scene_excludes_archived_rows() {
    use spindle_core::models::{
        ArchiveEntityInput, CreateProjectInput, ReaderContract, ResearchAddClaimInput,
        ResearchPackForSceneInput,
    };

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Pack Archive".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "epic".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let claim_out = svc
        .research_add_claim(ResearchAddClaimInput {
            project_id: proj.project_id.clone(),
            source_id: None,
            note_id: None,
            branch_id: None,
            claim: "Salt wards repel the mist".into(),
            topic: Some("wards".into()),
            time_period: None,
            location: None,
            confidence: "verified".into(),
            tags: vec!["wards".into()],
        })
        .await
        .unwrap();

    // Present before archival.
    let pack_before = svc
        .research_pack_for_scene(ResearchPackForSceneInput {
            project_id: proj.project_id.clone(),
            branch_id: None,
            scene_summary: None,
            scene_location: None,
            character_ids: vec![],
            tags: vec!["wards".into()],
            explicit_query: None,
            limit: Some(10),
        })
        .await
        .unwrap();
    assert_eq!(pack_before.claims.len(), 1, "claim present before archival");

    svc.archive_entity(ArchiveEntityInput {
        entity_type: "research_claim".into(),
        entity_id: claim_out.claim_id.clone(),
    })
    .await
    .unwrap();

    let pack_after = svc
        .research_pack_for_scene(ResearchPackForSceneInput {
            project_id: proj.project_id.clone(),
            branch_id: None,
            scene_summary: None,
            scene_location: None,
            character_ids: vec![],
            tags: vec!["wards".into()],
            explicit_query: None,
            limit: Some(10),
        })
        .await
        .unwrap();
    assert!(
        pack_after.claims.is_empty(),
        "archived claim must be excluded from the pack, got: {:?}",
        pack_after.claims
    );
}

#[tokio::test]
async fn research_library_integration_test() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Vegas 1970s".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "grounded in research".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let project_id = proj.project_id;

    // 1. Add research source
    let add_src_out = svc
        .research_add_source(ResearchAddSourceInput {
            project_id: project_id.clone(),
            branch_id: None,
            title: "Las Vegas in the 1970s".to_string(),
            source_type: "book".to_string(),
            url: Some("https://example.com/vegas-70s".to_string()),
            file_path: None,
            author: Some("John Doe".to_string()),
            publisher: Some("Vegas Press".to_string()),
            published_date: Some("1982-05-12".to_string()),
            accessed_at: Some(1786523901000000),
            reliability: "primary".to_string(),
            tags: vec![
                "Vegas  ".to_string(),
                "  1970s".to_string(),
                "Vegas".to_string(),
                "".to_string(),
            ],
            summary: Some(
                "Detailed account of the casino expansion and culture in 1970s Las Vegas."
                    .to_string(),
            ),
        })
        .await
        .unwrap();
    let source_id = add_src_out.source_id;
    assert!(source_id.starts_with("research_source:"));

    // 2. Add research note linked to source
    let add_note_out = svc.research_add_note(ResearchAddNoteInput {
            project_id: project_id.clone(),
            source_id: Some(source_id.clone()),
            branch_id: None,
            note: "The Stardust casino had a massive neon sign built in 1958 and modernized in the 70s.".to_string(),
            quote: Some("Stardust neon sign was a beacon of the Strip".to_string()),
            locator: Some("page 45".to_string()),
            tags: vec!["Stardust".to_string(), "neon".to_string()],
        }).await.unwrap();
    let note_id = add_note_out.note_id;
    assert!(note_id.starts_with("research_note:"));

    // 3. Add research claim linked to source and note
    let add_claim_out = svc
        .research_add_claim(ResearchAddClaimInput {
            project_id: project_id.clone(),
            source_id: None,
            note_id: Some(note_id.clone()),
            branch_id: None,
            claim:
                "Casinos used mob skimming techniques extensively until the late 70s crackdowns."
                    .to_string(),
            topic: Some("mob skimming".to_string()),
            time_period: Some("1970s".to_string()),
            location: Some("Las Vegas Strip".to_string()),
            confidence: "high".to_string(),
            tags: vec!["mob".to_string(), "crime".to_string()],
        })
        .await
        .unwrap();
    let claim_id = add_claim_out.claim_id;
    assert!(claim_id.starts_with("research_claim:"));
    let stored_claim = svc
        .repository
        .get_research_claim(&claim_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored_claim.source_id.as_deref(), Some(source_id.as_str()));

    // 4. Verify tag normalization and deduping
    let tags_out = svc
        .repository
        .research_tags_by_project(&project_id)
        .await
        .unwrap();
    assert!(tags_out.contains(&"Vegas".to_string()));
    assert!(tags_out.contains(&"1970s".to_string()));
    assert!(tags_out.contains(&"Stardust".to_string()));
    assert!(tags_out.contains(&"neon".to_string()));
    assert!(tags_out.contains(&"mob".to_string()));
    assert!(tags_out.contains(&"crime".to_string()));
    assert!(!tags_out.contains(&"".to_string()));
    assert!(!tags_out.contains(&"Vegas  ".to_string()));

    // 5. Search research and verify results and provenance
    let search_out = svc
        .research_search(ResearchSearchInput {
            project_id: project_id.clone(),
            branch_id: None,
            query: "Stardust".to_string(),
            tags: Vec::new(),
            time_period: None,
            location: None,
            limit: None,
        })
        .await
        .unwrap();
    assert!(!search_out.results.is_empty());
    let item = &search_out.results[0];
    assert_eq!(item.item_type, "note");
    assert_eq!(
        item.source_title,
        Some("Las Vegas in the 1970s".to_string())
    );
    assert_eq!(item.source_author, Some("John Doe".to_string()));
    assert_eq!(
        item.source_url,
        Some("https://example.com/vegas-70s".to_string())
    );
    assert_eq!(item.locator, Some("page 45".to_string()));

    let punctuation_search = svc
        .research_search(ResearchSearchInput {
            project_id: project_id.clone(),
            branch_id: None,
            query: "70's Stardust".to_string(),
            tags: vec![" neon ".to_string()],
            time_period: None,
            location: None,
            limit: None,
        })
        .await
        .unwrap();
    assert!(
        punctuation_search
            .results
            .iter()
            .any(|item| item.id == note_id),
        "punctuation-safe FTS query should find the Stardust note"
    );

    let search_claim = svc
        .research_search(ResearchSearchInput {
            project_id: project_id.clone(),
            branch_id: None,
            query: "skimming".to_string(),
            tags: Vec::new(),
            time_period: None,
            location: None,
            limit: None,
        })
        .await
        .unwrap();
    assert!(!search_claim.results.is_empty());
    assert_eq!(search_claim.results[0].item_type, "claim");
    assert!(search_claim.results[0].preview.contains("mob skimming"));

    // 6. Cross-project isolation: create second project
    let proj2 = svc
        .create_project(CreateProjectInput {
            name: "Isolation Test".to_string(),
            project_type: "novel".to_string(),
            genre: "mystery".to_string(),
            reader_contract: ReaderContract {
                promise: "must isolate books".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let search_proj2 = svc
        .research_search(ResearchSearchInput {
            project_id: proj2.project_id.clone(),
            branch_id: None,
            query: "Stardust".to_string(),
            tags: Vec::new(),
            time_period: None,
            location: None,
            limit: None,
        })
        .await
        .unwrap();
    assert!(search_proj2.results.is_empty());

    let proj2_branch = svc
        .repository
        .get_active_branch(&proj2.project_id)
        .await
        .unwrap();
    let wrong_project_branch_err = svc
        .research_add_source(ResearchAddSourceInput {
            project_id: project_id.clone(),
            branch_id: Some(proj2_branch.id.clone()),
            title: "Wrong branch".to_string(),
            source_type: "article".to_string(),
            url: None,
            file_path: None,
            author: None,
            publisher: None,
            published_date: None,
            accessed_at: None,
            reliability: "secondary".to_string(),
            tags: Vec::new(),
            summary: None,
        })
        .await
        .expect_err("foreign project branch must be rejected");
    assert!(
        wrong_project_branch_err
            .to_string()
            .contains("branch does not belong"),
        "got: {wrong_project_branch_err}"
    );

    let alt_branch = svc
        .create_branch(CreateBranchInput {
            project_id: project_id.clone(),
            name: "alternate".to_string(),
            branch_type: "what-if".to_string(),
            description: None,
            parent_branch_id: None,
        })
        .await
        .unwrap();
    let alt_branch_search = svc
        .research_search(ResearchSearchInput {
            project_id: project_id.clone(),
            branch_id: Some(alt_branch.branch_id.clone()),
            query: "Stardust".to_string(),
            tags: Vec::new(),
            time_period: None,
            location: None,
            limit: None,
        })
        .await
        .unwrap();
    assert!(
        alt_branch_search.results.is_empty(),
        "branch-scoped search should not leak active-branch research"
    );
    let mismatched_branch_err = svc
        .research_add_note(ResearchAddNoteInput {
            project_id: project_id.clone(),
            source_id: Some(source_id.clone()),
            branch_id: Some(alt_branch.branch_id.clone()),
            note: "Should not attach to a source from another branch".to_string(),
            quote: None,
            locator: None,
            tags: Vec::new(),
        })
        .await
        .expect_err("note should not attach to a source from another branch");
    assert!(
        mismatched_branch_err
            .to_string()
            .contains("source branch does not match"),
        "got: {mismatched_branch_err}"
    );

    let alt_pack = svc
        .research_pack_for_scene(ResearchPackForSceneInput {
            project_id: project_id.clone(),
            branch_id: Some(alt_branch.branch_id.clone()),
            scene_summary: Some("A casino heist takes place in 1970s Stardust hotel.".to_string()),
            scene_location: Some("Las Vegas Strip".to_string()),
            character_ids: Vec::new(),
            tags: vec!["mob".to_string()],
            explicit_query: None,
            limit: Some(1),
        })
        .await
        .unwrap();
    assert!(alt_pack.sources.is_empty());
    assert!(alt_pack.notes.is_empty());
    assert!(alt_pack.claims.is_empty());

    // 7. Research pack for scene
    let pack_out = svc
        .research_pack_for_scene(ResearchPackForSceneInput {
            project_id: project_id.clone(),
            branch_id: None,
            scene_summary: Some("A casino heist takes place in 1970s Stardust hotel.".to_string()),
            scene_location: Some("Las Vegas Strip".to_string()),
            character_ids: Vec::new(),
            tags: vec!["mob".to_string()],
            explicit_query: None,
            limit: Some(1),
        })
        .await
        .unwrap();
    assert!(pack_out.sources.len() <= 1);
    assert!(pack_out.notes.len() <= 1);
    assert!(pack_out.claims.len() <= 1);
    assert!(!pack_out.claims.is_empty());
    assert_eq!(
        pack_out.claims[0].claim,
        "Casinos used mob skimming techniques extensively until the late 70s crackdowns."
            .to_string()
    );
}

#[tokio::test]
async fn test_check_consistency_research_accuracy() {
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, ContentRating, PlanChapterInput,
        PlanChapterSceneInput, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "research-accuracy-test".into(),
            project_type: "novel".into(),
            genre: "historical".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let branch = svc
        .repository
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();

    // 1. Create a scene plan that requires research
    svc.plan_chapter(PlanChapterInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: None,
        synopsis: "synopsis".to_string(),
        target_theme_ids: Vec::new(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "Historical battle scene".to_string(),
            beat_structure: Vec::new(),
            character_ids: Vec::new(),
            purpose: "action".to_string(),
            research_required: Some(true),
            research_tags: vec!["battle".to_string()],
            explicit_query: None,
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    // 2. Draft the scene with NO research usage
    let saved_scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: "The battle raged on.".to_string(),
            content_rating: ContentRating::Teen,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // 3. Run check_consistency and assert warning for missing usage
    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["research_accuracy".to_string()],
            severity_filter: Vec::new(),
            subjects: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    let missing_usage_issue = out.issues.iter().find(|issue| {
        issue.check_type == "research_accuracy"
            && issue.message.contains("no recorded research usage")
    });
    assert!(missing_usage_issue.is_some());

    svc.repository
        .create_research_usage(
            &proj.project_id,
            &branch.id,
            "manual",
            None,
            &saved_scene.scene_id,
            &["research_source:local".to_string()],
            &[],
            &[],
            r#"{"query":"battle"}"#,
            "usage-hash",
        )
        .await
        .unwrap();

    let scoped_usage = svc
        .list_research_usages_for_scene(&proj.project_id, &saved_scene.scene_id)
        .await
        .unwrap();
    assert_eq!(scoped_usage.len(), 1);

    let other_project = svc
        .create_project(CreateProjectInput {
            name: "usage-scope-other".into(),
            project_type: "novel".into(),
            genre: "historical".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let leaked_usage = svc
        .list_research_usages_for_scene(&other_project.project_id, &saved_scene.scene_id)
        .await
        .unwrap();
    assert!(leaked_usage.is_empty());

    let resource_json = svc
        .read_project_resource(
            &other_project.project_id,
            &format!("research/usage/{}", saved_scene.scene_id),
        )
        .await
        .unwrap();
    assert_eq!(resource_json, serde_json::json!([]));

    svc.repository
        .create_research_claim(
            &proj.project_id,
            None,             // source
            None,             // note
            Some(&branch.id), // branch
            "Some orphaned claim",
            Some("history"),
            None,
            None,
            "high",
            &[],
        )
        .await
        .unwrap();

    // 5. Run check_consistency and assert warnings for both missing usage AND orphaned claim
    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["research_accuracy".to_string()],
            severity_filter: Vec::new(),
            subjects: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    let orphaned_claim_issue = out.issues.iter().find(|issue| {
        issue.check_type == "research_accuracy"
            && issue.message.contains("no note or source provenance")
    });
    assert!(orphaned_claim_issue.is_some());
}

#[tokio::test]
async fn research_search_matches_multi_word_queries_across_fields_and_tags() {
    use spindle_core::models::{
        CreateProjectInput, ReaderContract, ResearchAddClaimInput, ResearchAddNoteInput,
        ResearchAddSourceInput, ResearchSearchInput,
    };

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Search Fixture".to_string(),
            project_type: "novel".to_string(),
            genre: "adventure".to_string(),
            reader_contract: ReaderContract {
                promise: "generic research matching".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let project_id = proj.project_id;

    let source_id = svc
        .research_add_source(ResearchAddSourceInput {
            project_id: project_id.clone(),
            branch_id: None,
            title: "Camp routines".to_string(),
            source_type: "book".to_string(),
            url: None,
            file_path: None,
            author: None,
            publisher: None,
            published_date: None,
            accessed_at: None,
            reliability: "secondary".to_string(),
            tags: vec!["field-camp".to_string()],
            summary: None,
        })
        .await
        .unwrap()
        .source_id;

    let camp_note = svc
        .research_add_note(ResearchAddNoteInput {
            project_id: project_id.clone(),
            source_id: None,
            branch_id: None,
            note: "A dry ridge above the water.".to_string(),
            quote: None,
            locator: None,
            tags: vec!["camp".to_string()],
        })
        .await
        .unwrap()
        .note_id;

    let army_note = svc
        .research_add_note(ResearchAddNoteInput {
            project_id: project_id.clone(),
            source_id: None,
            branch_id: None,
            note: "A mounted patrol crossed at dawn.".to_string(),
            quote: None,
            locator: None,
            tags: vec!["army".to_string()],
        })
        .await
        .unwrap()
        .note_id;

    let supply_note = svc
        .research_add_note(ResearchAddNoteInput {
            project_id: project_id.clone(),
            source_id: None,
            branch_id: None,
            note: "Haulers moved barrels along the north road.".to_string(),
            quote: None,
            locator: None,
            tags: vec!["supply-lines".to_string()],
        })
        .await
        .unwrap()
        .note_id;

    let both_note = svc
        .research_add_note(ResearchAddNoteInput {
            project_id: project_id.clone(),
            source_id: None,
            branch_id: None,
            note: "The main column pitched tents beside the river.".to_string(),
            quote: None,
            locator: None,
            tags: vec!["army".to_string(), "camp".to_string()],
        })
        .await
        .unwrap()
        .note_id;

    let claim_id = svc
        .research_add_claim(ResearchAddClaimInput {
            project_id: project_id.clone(),
            source_id: None,
            note_id: None,
            branch_id: None,
            claim: "Camp layouts followed the ridge.".to_string(),
            topic: Some("camp".to_string()),
            time_period: None,
            location: None,
            confidence: "high".to_string(),
            tags: vec!["camp".to_string()],
        })
        .await
        .unwrap()
        .claim_id;

    let search = |query: &str, tags: Vec<String>| {
        let project_id = project_id.clone();
        let query = query.to_string();
        let svc = svc.clone();
        async move {
            svc.research_search(ResearchSearchInput {
                project_id,
                branch_id: None,
                query,
                tags,
                time_period: None,
                location: None,
                limit: None,
            })
            .await
            .unwrap()
        }
    };

    let multi_word = search("army camp", Vec::new()).await;
    let ids: Vec<&str> = multi_word
        .results
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    assert!(ids.contains(&camp_note.as_str()), "camp tag matches");
    assert!(ids.contains(&army_note.as_str()), "army tag matches");
    assert!(ids.contains(&both_note.as_str()), "both tags match");
    assert!(
        ids.contains(&source_id.as_str()),
        "source title matches camp"
    );
    assert!(ids.contains(&claim_id.as_str()), "claim text matches camp");
    assert!(
        !ids.contains(&supply_note.as_str()),
        "unrelated note must not match"
    );
    assert_eq!(
        multi_word.results[0].id, both_note,
        "the item matching both query terms ranks first"
    );

    let hyphen_query = search("supply lines", Vec::new()).await;
    assert_eq!(hyphen_query.results.len(), 1);
    assert_eq!(hyphen_query.results[0].id, supply_note);

    let space_query = search("supply-lines", Vec::new()).await;
    assert_eq!(space_query.results.len(), 1);
    assert_eq!(space_query.results[0].id, supply_note);

    let partial = search("army camp supply", Vec::new()).await;
    assert!(
        partial.results.iter().any(|item| item.id == supply_note),
        "partial matches are included"
    );
    assert_eq!(
        partial.results[0].id, both_note,
        "the two-term match still ranks above one-term matches"
    );

    let filtered = search("barrels", vec!["Supply_Lines".to_string()]).await;
    assert_eq!(filtered.results.len(), 1);
    assert_eq!(filtered.results[0].id, supply_note);
}
