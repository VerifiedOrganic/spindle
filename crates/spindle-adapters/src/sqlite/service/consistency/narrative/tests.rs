use super::*;

#[tokio::test]
async fn motif_usage_audit_flags_overuse_beyond_limit() {
    use spindle_core::models::{
        AnnotateSceneBeatsInput, CheckConsistencyInput, ConsistencyScopeInput, ContentRating,
        CreateMotifInput, SaveSceneDraftInput,
    };

    let (_tmp, svc, project_id, scene1) = motif_theme_fixture_scene(1).await;
    // A second scene in the SAME chapter so two annotation links land in
    // chapter 1.
    let scene2 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
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
    let motif = svc
        .create_motif(CreateMotifInput {
            project_id: project_id.clone(),
            name: "raven".into(),
            description: "a black bird".into(),
            max_uses_per_chapter: Some(1),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();
    for sid in [&scene1, &scene2.scene_id] {
        svc.annotate_scene_beats(AnnotateSceneBeatsInput {
            project_id: project_id.clone(),
            scene_id: sid.clone(),
            beats: Vec::new(),
            motif_ids: vec![motif.motif_id.clone()],
            theme_ids: Vec::new(),
            conflict_ids: Vec::new(),
            intensity: None,
        })
        .await
        .unwrap();
    }

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["motif_usage_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    let hits: Vec<_> = out
        .issues
        .iter()
        .filter(|i| i.check_type == "motif_usage_audit" && i.severity == "warning")
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "2 links with limit 1 should warn once: {:?}",
        out.issues
    );
    assert!(hits[0].entity_ids.contains(&motif.motif_id));
}

#[tokio::test]
async fn motif_usage_audit_at_limit_is_clean() {
    use spindle_core::models::{
        AnnotateSceneBeatsInput, CheckConsistencyInput, ConsistencyScopeInput, CreateMotifInput,
    };

    let (_tmp, svc, project_id, scene1) = motif_theme_fixture_scene(1).await;
    let motif = svc
        .create_motif(CreateMotifInput {
            project_id: project_id.clone(),
            name: "raven".into(),
            description: "a black bird".into(),
            max_uses_per_chapter: Some(1),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();
    // Exactly one link in chapter 1 == the limit; boundary must be clean.
    svc.annotate_scene_beats(AnnotateSceneBeatsInput {
        project_id: project_id.clone(),
        scene_id: scene1.clone(),
        beats: Vec::new(),
        motif_ids: vec![motif.motif_id.clone()],
        theme_ids: Vec::new(),
        conflict_ids: Vec::new(),
        intensity: None,
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            // Two-chapter scope: below the ≥3 threshold, so no unused-info
            // noise either.
            scope: ConsistencyScopeInput::chapter_range(1, 1, 1, 2),
            checks: vec!["motif_usage_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "motif_usage_audit"),
        "at-limit usage must not fire: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn motif_usage_audit_null_limit_is_silent() {
    use spindle_core::models::{
        AnnotateSceneBeatsInput, CheckConsistencyInput, ConsistencyScopeInput, ContentRating,
        CreateMotifInput, SaveSceneDraftInput,
    };

    let (_tmp, svc, project_id, scene1) = motif_theme_fixture_scene(1).await;
    let scene2 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
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
    let motif = svc
        .create_motif(CreateMotifInput {
            project_id: project_id.clone(),
            name: "raven".into(),
            description: "a black bird".into(),
            max_uses_per_chapter: None,
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();
    // Two links, but no limit declared: overuse arm must stay silent.
    for sid in [&scene1, &scene2.scene_id] {
        svc.annotate_scene_beats(AnnotateSceneBeatsInput {
            project_id: project_id.clone(),
            scene_id: sid.clone(),
            beats: Vec::new(),
            motif_ids: vec![motif.motif_id.clone()],
            theme_ids: Vec::new(),
            conflict_ids: Vec::new(),
            intensity: None,
        })
        .await
        .unwrap();
    }

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["motif_usage_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "motif_usage_audit"),
        "None limit must be silent for overuse: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn motif_usage_audit_unused_info_only_on_wide_scope() {
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateChapterInput,
        CreateMotifInput, SaveSceneDraftInput,
    };

    let (_tmp, svc, project_id, _scene1) = motif_theme_fixture_scene(1).await;
    // Add chapters 2 and 3 with scenes so the full scope covers ≥3 chapters.
    for ch in 2..=3 {
        svc.create_chapter(CreateChapterInput {
            project_id: project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(ch),
            title: None,
        })
        .await
        .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: ch,
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
    }
    // A motif never linked by any beat annotation anywhere.
    let motif = svc
        .create_motif(CreateMotifInput {
            project_id: project_id.clone(),
            name: "raven".into(),
            description: "a black bird".into(),
            max_uses_per_chapter: Some(2),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();

    // Wide scope (3 chapters) → info "declared but unused".
    let wide = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["motif_usage_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        wide.issues.iter().any(|i| {
            i.check_type == "motif_usage_audit"
                && i.severity == "info"
                && i.entity_ids.contains(&motif.motif_id)
        }),
        "unused motif over a ≥3-chapter scope should raise info: {:?}",
        wide.issues
    );

    // Narrow scope (2 chapters) → no unused-info noise.
    let narrow = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::chapter_range(1, 1, 1, 2),
            checks: vec!["motif_usage_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        !narrow
            .issues
            .iter()
            .any(|i| i.check_type == "motif_usage_audit"),
        "unused motif on a <3-chapter scope must stay silent: {:?}",
        narrow.issues
    );
}

#[tokio::test]
async fn theme_placement_audit_flags_unlinked_introduction() {
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, CreateThemeInput, StoryPlacement,
    };

    let (_tmp, svc, project_id, _scene1) = motif_theme_fixture_scene(1).await;
    // Theme introduced in chapter 1 (in scope) but no beat annotation links
    // it there.
    let theme = svc
        .create_theme(CreateThemeInput {
            project_id: project_id.clone(),
            theme_statement: "power corrupts".into(),
            thesis_antithesis: "order vs freedom".into(),
            introduction_point: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: None,
                note: None,
            }),
            resolution_point: None,
        })
        .await
        .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["theme_placement_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        out.issues.iter().any(|i| {
            i.check_type == "theme_placement_audit"
                && i.severity == "info"
                && i.entity_ids.contains(&theme.theme_id)
        }),
        "introduction chapter with no theme link should raise info: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn theme_placement_audit_linked_introduction_is_clean() {
    use spindle_core::models::{
        AnnotateSceneBeatsInput, CheckConsistencyInput, ConsistencyScopeInput, CreateThemeInput,
        StoryPlacement,
    };

    let (_tmp, svc, project_id, scene1) = motif_theme_fixture_scene(1).await;
    let theme = svc
        .create_theme(CreateThemeInput {
            project_id: project_id.clone(),
            theme_statement: "power corrupts".into(),
            thesis_antithesis: "order vs freedom".into(),
            introduction_point: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: None,
                note: None,
            }),
            resolution_point: None,
        })
        .await
        .unwrap();
    // Link the theme in chapter 1 → placement is honored, no finding.
    svc.annotate_scene_beats(AnnotateSceneBeatsInput {
        project_id: project_id.clone(),
        scene_id: scene1.clone(),
        beats: Vec::new(),
        motif_ids: Vec::new(),
        theme_ids: vec![theme.theme_id.clone()],
        conflict_ids: Vec::new(),
        intensity: None,
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["theme_placement_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "theme_placement_audit"),
        "linked introduction must be clean: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn theme_placement_audit_null_placement_is_silent() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput, CreateThemeInput};

    let (_tmp, svc, project_id, _scene1) = motif_theme_fixture_scene(1).await;
    // No introduction_point / resolution_point → silent.
    svc.create_theme(CreateThemeInput {
        project_id: project_id.clone(),
        theme_statement: "power corrupts".into(),
        thesis_antithesis: "order vs freedom".into(),
        introduction_point: None,
        resolution_point: None,
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["theme_placement_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "theme_placement_audit"),
        "null placement must be silent: {:?}",
        out.issues
    );
}

// ── plot_line_convergence_audit ──────────────────────────────────────

/// Convergence chapter is in scope, connected conflict declared, but no
/// beat annotation in that chapter links it → info naming line + chapter.
#[tokio::test]
async fn plot_line_convergence_audit_fires_when_unlinked() {
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, CreateConflictInput, CreatePlotLineInput,
    };

    let (_tmp, svc, project_id, _scene, _char) = convergence_fixture_scene(1).await;
    let conflict = svc
        .create_conflict(CreateConflictInput {
            project_id: project_id.clone(),
            name: "the siege".into(),
            conflict_type: "external".into(),
            stakes: "the city".into(),
            escalation_stages: Vec::new(),
            expected_total_cycles: None,
            try_fail_cycles: Vec::new(),
            stated_consequences: Vec::new(),
        })
        .await
        .unwrap();
    let plot = svc
        .create_plot_line(CreatePlotLineInput {
            project_id: project_id.clone(),
            name: "main line".into(),
            plot_type: "main".into(),
            summary: "s".into(),
            status: None,
            convergence_points: vec![placement(1, 1)],
            connected_conflict_ids: vec![conflict.conflict_id.clone()],
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["plot_line_convergence_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    let hits: Vec<_> = out
        .issues
        .iter()
        .filter(|i| i.check_type == "plot_line_convergence_audit")
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "one unlinked convergence should fire: {:?}",
        out.issues
    );
    assert_eq!(hits[0].severity, "info");
    assert!(hits[0].entity_ids.contains(&plot.plot_line_id));
}

/// A beat annotation in the convergence chapter links the connected conflict
/// → no finding.
#[tokio::test]
async fn plot_line_convergence_audit_linked_is_clean() {
    use spindle_core::models::{
        AnnotateSceneBeatsInput, CheckConsistencyInput, ConsistencyScopeInput, CreateConflictInput,
        CreatePlotLineInput,
    };

    let (_tmp, svc, project_id, scene, _char) = convergence_fixture_scene(1).await;
    let conflict = svc
        .create_conflict(CreateConflictInput {
            project_id: project_id.clone(),
            name: "the siege".into(),
            conflict_type: "external".into(),
            stakes: "the city".into(),
            escalation_stages: Vec::new(),
            expected_total_cycles: None,
            try_fail_cycles: Vec::new(),
            stated_consequences: Vec::new(),
        })
        .await
        .unwrap();
    svc.create_plot_line(CreatePlotLineInput {
        project_id: project_id.clone(),
        name: "main line".into(),
        plot_type: "main".into(),
        summary: "s".into(),
        status: None,
        convergence_points: vec![placement(1, 1)],
        connected_conflict_ids: vec![conflict.conflict_id.clone()],
        connected_theme_ids: Vec::new(),
    })
    .await
    .unwrap();
    svc.annotate_scene_beats(AnnotateSceneBeatsInput {
        project_id: project_id.clone(),
        scene_id: scene,
        beats: Vec::new(),
        motif_ids: Vec::new(),
        theme_ids: Vec::new(),
        conflict_ids: vec![conflict.conflict_id.clone()],
        intensity: None,
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["plot_line_convergence_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "plot_line_convergence_audit"),
        "linked convergence must be clean: {:?}",
        out.issues
    );
}

/// A plot line that declares no connected conflicts/themes is silent even if
/// its convergence chapter is unannotated.
#[tokio::test]
async fn plot_line_convergence_audit_no_connections_is_silent() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput, CreatePlotLineInput};

    let (_tmp, svc, project_id, _scene, _char) = convergence_fixture_scene(1).await;
    svc.create_plot_line(CreatePlotLineInput {
        project_id: project_id.clone(),
        name: "main line".into(),
        plot_type: "main".into(),
        summary: "s".into(),
        status: None,
        convergence_points: vec![placement(1, 1)],
        connected_conflict_ids: Vec::new(),
        connected_theme_ids: Vec::new(),
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["plot_line_convergence_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "plot_line_convergence_audit"),
        "no declared connections must be silent: {:?}",
        out.issues
    );
}

/// Milestone placed at chapter P, not reached, and the manuscript already
/// runs past P → warning with the overdue chapter count.
#[tokio::test]
async fn arc_milestone_audit_overdue_warns() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};

    let (_tmp, svc, project_id, _scene, character_id) = convergence_fixture_scene(1).await;
    // Scenes exist through chapter 4; milestone due at chapter 2.
    for ch in [2, 3, 4] {
        seed_scene_at(&svc, &project_id, ch).await;
    }
    let arc_id = seed_arc_with_milestone(
        &svc,
        &project_id,
        &character_id,
        Some(placement(1, 2)),
        None,
    )
    .await;

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["arc_milestone_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    let hits: Vec<_> = out
        .issues
        .iter()
        .filter(|i| i.check_type == "arc_milestone_audit")
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "one overdue milestone should warn: {:?}",
        out.issues
    );
    assert_eq!(hits[0].severity, "warning");
    assert!(hits[0].entity_ids.contains(&arc_id));
    assert!(
        hits[0].message.contains("the reckoning"),
        "message should name the milestone: {}",
        hits[0].message
    );
    assert!(
        hits[0].message.contains('2'),
        "overdue by max(4) - P(2) = 2: {}",
        hits[0].message
    );
}

/// Milestone due at the current max chapter (== P), not reached → info
/// "due now".
#[tokio::test]
async fn arc_milestone_audit_due_now_info() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};

    let (_tmp, svc, project_id, _scene, character_id) = convergence_fixture_scene(1).await;
    seed_scene_at(&svc, &project_id, 2).await; // max scoped chapter = 2
    let arc_id = seed_arc_with_milestone(
        &svc,
        &project_id,
        &character_id,
        Some(placement(1, 2)),
        None,
    )
    .await;

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["arc_milestone_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    let hits: Vec<_> = out
        .issues
        .iter()
        .filter(|i| i.check_type == "arc_milestone_audit")
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "due-now should surface one info: {:?}",
        out.issues
    );
    assert_eq!(hits[0].severity, "info");
    assert!(hits[0].entity_ids.contains(&arc_id));
}

/// Milestone with a `reached_at` marker is silent regardless of chapters.
#[tokio::test]
async fn arc_milestone_audit_reached_is_silent() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};

    let (_tmp, svc, project_id, _scene, character_id) = convergence_fixture_scene(1).await;
    for ch in [2, 3, 4] {
        seed_scene_at(&svc, &project_id, ch).await;
    }
    seed_arc_with_milestone(
        &svc,
        &project_id,
        &character_id,
        Some(placement(1, 2)),
        Some(placement(1, 3)),
    )
    .await;

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["arc_milestone_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "arc_milestone_audit"),
        "reached milestone must be silent: {:?}",
        out.issues
    );
}

/// Milestone with no placement is silent.
#[tokio::test]
async fn arc_milestone_audit_null_placement_is_silent() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};

    let (_tmp, svc, project_id, _scene, character_id) = convergence_fixture_scene(1).await;
    for ch in [2, 3, 4] {
        seed_scene_at(&svc, &project_id, ch).await;
    }
    seed_arc_with_milestone(&svc, &project_id, &character_id, None, None).await;

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["arc_milestone_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "arc_milestone_audit"),
        "null placement must be silent: {:?}",
        out.issues
    );
}

/// Stage 2 (index 1) demonstrated while stage 1 (index 0) is not → warning.
#[tokio::test]
async fn conflict_escalation_audit_out_of_order_warns() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};

    let (_tmp, svc, project_id, _scene, _char) = convergence_fixture_scene(1).await;
    let conflict_id = seed_conflict_with_markers(
        &svc,
        &project_id,
        serde_json::json!([
            null,
            {"book_number": 1, "chapter_number": 2, "scene_order": null, "note": null}
        ]),
    )
    .await;

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["conflict_escalation_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    let hits: Vec<_> = out
        .issues
        .iter()
        .filter(|i| i.check_type == "conflict_escalation_audit")
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "out-of-order should warn once: {:?}",
        out.issues
    );
    assert_eq!(hits[0].severity, "warning");
    assert!(hits[0].entity_ids.contains(&conflict_id));
    assert!(
        hits[0].message.contains("tension") && hits[0].message.contains("clash"),
        "message should name the out-of-order pair labels: {}",
        hits[0].message
    );
}

/// Stages demonstrated in order (1 then 2) → silent.
#[tokio::test]
async fn conflict_escalation_audit_in_order_is_clean() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};

    let (_tmp, svc, project_id, _scene, _char) = convergence_fixture_scene(1).await;
    seed_conflict_with_markers(
        &svc,
        &project_id,
        serde_json::json!([
            {"book_number": 1, "chapter_number": 1, "scene_order": null, "note": null},
            {"book_number": 1, "chapter_number": 2, "scene_order": null, "note": null}
        ]),
    )
    .await;

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["conflict_escalation_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "conflict_escalation_audit"),
        "in-order demonstration must be clean: {:?}",
        out.issues
    );
}

/// No demonstration markers at all → silent.
#[tokio::test]
async fn conflict_escalation_audit_no_markers_is_silent() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};

    let (_tmp, svc, project_id, _scene, _char) = convergence_fixture_scene(1).await;
    seed_conflict_with_markers(&svc, &project_id, serde_json::Value::Null).await;

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["conflict_escalation_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "conflict_escalation_audit"),
        "no markers must be silent: {:?}",
        out.issues
    );
}

/// Positive: a scoped scene carrying the payoff sentinel yields exactly one
/// info finding naming the promise and the `update_promise_status` follow-up.
#[tokio::test]
async fn deep_check_surfaces_promise_payoff_candidate() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project_id, promise_id) = seed_promise_payoff_project(
        &svc,
        "promise-payoff",
        "At last the reclaimed crown settled on her brow, and the hall knelt. \
             MOCK_PROMISE_PAYOFF. The old debt was paid in full.",
    )
    .await;

    let deep = svc
        .check_consistency(promise_payoff_input(&project_id, true))
        .await
        .unwrap();

    let findings: Vec<_> = deep
        .issues
        .iter()
        .filter(|i| i.check_type == "promise_payoff_detection")
        .collect();
    assert_eq!(
        findings.len(),
        1,
        "exactly one payoff candidate expected: {:?}",
        deep.issues
    );
    let finding = findings[0];
    assert_eq!(finding.severity, "info");
    assert!(
        finding.message.contains(&promise_id),
        "message must name the promise id: {}",
        finding.message
    );
    assert!(
        finding.message.contains("update_promise_status"),
        "message must point at the write tool: {}",
        finding.message
    );
}

/// Shallow-silent: the same setup without `deep_check` yields nothing.
#[tokio::test]
async fn shallow_check_has_no_promise_payoff_finding() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project_id, _promise_id) = seed_promise_payoff_project(
        &svc,
        "promise-payoff-shallow",
        "At last the reclaimed crown settled on her brow. MOCK_PROMISE_PAYOFF.",
    )
    .await;

    let shallow = svc
        .check_consistency(promise_payoff_input(&project_id, false))
        .await
        .unwrap();
    assert!(
        !shallow
            .issues
            .iter()
            .any(|i| i.check_type == "promise_payoff_detection"),
        "the deep-only check must be silent without deep_check: {:?}",
        shallow.issues
    );
}

/// Cap + no silent cap: 12 unresolved promises, at most 10 scanned, and a
/// single summary info finding reports the 2 that were not scanned. Prose
/// carries no sentinel, so the model returns empty for the scanned promises.
#[tokio::test]
async fn deep_check_caps_promise_scan_and_reports_unscanned() {
    use spindle_core::models::{CreateNarrativePromiseInput, StoryPlacement};
    let (_tmp, svc) = fresh_service_local().await;
    // One scoped scene, no sentinel -> model returns empty findings.
    let (project_id, _first) =
        seed_promise_payoff_project(&svc, "promise-payoff-cap", "A quiet, unresolved scene.").await;

    // The seed already created promise #1; add 11 more for 12 total.
    for i in 0..11 {
        svc.create_narrative_promise(CreateNarrativePromiseInput {
            project_id: project_id.clone(),
            promise_type: "setup".into(),
            description: format!("dangling thread {i}"),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(0),
                note: None,
            },
            planned_payoff: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 2,
                scene_order: Some(0),
                note: None,
            }),
            notes: Vec::new(),
        })
        .await
        .unwrap();
    }

    let deep = svc
        .check_consistency(promise_payoff_input(&project_id, true))
        .await
        .unwrap();

    let summary = deep
        .issues
        .iter()
        .find(|i| {
            i.check_type == "promise_payoff_detection"
                && i.severity == "info"
                && i.message.contains("not scanned")
        })
        .expect("a summary finding must report the unscanned overflow");
    assert!(
        summary.message.contains('2'),
        "12 candidates - 10 scanned = 2 unscanned: {}",
        summary.message
    );
}

/// No-writes: the deep run must not mutate any promise row.
#[tokio::test]
async fn deep_check_does_not_write_promise_rows() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project_id, promise_id) = seed_promise_payoff_project(
        &svc,
        "promise-payoff-nowrite",
        "At last the reclaimed crown settled on her brow. MOCK_PROMISE_PAYOFF.",
    )
    .await;

    let before = svc
        .repository
        .get_narrative_promise(&promise_id)
        .await
        .unwrap();
    svc.check_consistency(promise_payoff_input(&project_id, true))
        .await
        .unwrap();
    let after = svc
        .repository
        .get_narrative_promise(&promise_id)
        .await
        .unwrap();

    assert_eq!(before.status, after.status, "status must be untouched");
    assert_eq!(
        before.updated_at, after.updated_at,
        "updated_at must be byte-identical"
    );
}

/// Resolved-promise exclusion: a paid_off promise with sentinel prose is
/// never proposed.
#[tokio::test]
async fn deep_check_excludes_resolved_promise() {
    use spindle_core::models::UpdatePromiseStatusInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project_id, promise_id) = seed_promise_payoff_project(
        &svc,
        "promise-payoff-resolved",
        "At last the reclaimed crown settled on her brow. MOCK_PROMISE_PAYOFF.",
    )
    .await;
    svc.update_promise_status(UpdatePromiseStatusInput {
        narrative_promise_id: promise_id.clone(),
        status: "paid_off".into(),
        note: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let deep = svc
        .check_consistency(promise_payoff_input(&project_id, true))
        .await
        .unwrap();
    assert!(
        !deep
            .issues
            .iter()
            .any(|i| i.check_type == "promise_payoff_detection"),
        "a resolved promise must never be proposed: {:?}",
        deep.issues
    );
}

#[test]
fn promise_payoff_parse_extracts_matches() {
    let out = r#"{"matches":[{"paid_off":true,"scene_ref":"scene 1","evidence":"e"}]}"#;
    let matches = super::parse_deep_promise_payoff_output(out).unwrap();
    assert_eq!(matches.len(), 1);
    assert!(matches[0].paid_off);
    assert_eq!(matches[0].scene_ref.as_deref(), Some("scene 1"));
}

#[test]
fn promise_payoff_parse_handles_code_fence_and_prose() {
    let out = "Here is the audit:\n```json\n{\"matches\":[{\"paid_off\":true}]}\n```";
    let matches = super::parse_deep_promise_payoff_output(out).unwrap();
    assert_eq!(matches.len(), 1);
    assert!(matches[0].paid_off);
}

#[test]
fn promise_payoff_parse_empty_matches() {
    let matches = super::parse_deep_promise_payoff_output(r#"{"matches":[]}"#).unwrap();
    assert!(matches.is_empty());
}

#[test]
fn promise_payoff_parse_rejects_non_json() {
    // The default local `review` stub returns prose, not JSON; the caller
    // turns this Err into no matches (never a guess).
    assert!(
        super::parse_deep_promise_payoff_output(
            "Literary critic and craft technician both reviewed: ..."
        )
        .is_err()
    );
}

#[test]
fn payoff_skip_issue_names_route_and_rating_on_rating_not_covered() {
    let err = anyhow::Error::new(crate::ai::RouteClearanceError::RatingNotCovered {
        route: "review".to_string(),
        rating: "explicit".to_string(),
        agent_id: "reviewer".to_string(),
    });
    let issue = super::payoff_skip_issue(&err);
    assert_eq!(issue.check_type, "promise_payoff_detection");
    assert_eq!(issue.severity, "info");
    assert!(issue.message.contains("SKIPPED"));
    assert!(issue.message.contains("`review`"));
    assert!(issue.message.contains("`explicit`"));
    // Never any prose — only ids/route/rating tokens.
    assert!(issue.message.contains("not cleared for rating"));
}

#[test]
fn payoff_skip_issue_generic_message_for_non_clearance_error() {
    let err = anyhow::anyhow!("connection refused");
    let issue = super::payoff_skip_issue(&err);
    assert!(issue.message.contains("SKIPPED"));
    assert!(issue.message.contains("no usable review route"));
}
