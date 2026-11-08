use super::*;

impl SqliteSpindleService {
    /// Run the cumulative reader-simulation pass for a single chapter (evolution
    /// §3.6, P3.4). Concatenates the given scenes' prose in spine order, derives
    /// the reader persona from the project's reader contract, folds in the prior
    /// notes (the reader's rolling memory), dispatches through the
    /// `reader_sim` → `review` fallback ladder (§2.3, mirroring the mine ladder)
    /// gated at `rating`, and parses strict JSON out. Never mutates canon — this
    /// is enrichment, not a gate. Honest outcomes (I8): `skipped` on no cleared
    /// route / transport error / no prose (never fakes a clean read), `unparsed`
    /// on malformed output (prior notes preserved so the reader loses no memory),
    /// `read` otherwise.
    pub async fn reader_sim_chapter(
        &self,
        input: spindle_core::models::ReaderSimChapterInput,
    ) -> Result<spindle_core::models::ReaderSimChapterOutcome> {
        use crate::ai::{ModelRequest, RouteClearanceError};
        use spindle_core::models::ReaderSimChapterOutcome;

        let project = self.repository.get_project(&input.project_id).await?;

        // Gather the chapter's scenes IN SPINE ORDER. The caller supplies the
        // ordered scene ids; we fetch each and drop empty prose. No scenes / all
        // empty ⇒ honest skip before any model call (I8).
        let mut resolved_scenes = Vec::new();
        let mut prose_blocks: Vec<String> = Vec::new();
        for scene_id in &input.scene_ids {
            let scene = self.repository.get_scene(scene_id).await?;
            if scene.project_id != input.project_id {
                anyhow::bail!(
                    "scene {} does not belong to project {}",
                    scene_id,
                    input.project_id
                );
            }
            resolved_scenes.push(scene);
        }
        resolved_scenes.sort_by_key(|s| (s.book_number, s.chapter_number, s.scene_order));
        if let Some(first) = resolved_scenes.first() {
            anyhow::ensure!(
                resolved_scenes
                    .iter()
                    .all(|s| s.branch_id == first.branch_id
                        && s.book_number == first.book_number
                        && s.chapter_number == first.chapter_number),
                "reader simulation scenes must belong to one chapter and branch"
            );
        }
        let rating = max_scene_rating(&resolved_scenes)
            .into_iter()
            .chain(input.rating.clone())
            .max_by_key(|rating| match rating.to_ascii_lowercase().as_str() {
                "explicit" => 3,
                "mature" => 2,
                "teen" => 1,
                _ => 0,
            });
        for scene in &resolved_scenes {
            if !scene.full_text.trim().is_empty() {
                prose_blocks.push(format!(
                    "[chapter {} scene {}]\n{}",
                    scene.chapter_number, scene.scene_order, scene.full_text
                ));
            }
        }
        if prose_blocks.is_empty() {
            return Ok(ReaderSimChapterOutcome {
                open_questions: None,
                status: "skipped".to_string(),
                engagement: "skipped".to_string(),
                notes: String::new(),
                concerns: Vec::new(),
                skip_reason: Some("no prose".to_string()),
            });
        }

        let contract = project.reader_contract.clone().into_core();
        let prompt =
            build_reader_sim_prompt(&contract, &input.prior_notes, &prose_blocks.join("\n\n"));

        // Fallback ladder (§2.3): reader_sim → review (on NoRoute) → skip. The
        // chokepoint inside complete() gates the rating for the uncleared case,
        // so no prose leaves for an uncleared agent.
        let response = match self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "reader_sim".to_string(),
                prompt: prompt.clone(),
                rating: rating.clone(),
                context: Some(crate::ai::RequestContext {
                    project_id: Some(input.project_id.clone()),
                    scene_id: resolved_scenes.first().map(|s| s.id.clone()),
                    ..Default::default()
                }),
            })
            .await
        {
            Ok(response) => response,
            Err(err) => {
                let is_no_route = matches!(
                    err.downcast_ref::<RouteClearanceError>(),
                    Some(RouteClearanceError::NoRoute { .. })
                );
                if is_no_route {
                    // Retry via the review route (same rating).
                    match self
                        .repository
                        .model_router()
                        .complete(&ModelRequest {
                            route: "review".to_string(),
                            prompt: prompt.clone(),
                            rating: rating.clone(),
                            context: Some(crate::ai::RequestContext {
                                project_id: Some(input.project_id.clone()),
                                scene_id: resolved_scenes.first().map(|s| s.id.clone()),
                                ..Default::default()
                            }),
                        })
                        .await
                    {
                        Ok(response) => response,
                        Err(review_err) => {
                            return Ok(reader_sim_skip_outcome(&review_err, "review"));
                        }
                    }
                } else {
                    return Ok(reader_sim_skip_outcome(&err, "reader_sim"));
                }
            }
        };

        // Strict parse; malformed → keep prior notes, engagement "unparsed",
        // never guess (I8). The reader loses no memory.
        match (!response.truncated)
            .then(|| parse_reader_sim_output(&response.output))
            .flatten()
        {
            Some(parsed) => Ok(ReaderSimChapterOutcome {
                open_questions: parsed.open_questions,
                status: "read".to_string(),
                engagement: parsed.engagement,
                notes: parsed.notes,
                concerns: parsed.concerns,
                skip_reason: None,
            }),
            None => Ok(ReaderSimChapterOutcome {
                open_questions: None,
                status: "unparsed".to_string(),
                engagement: "unparsed".to_string(),
                notes: input.prior_notes.clone(),
                concerns: Vec::new(),
                skip_reason: None,
            }),
        }
    }
}
