use super::*;

impl SqliteSpindleService {
    /// Two-persona scene review (literary critic + craft technician),
    /// optionally with 1–3 rounds. Each round calls the model router
    /// twice through the "review" route. Local-adapter responses are
    /// replaced with deterministic heuristic concerns derived from the
    /// scene metadata so unit tests work without a real model. Persists
    /// to `dual_persona_review` via the existing repository upsert.
    /// Run (or join) a dual-persona review for a scene.
    ///
    /// # Why this is a detached job
    ///
    /// Each round dispatches one model call per persona, and a reasoning model
    /// can spend 30–60s on a single call. A two-round review is therefore
    /// minutes of wall time — well past an MCP client's request timeout. The
    /// original implementation ran every round inline and persisted ONCE at the
    /// end, so a client timeout threw away the whole run: the table was left
    /// with zero rows despite every model call having been paid for, and the
    /// retry started from scratch.
    ///
    /// So the work is detached from the request. This call starts (or joins) a
    /// background job, waits [`REVIEW_JOB_JOIN_WAIT`], and then either returns
    /// the finished review or reports `status: "in_progress"` with the rounds
    /// banked so far. The job keeps running and persists EACH round as it
    /// lands, so progress survives both the client giving up and this call
    /// returning. A retry joins the same job (never a duplicate run) and
    /// collects the result once it is done.
    pub async fn run_dual_persona_review(
        &self,
        input: spindle_core::models::RunDualPersonaReviewInput,
    ) -> Result<spindle_core::models::RunDualPersonaReviewOutput> {
        use spindle_core::models::RunDualPersonaReviewOutput;

        let branch = match input.branch_id.as_deref() {
            Some(id) => self.repository.get_branch(id).await?,
            None => self.repository.get_active_branch(&input.project_id).await?,
        };
        let branch_id = branch.id.clone();

        if branch
            .project_id
            .as_deref()
            .is_some_and(|id| id != input.project_id)
        {
            anyhow::bail!(
                "review branch {branch_id} does not belong to project {}",
                input.project_id
            );
        }
        let scene = self.repository.get_scene(&input.scene_id).await?;
        if scene.project_id != input.project_id || scene.branch_id != branch_id {
            anyhow::bail!(
                "review scene {} does not belong to branch {branch_id} of project {}",
                input.scene_id,
                input.project_id
            );
        }

        let requested_rounds = input.rounds.unwrap_or(2).clamp(1, 3);
        let scene_fingerprint = scene_revision_fingerprint(&scene);
        let job_key = format!("{branch_id}|{}|{scene_fingerprint}", input.scene_id);

        // Rounds already banked against THIS prose version.
        let prior_rounds = self
            .persisted_review_rounds(&branch_id, &input.scene_id, &scene_fingerprint)
            .await?;

        // An in-flight job for this exact key decides the target: joining it
        // must not stack another `requested_rounds` on top of what it is
        // already producing, or a post-timeout retry would silently double the
        // review depth. Absent a job, the request tops up from what is banked
        // (the established `rounds` semantics).
        // Clearance is checked HERE, on the caller's thread, before any work is
        // detached. A `review` route that is not cleared for this scene's rating
        // must fail with the TYPED `RouteClearanceError` the caller downcasts
        // on - the auto-checkpoint path uses it to fall back to manual review
        // rather than reporting the run blocked. A detached task can only hand
        // back a flattened string, so this case must never reach one. It also
        // means an uncleared route costs nothing: no job, no spawn, no prose.
        self.repository.model_router().check_route_clearance(
            "review",
            Some(scene.content_rating.trim().to_ascii_lowercase()).as_deref(),
        )?;

        let existing_job = self.lookup_review_job(&job_key)?;
        let target_rounds = match existing_job.as_ref() {
            Some(job) => job.target_rounds,
            None => prior_rounds.len() + requested_rounds,
        };

        // Already satisfied by persisted work — collect without touching the
        // model. This is what makes a retry after a client timeout idempotent.
        if existing_job.is_none() && prior_rounds.len() >= target_rounds {
            let persisted = self
                .repository
                .get_dual_persona_review(&branch_id, &input.scene_id)
                .await?;
            if let Some(persisted) = persisted {
                return Ok(RunDualPersonaReviewOutput {
                    scene_id: input.scene_id,
                    branch_id,
                    rounds_completed: prior_rounds.len(),
                    review_id: persisted.id,
                    status: persisted.status,
                    review_rounds: prior_rounds,
                });
            }
        }

        let job = match existing_job {
            Some(job) => job,
            None => self.spawn_review_job(&job_key, &input, &scene, target_rounds, prior_rounds)?,
        };

        // Bounded wait: answer the caller promptly either way.
        let mut state = job.state.clone();
        // Take an OWNED snapshot of the terminal state. The watch guard is not
        // `Send`, and holding it across the awaits below would make this whole
        // future non-`Send` and unusable from the MCP tool handler.
        let terminal: Option<ReviewJobState> = {
            let settled = tokio::time::timeout(
                REVIEW_JOB_JOIN_WAIT,
                state.wait_for(|s| !matches!(s, ReviewJobState::Running)),
            )
            .await;
            match settled {
                Ok(Ok(guard)) => Some(guard.clone()),
                // Sender dropped without a terminal state (the job task was torn
                // down). Treat it as settled so the next call starts fresh
                // rather than joining a corpse forever.
                Ok(Err(_)) => Some(ReviewJobState::Done),
                // Still running - report banked progress below.
                Err(_) => None,
            }
        };

        if let Some(terminal) = terminal {
            self.clear_review_job(&job_key)?;
            if let ReviewJobState::Failed(message) = terminal {
                anyhow::bail!("dual persona review failed: {message}");
            }
        }

        let banked = self
            .persisted_review_rounds(&branch_id, &input.scene_id, &scene_fingerprint)
            .await?;
        let persisted = self
            .repository
            .get_dual_persona_review(&branch_id, &input.scene_id)
            .await?;
        let (review_id, status) = match persisted {
            Some(row) => (row.id, row.status),
            // Nothing banked yet: the first round has not landed. Report the
            // honest in-progress shape rather than inventing a row id.
            None => (String::new(), "in_progress".to_string()),
        };

        Ok(RunDualPersonaReviewOutput {
            scene_id: input.scene_id,
            branch_id,
            rounds_completed: banked.len(),
            review_id,
            status,
            review_rounds: banked,
        })
    }

    /// Rounds already persisted for this scene AT this prose version. A stale
    /// row, or one whose fingerprint no longer matches, contributes nothing —
    /// the review restarts against the new prose.
    pub(super) async fn persisted_review_rounds(
        &self,
        branch_id: &str,
        scene_id: &str,
        scene_fingerprint: &str,
    ) -> Result<Vec<spindle_core::models::DualPersonaReviewRound>> {
        let Some(existing) = self
            .repository
            .get_dual_persona_review(branch_id, scene_id)
            .await?
        else {
            return Ok(Vec::new());
        };
        // `in_progress` counts: those rounds are real completed model work,
        // banked mid-run. Only a `stale` row (prose moved on) is discarded.
        if existing.status == "stale" || existing.scene_revision_fingerprint != scene_fingerprint {
            return Ok(Vec::new());
        }
        Ok(existing
            .review_rounds
            .get("rounds")
            .cloned()
            .and_then(|rounds| {
                serde_json::from_value::<
                    Vec<crate::sqlite::json_records::StoredDualPersonaReviewRound>,
                >(rounds)
                .ok()
            })
            .map(|rounds| {
                rounds
                    .into_iter()
                    .map(crate::sqlite::json_records::StoredDualPersonaReviewRound::into_core)
                    .collect()
            })
            .unwrap_or_default())
    }

    pub(super) fn lookup_review_job(&self, job_key: &str) -> Result<Option<ReviewJob>> {
        Ok(self
            .review_jobs
            .lock()
            .map_err(|_| poisoned_lock("review job registry lock"))?
            .get(job_key)
            .cloned())
    }

    pub(super) fn clear_review_job(&self, job_key: &str) -> Result<()> {
        self.review_jobs
            .lock()
            .map_err(|_| poisoned_lock("review job registry lock"))?
            .remove(job_key);
        Ok(())
    }

    /// Detach the remaining rounds onto a background task and register it so
    /// concurrent/retrying callers join rather than duplicate.
    pub(super) fn spawn_review_job(
        &self,
        job_key: &str,
        input: &spindle_core::models::RunDualPersonaReviewInput,
        scene: &crate::sqlite::records::Scene,
        target_rounds: usize,
        prior_rounds: Vec<spindle_core::models::DualPersonaReviewRound>,
    ) -> Result<ReviewJob> {
        let (tx, rx) = tokio::sync::watch::channel(ReviewJobState::Running);
        let job = ReviewJob {
            target_rounds,
            state: rx,
        };
        {
            let mut guard = self
                .review_jobs
                .lock()
                .map_err(|_| poisoned_lock("review job registry lock"))?;
            // Re-check under the lock: another caller may have registered a job
            // for this key between our lookup and here.
            if let Some(existing) = guard.get(job_key) {
                return Ok(existing.clone());
            }
            guard.insert(job_key.to_string(), job.clone());
        }

        let service = self.clone();
        let project_id = input.project_id.clone();
        let scene = scene.clone();
        let job_key_owned = job_key.to_string();
        tokio::spawn(async move {
            let outcome = service
                .drive_review_rounds(&project_id, &scene, target_rounds, prior_rounds)
                .await;
            let state = match outcome {
                Ok(()) => ReviewJobState::Done,
                Err(error) => {
                    tracing::warn!(
                        scene_id = %scene.id,
                        error = %format!("{error:#}"),
                        "dual persona review job failed"
                    );
                    ReviewJobState::Failed(format!("{error:#}"))
                }
            };
            // Best-effort: a dropped receiver just means nobody is waiting.
            let _ = tx.send(state);
            // Leave the entry for a waiter to collect; if none ever comes, drop
            // it here so the registry cannot grow without bound.
            tokio::time::sleep(Duration::from_secs(if cfg!(test) { 0 } else { 300 })).await;
            // The reaper runs detached with no caller to propagate to; a
            // poisoned registry is logged and stays observable: every
            // request-path registry access surfaces it as `Err`.
            if let Err(error) = service.clear_review_job(&job_key_owned) {
                tracing::warn!(
                    error = %format!("{error:#}"),
                    "review job reaper could not clear its registry entry"
                );
            }
        });

        Ok(job)
    }

    /// Run rounds until `target_rounds` are banked, persisting after EACH one.
    /// Per-round persistence is what makes the work survivable: a failure (or a
    /// process exit) at round N leaves rounds 1..N-1 durable and resumable
    /// instead of discarding every model call the run already paid for.
    pub(super) async fn drive_review_rounds(
        &self,
        project_id: &str,
        scene: &crate::sqlite::records::Scene,
        target_rounds: usize,
        prior_rounds: Vec<spindle_core::models::DualPersonaReviewRound>,
    ) -> Result<()> {
        let branch_id = scene.branch_id.clone();
        let scene_fingerprint = scene_revision_fingerprint(scene);

        // The Target Reader persona judges genre delivery, a different axis
        // from craft quality. It is populated from the project style contract;
        // skip it entirely when the project declares no style signal.
        let style_directive = self
            .style_directive_for(project_id, &branch_id)
            .await
            .unwrap_or_default();

        let mut all_rounds = prior_rounds;
        while all_rounds.len() < target_rounds {
            // Jobs are keyed by fingerprint, so an edit starts a second job.
            // Persistence is unique on (branch, scene): without this check an
            // in-flight job for fingerprint A can finish after B has started
            // and overwrite B's banked rounds.
            let live = self.repository.get_scene(&scene.id).await?;
            if scene_revision_fingerprint(&live) != scene_fingerprint {
                tracing::info!(
                    scene_id = %scene.id,
                    "dropping dual-persona job; scene prose moved on"
                );
                return Ok(());
            }
            let round = all_rounds.len() + 1;
            let completed = self
                .execute_review_round(scene, round, &style_directive)
                .await?;
            all_rounds.push(completed);

            let status = if all_rounds.len() >= target_rounds {
                "current"
            } else {
                "in_progress"
            };
            let persisted = self
                .repository
                .upsert_dual_persona_review(
                    crate::sqlite::repository::UpsertDualPersonaReviewParams {
                        project_id: project_id.to_string(),
                        branch_id: branch_id.clone(),
                        scene_id: scene.id.clone(),
                        rounds_completed: all_rounds.len(),
                        review_rounds: all_rounds.clone(),
                        scene_revision_fingerprint: scene_fingerprint.clone(),
                        status: status.to_string(),
                    },
                )
                .await?;
            if persisted.scene_revision_fingerprint != scene_fingerprint {
                return Ok(());
            }
        }
        Ok(())
    }

    /// One review round: every persona for this round dispatched CONCURRENTLY.
    ///
    /// The personas judge the same prose on independent axes and none consumes
    /// another's output, so running them in sequence only added their latencies
    /// together — on a reasoning model that turned a round from ~1 model call of
    /// wall time into three.
    pub(super) async fn execute_review_round(
        &self,
        scene: &crate::sqlite::records::Scene,
        round: usize,
        style_directive: &spindle_core::style::StyleDirective,
    ) -> Result<spindle_core::models::DualPersonaReviewRound> {
        use crate::ai::ModelRequest;
        use spindle_core::models::{DualPersonaReviewRound, PersonaReviewNotes};

        let run_genre_reader = !style_directive.is_empty();
        let genre_brief = style_directive.render_markdown().unwrap_or_default();
        let review_rating = Some(scene.content_rating.clone());
        let review_context = Some(crate::ai::RequestContext {
            project_id: Some(scene.project_id.clone()),
            book_id: None,
            chapter_id: Some(format!("{}.{}", scene.book_number, scene.chapter_number)),
            scene_id: Some(scene.id.clone()),
            ..Default::default()
        });
        let antislop_injection = scan_scene_anti_slop(&scene.full_text)
            .as_ref()
            .map(spindle_core::style::antislop::dual_persona_injection);
        let literary_antislop = antislop_injection
            .as_ref()
            .map(|injection| {
                format!(
                    "\n\n{}\n\n{}",
                    injection.report_block, injection.literary_critic_structure_block
                )
            })
            .unwrap_or_default();
        let craft_antislop = antislop_injection
            .as_ref()
            .map(|injection| {
                format!(
                    "\n\n{}\n\n{}",
                    injection.report_block, injection.craft_technician_block
                )
            })
            .unwrap_or_default();
        let router = self.repository.model_router();
        {
            let literary_request = ModelRequest {
                route: "review".to_string(),
                prompt: format!(
                    "{FICTION_REVIEW_PROMPT_FRAME}\n\n\
                         You are a literary critic reviewing round {round} of a scene draft.\n\n\
                         Scene summary: {}\n\n\
                         Full prose:\n{}\n\n\
                         Evaluate as a reader and literary critic. Focus on:\n\
                         - Emotional impact and reader engagement\n\
                         - Character voice authenticity\n\
                         - Tension, pacing, and scene structure\n\
                         - Hook strength (opening and closing)\n\n\
                         Format your response as:\n\
                         STRENGTHS:\n- one strength per line\n\n\
                         CONCERNS:\n- one concern per line\n\n\
                         Be specific to THIS scene. Reference actual lines, images, or moments.{literary_antislop}",
                    scene.summary, scene.full_text
                ),
                rating: review_rating.clone(),
                context: review_context.clone(),
            };
            let craft_request = ModelRequest {
                route: "review".to_string(),
                prompt: format!(
                    "{FICTION_REVIEW_PROMPT_FRAME}\n\n\
                         You are a craft technician reviewing round {round} of a scene draft.\n\n\
                         Tone: {}\n\
                         Scene summary: {}\n\n\
                         Full prose:\n{}\n\n\
                         Evaluate the prose craft. Focus on:\n\
                         - POV discipline and filter word usage\n\
                         - Sentence rhythm and variety\n\
                         - Sensory detail quality\n\
                         - Dialogue naturalness\n\
                         - Verb strength (linking verbs vs active verbs)\n\n\
                         Format your response as:\n\
                         STRENGTHS:\n- one strength per line\n\n\
                         CONCERNS:\n- one concern per line\n\n\
                         Be specific to THIS scene. Quote actual phrases that need work.{craft_antislop}",
                    scene.tone.as_deref().unwrap_or("unspecified"),
                    scene.summary,
                    scene.full_text
                ),
                rating: review_rating.clone(),
                context: review_context.clone(),
            };
            // Target Reader persona: only invoked when there is a style
            // contract to judge against.
            let genre_request = ModelRequest {
                route: "review".to_string(),
                prompt: format!(
                    "{FICTION_REVIEW_PROMPT_FRAME}\n\n\
                             You are the TARGET READER of this book's declared genre, reviewing \
                             round {round} of a scene draft. You are NOT a craft critic — you are \
                             the reader this book is FOR, judging whether the scene delivers what \
                             you came for.\n\n\
                             This project's style contract:\n{genre_brief}\n\n\
                             Author-declared tone: {}\n\
                             Scene summary: {}\n\n\
                             Full prose:\n{}\n\n\
                             As the target reader, answer honestly:\n\
                             - Did this scene deliver the genre experience the contract promises? \
                             (comedy: did I laugh? thriller: was I gripped? romance: did I feel the \
                             chemistry?)\n\
                             - Is the narrator's voice the voice the contract asks for, or has it \
                             drifted (e.g. into quiet literary introspection where it should be \
                             sarcastic and funny)?\n\
                             - Are the genre-critical characters present and doing their job?\n\
                             - Does the ending make me want the next chapter the way this genre \
                             demands (hook/cliffhanger/laugh), or close on a beat that's wrong for \
                             the genre?\n\
                             - Is the pacing what this genre's reader expects?\n\n\
                             Format your response as:\n\
                             STRENGTHS:\n- one strength per line\n\n\
                             CONCERNS:\n- one concern per line\n\n\
                             A scene that is well-crafted but OFF-GENRE is a CONCERN, not a \
                             strength. Be specific to THIS scene.",
                    scene.tone.as_deref().unwrap_or("unspecified"),
                    scene.summary,
                    scene.full_text
                ),
                rating: review_rating.clone(),
                context: review_context.clone(),
            };

            // The three personas dispatch together; the round costs one model
            // call of wall time instead of three.
            let genre_call = async {
                if !run_genre_reader {
                    return Ok(None);
                }
                router.complete(&genre_request).await.map(Some)
            };
            let (literary, craft, genre) = tokio::try_join!(
                router.complete(&literary_request),
                router.complete(&craft_request),
                genre_call
            )?;

            let genre_reader = match genre {
                Some(genre) => {
                    let (strengths, concerns) = if genre.adapter_kind == "local" {
                        (
                            vec!["local heuristic pass (no external model configured)".to_string()],
                            derive_genre_concerns(scene, style_directive),
                        )
                    } else {
                        parse_review_sections(&genre.output)
                    };
                    PersonaReviewNotes {
                        persona: "target_reader".to_string(),
                        strengths,
                        concerns,
                    }
                }
                None => PersonaReviewNotes {
                    persona: "target_reader".to_string(),
                    ..Default::default()
                },
            };

            let (literary_strengths, literary_concerns) = if literary.adapter_kind == "local" {
                (
                    vec!["local heuristic pass (no external model configured)".to_string()],
                    derive_literary_concerns(scene),
                )
            } else {
                parse_review_sections(&literary.output)
            };
            let (craft_strengths, craft_concerns) = if craft.adapter_kind == "local" {
                (
                    vec!["local heuristic pass (no external model configured)".to_string()],
                    derive_craft_concerns(scene),
                )
            } else {
                parse_review_sections(&craft.output)
            };
            // Genre delivery is the primary quality metric, so genre-reader
            // concerns lead the priority actions.
            let priority_actions = if literary.adapter_kind == "local" {
                let mut actions: Vec<String> = genre_reader
                    .concerns
                    .iter()
                    .take(2)
                    .map(|c| format!("Genre fix: {c}"))
                    .collect();
                actions.extend(derive_review_actions(scene));
                actions
            } else {
                let mut actions: Vec<String> = genre_reader
                    .concerns
                    .iter()
                    .take(2)
                    .map(|c| format!("Genre fix: {c}"))
                    .chain(
                        literary_concerns
                            .iter()
                            .take(1)
                            .chain(craft_concerns.iter().take(1))
                            .map(|c| format!("Address: {c}")),
                    )
                    .collect();
                if actions.is_empty() {
                    actions.push("No critical actions identified.".to_string());
                }
                actions
            };

            Ok(DualPersonaReviewRound {
                round,
                literary_critic: PersonaReviewNotes {
                    persona: "literary_critic".to_string(),
                    strengths: literary_strengths,
                    concerns: literary_concerns,
                },
                craft_technician: PersonaReviewNotes {
                    persona: "craft_technician".to_string(),
                    strengths: craft_strengths,
                    concerns: craft_concerns,
                },
                genre_reader,
                priority_actions,
            })
        }
    }
}
