use super::*;

/// Exercises every plain narrative-entity create (+ matching get) against
/// a fresh project. Catches column-order drift between INSERT param order
/// and the `*_COLUMNS` constant used by the read path.
#[tokio::test]
async fn narrative_entity_creates_round_trip() {
    use spindle_core::models::{
        CreateConflictInput, CreateEconomyInput, CreateFactionInput, CreateLocationInput,
        CreateMotifInput, CreateNarrativePromiseInput, CreatePlotLineInput, CreateReligionInput,
        CreateTermInput, CreateThemeInput, StoryPlacement, WorldStateInput,
    };
    let (_tmp, repo) = fresh_repo().await;
    let input = CreateProjectInput {
        name: "P".into(),
        project_type: "novel".into(),
        genre: "fantasy".into(),
        reader_contract: ReaderContract {
            promise: "p".into(),
            style_notes: Vec::new(),
            boundaries: Vec::new(),
        },
    };
    let (project, _branch, _book, _chapter) = repo.create_project(&input).await.unwrap();

    let (loc, ws) = repo
        .create_location(&CreateLocationInput {
            project_id: project.id.clone(),
            name: "Ash Gate".into(),
            kind: "city".into(),
            realm: Some("Northern Marches".into()),
            summary: "A blackened wall.".into(),
            initial_state: WorldStateInput {
                controlling_faction: Some("Wardens".into()),
                status: Some("watchful".into()),
                prosperity: Some("poor".into()),
                stability: Some("tense".into()),
                threat_level: Some("high".into()),
                sensory_details: vec!["smell of ash".into()],
            },
        })
        .await
        .unwrap();
    assert!(loc.id.starts_with("location:"));
    assert_eq!(loc.normalized_name, "ash gate");
    assert_eq!(ws.location_id, loc.id);

    let faction = repo
        .create_faction(&CreateFactionInput {
            project_id: project.id.clone(),
            name: "Wardens".into(),
            faction_type: "military".into(),
            realm: None,
            summary: "Oathbound.".into(),
            tags: vec!["oath".into(), "fire".into()],
        })
        .await
        .unwrap();
    assert_eq!(faction.tags, vec!["oath".to_string(), "fire".to_string()]);

    let religion = repo
        .create_religion(&CreateReligionInput {
            project_id: project.id.clone(),
            name: "Ash Pact".into(),
            deity_or_principle: "Cinders".into(),
            summary: "An oath rite.".into(),
            tags: vec!["oath".into()],
        })
        .await
        .unwrap();
    assert_eq!(religion.deity_or_principle, "Cinders");

    let economy = repo
        .create_economy(&CreateEconomyInput {
            project_id: project.id.clone(),
            name: "Marches".into(),
            realm: None,
            summary: "Trade dies at the gate.".into(),
            scarce_resources: vec!["salt".into()],
            trade_goods: vec!["pelts".into()],
            currency: Some("iron coin".into()),
            notes: vec!["barter common".into()],
        })
        .await
        .unwrap();
    assert_eq!(economy.scarce_resources, vec!["salt".to_string()]);

    let term = repo
        .create_term(&CreateTermInput {
            project_id: project.id.clone(),
            term_text: "Ashbound".into(),
            pronunciation: Some("ASH-bownd".into()),
            definition: "Oathsworn to the Ash Gate.".into(),
            usage_context: None,
            origin: None,
        })
        .await
        .unwrap();
    assert_eq!(term.normalized_term, "ashbound");

    let plot = repo
        .create_plot_line(&CreatePlotLineInput {
            project_id: project.id.clone(),
            name: "Gate Breach".into(),
            plot_type: "main".into(),
            summary: "The wardens fail.".into(),
            status: Some("rising".into()),
            convergence_points: vec![StoryPlacement {
                book_number: 1,
                chapter_number: 3,
                scene_order: Some(2),
                note: None,
            }],
            connected_conflict_ids: Vec::new(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(plot.status, "rising");
    assert_eq!(plot.convergence_points.len(), 1);

    let conflict = repo
        .create_conflict(&CreateConflictInput {
            project_id: project.id.clone(),
            name: "Oath vs Mercy".into(),
            conflict_type: "internal".into(),
            stakes: "the city".into(),
            escalation_stages: vec!["doubt".into(), "fracture".into()],
            expected_total_cycles: Some(3),
            try_fail_cycles: Vec::new(),
            stated_consequences: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(conflict.escalation_stages.len(), 2);

    let theme = repo
        .create_theme(&CreateThemeInput {
            project_id: project.id.clone(),
            theme_statement: "Mercy costs.".into(),
            thesis_antithesis: "Oath / Mercy".into(),
            introduction_point: None,
            resolution_point: None,
        })
        .await
        .unwrap();
    assert_eq!(theme.theme_statement, "Mercy costs.");

    let motif = repo
        .create_motif(&CreateMotifInput {
            project_id: project.id.clone(),
            name: "Ash on lips".into(),
            description: "Recurs at oath moments.".into(),
            max_uses_per_chapter: Some(2),
            connected_theme_ids: vec![theme.id.clone()],
        })
        .await
        .unwrap();
    assert_eq!(motif.connected_theme_ids, vec![theme.id]);

    let promise = repo
        .create_narrative_promise(&CreateNarrativePromiseInput {
            project_id: project.id.clone(),
            promise_type: "payoff".into(),
            description: "The gate will fall.".into(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(0),
                note: None,
            },
            planned_payoff: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 12,
                scene_order: None,
                note: Some("climax".into()),
            }),
            notes: vec!["watch for ash imagery".into()],
        })
        .await
        .unwrap();
    assert_eq!(promise.status, "planted");
    assert_eq!(promise.planned_payoff.as_ref().unwrap().chapter_number, 12);
}
