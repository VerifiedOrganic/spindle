use super::*;

#[tokio::test]
async fn dual_persona_upsert_does_not_clobber_a_newer_fingerprint() {
    use spindle_core::models::{
        ContentRating, DualPersonaReviewRound, PersonaReviewNotes, SaveSceneDraftInput,
    };

    let round = |n: usize| DualPersonaReviewRound {
        round: n,
        literary_critic: PersonaReviewNotes {
            persona: "literary".into(),
            strengths: vec!["s".into()],
            concerns: Vec::new(),
        },
        craft_technician: PersonaReviewNotes {
            persona: "craft".into(),
            strengths: Vec::new(),
            concerns: Vec::new(),
        },
        genre_reader: PersonaReviewNotes::default(),
        priority_actions: Vec::new(),
    };

    let (_tmp, repo, project, branch, scene_id) = repo_with_scene().await;
    let scene_a = repo.get_scene(&scene_id).await.unwrap();
    let fp_a = Repository::scene_revision_fingerprint(&scene_a);
    repo.upsert_dual_persona_review(UpsertDualPersonaReviewParams {
        project_id: project.id.clone(),
        branch_id: branch.id.clone(),
        scene_id: scene_id.clone(),
        rounds_completed: 1,
        review_rounds: vec![round(1)],
        scene_revision_fingerprint: fp_a.clone(),
        status: "in_progress".into(),
    })
    .await
    .unwrap();

    repo.save_scene_draft(
        &project.id,
        &branch.id,
        &SaveSceneDraftInput {
            project_id: project.id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "She turned back and said the line differently.".into(),
            summary: "turn".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let scene_b = repo.get_scene(&scene_id).await.unwrap();
    let fp_b = Repository::scene_revision_fingerprint(&scene_b);
    assert_ne!(fp_a, fp_b, "an edit must change the review fingerprint");

    repo.upsert_dual_persona_review(UpsertDualPersonaReviewParams {
        project_id: project.id.clone(),
        branch_id: branch.id.clone(),
        scene_id: scene_id.clone(),
        rounds_completed: 1,
        review_rounds: vec![round(1)],
        scene_revision_fingerprint: fp_b.clone(),
        status: "in_progress".into(),
    })
    .await
    .unwrap();

    let after_stale = repo
        .upsert_dual_persona_review(UpsertDualPersonaReviewParams {
            project_id: project.id.clone(),
            branch_id: branch.id.clone(),
            scene_id: scene_id.clone(),
            rounds_completed: 2,
            review_rounds: vec![round(1), round(2)],
            scene_revision_fingerprint: fp_a,
            status: "current".into(),
        })
        .await
        .unwrap();
    assert_eq!(after_stale.scene_revision_fingerprint, fp_b);
    assert_eq!(after_stale.rounds_completed, 1);
}
