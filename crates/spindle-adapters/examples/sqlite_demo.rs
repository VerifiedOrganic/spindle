//! Demonstrates the SQLite-backed Spindle stack end-to-end as a runnable
//! program. This is the Phase 6 deliverable that proves the SQLite migration
//! produces a functional spindle backend independent of the MCP layer swap.
//!
//! Run with:
//!   cargo run -p spindle-adapters --example sqlite_demo
//!
//! Walks through:
//!   1. Open a fresh SQLite DB in a temp directory.
//!   2. Create a project + main branch + book + chapter via the service.
//!   3. Create two characters with voice/emotional profiles.
//!   4. Create a relationship between them.
//!   5. Create a location with paired world_state.
//!   6. Plan chapter 1 with two scenes.
//!   7. Save scene 1 prose (triggers fts_scene + search_embedding sync).
//!   8. Commit a character_state snapshot.
//!   9. Search the Bible in both Semantic (vec0) and Exact (FTS5) modes.
//!  10. Branch off main and switch active branch.
//!  11. Print a summary of what landed.

use anyhow::Result;
use spindle_adapters::sqlite::{Repository, SqliteSpindleService};
use spindle_adapters::{ModelRouter, SqlitePool};
use spindle_core::models::{
    CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
    CommitCharacterStateInput, ContentRating, CreateBranchInput, CreateCharacterInput,
    CreateLocationInput, CreateProjectInput, CreateRelationshipInput, PlanChapterInput,
    PlanChapterSceneInput, ReaderContract, SaveSceneDraftInput, SearchBibleInput, SearchBibleMode,
    SwitchBranchInput, WorldStateInput,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== Spindle SQLite stack demo ===\n");

    // 1. Open a fresh SQLite DB.
    let tmp = tempfile::TempDir::new()?;
    let db_path = tmp.path().join("spindle_demo.db");
    println!("DB:   {}", db_path.display());
    let pool = SqlitePool::open(&db_path).await?;
    let data_dir: PathBuf = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir)?;
    let repo = Repository::with_model_router(pool, data_dir, ModelRouter::default());
    let svc = SqliteSpindleService::new(repo);

    // 2. Project.
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Demo Project".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "A warden holds a gate as a city falls.".into(),
                style_notes: vec!["sparse prose".into()],
                boundaries: Vec::new(),
            },
        })
        .await?;
    println!("\n[2] project = {}", proj.project_id);
    println!("    book    = {}", proj.book_id);
    println!("    chapter = {}", proj.chapter_id);

    // 3. Characters.
    let mara = svc
        .create_character(CreateCharacterInput {
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Oathbound warden of the Ash Gate.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: Some("grim".into()),
                vocabulary: vec!["oath".into(), "ash".into()],
                sentence_structure: vec!["clipped".into()],
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: vec!["I know the cost.".into()],
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: BTreeMap::new(),
                suppressed: vec!["fear".into()],
                triggers: Vec::new(),
                defense_mechanisms: vec!["silence".into()],
                flex_range: None,
            },
            initial_state: Some(CharacterStatePatch {
                emotional_state: BTreeMap::new(),
                goals: Some(vec!["hold the gate".into()]),
                status: Some(vec!["wary".into()]),
                notes: None,
                source_summary: Some("introduction".into()),
            }),
        })
        .await?;
    println!("\n[3] mara    = {}", mara.character_id);

    let aldric = svc
        .create_character(CreateCharacterInput {
            project_id: proj.project_id.clone(),
            name: "Aldric".into(),
            summary: "Scribe of the marches.".into(),
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
                base_emotions: BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await?;
    println!("    aldric  = {}", aldric.character_id);

    // 4. Relationship.
    let _rel = svc
        .create_relationship(CreateRelationshipInput {
            character_a_id: mara.character_id.clone(),
            character_b_id: aldric.character_id.clone(),
            relationship_type: "ally".into(),
            initial_trust: 60,
            initial_tension: 20,
            dynamics: vec!["wary mutual respect".into()],
        })
        .await?;
    println!("\n[4] relationship: mara <-> aldric (ally, trust=60, tension=20)");

    // 5. Location.
    let gate = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A blackened wall holding back the dark.".into(),
            initial_state: WorldStateInput {
                controlling_faction: None,
                status: Some("watchful".into()),
                prosperity: None,
                stability: Some("fragile".into()),
                threat_level: Some("high".into()),
                sensory_details: vec!["smell of ash".into()],
            },
        })
        .await?;
    println!("\n[5] location    = {}", gate.location_id);
    println!("    world_state = {}", gate.world_state_id);

    // 6. Chapter plan.
    let plan = svc
        .plan_chapter(PlanChapterInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            pov_character_id: Some(mara.character_id.clone()),
            synopsis: "Mara holds the gate against the first dark.".into(),
            target_theme_ids: Vec::new(),
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes: vec![PlanChapterSceneInput {
                scene_order: 1,
                summary: "Mara takes the watch".into(),
                beat_structure: vec!["arrival".into(), "first dark".into()],
                character_ids: vec![mara.character_id.clone()],
                purpose: "establishing".into(),
            }],
        })
        .await?;
    println!("\n[6] chapter_plan = {}", plan.chapter_plan_id);

    // 7. Save scene.
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate. Her oath weighed heavier than her sword."
                .into(),
            summary: "Mara holds the gate at first dark".into(),
            content_rating: ContentRating::General,
            tone: Some("grim".into()),
            generation_id: None,
            source_path: None,
        })
        .await?;
    println!("\n[7] scene = {} ({})", scene.scene_id, scene.status);

    // 8. Commit state.
    let state = svc
        .commit_character_state(CommitCharacterStateInput {
            character_id: mara.character_id.clone(),
            scene_id: scene.scene_id.clone(),
            changes: CharacterStatePatch {
                emotional_state: BTreeMap::new(),
                goals: Some(vec!["hold the gate at all costs".into()]),
                status: Some(vec!["determined".into()]),
                notes: None,
                source_summary: Some("first watch".into()),
            },
        })
        .await?;
    println!("\n[8] character_state = {}", state.state_id);

    // 9. Search both modes.
    let lexical = svc
        .search_bible(SearchBibleInput {
            project_id: proj.project_id.clone(),
            query: "warden".into(),
            limit: Some(5),
            mode: Some(SearchBibleMode::Exact),
            field: None,
            subject_table: None,
            format: None,
            budget_tokens: None,
        })
        .await?;
    println!(
        "\n[9] search 'warden' (Exact/FTS5) — {} hits",
        lexical.results.len()
    );
    for hit in &lexical.results {
        println!(
            "      {} {} score={:.3}",
            hit.entity_type, hit.entity_id, hit.score
        );
    }

    let semantic = svc
        .search_bible(SearchBibleInput {
            project_id: proj.project_id.clone(),
            query: "Mara stood at the gate".into(),
            limit: Some(5),
            mode: Some(SearchBibleMode::Semantic),
            field: None,
            subject_table: None,
            format: None,
            budget_tokens: None,
        })
        .await?;
    println!(
        "   search 'Mara stood at the gate' (Semantic/vec0) — {} hits",
        semantic.results.len()
    );
    for hit in &semantic.results {
        println!(
            "      {} {} score={:.3}",
            hit.entity_type, hit.entity_id, hit.score
        );
    }

    // 10. Branch off main.
    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            name: "feature-alt-ending".into(),
            branch_type: "feature".into(),
            description: Some("alternate fall-of-the-gate ending".into()),
            parent_branch_id: None,
        })
        .await?;
    let switched = svc
        .switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await?;
    println!(
        "\n[10] new branch  = {} ({})",
        switched.branch_id, switched.branch_name
    );

    // 11. Summary.
    let projects = svc.list_projects().await?;
    println!(
        "\n[11] projects in DB: {} ({})",
        projects.projects.len(),
        projects.projects[0].name
    );

    println!(
        "\n=== Demo complete. DB file (auto-cleaned): {} ===",
        db_path.display()
    );
    Ok(())
}
