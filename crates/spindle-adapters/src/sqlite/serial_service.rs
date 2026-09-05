//! Long-running series read models and local editorial workflows.
use super::SqliteSpindleService;
use anyhow::Context;
use anyhow::Result;
use rusqlite::OptionalExtension;
use sha2::Digest;
use spindle_core::serial::*;
use std::collections::BTreeMap;

impl SqliteSpindleService {
    pub async fn get_editorial_queue(
        &self,
        input: GetEditorialQueueInput,
    ) -> Result<EditorialQueueOutput> {
        let signature = reader_model_signature(self)?;
        self.repository()
            .pool()
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    let branch =
                        editorial_branch(&tx, &input.project_id, input.branch_id.as_deref())?;
                    let items = editorial_items(&tx, &input.project_id, &branch)?;
                    let items: Vec<_> = items
                        .into_iter()
                        .filter(|i| {
                            input.status.map_or(
                                !matches!(
                                    i.status,
                                    EditorialStatus::Resolved | EditorialStatus::Dismissed
                                ),
                                |s| s == i.status,
                            )
                        })
                        .collect();
                    let total = items.len();
                    let offset = input.offset.unwrap_or(0);
                    let limit = input.limit.unwrap_or(30).clamp(1, 100);
                    let page: Vec<_> = items.into_iter().skip(offset).take(limit).collect();
                    let hashes = if let Some(last) = page
                        .iter()
                        .max_by_key(|i| (i.book_number, i.chapter_number))
                    {
                        reader_source(
                            &tx,
                            &ReadEpisodeInput {
                                project_id: input.project_id,
                                branch_id: Some(branch),
                                book_number: last.book_number,
                                chapter_number: last.chapter_number,
                                force: false,
                            },
                            &signature,
                        )?
                        .hashes
                    } else {
                        BTreeMap::new()
                    };
                    let items = page
                        .into_iter()
                        .map(|item| {
                            let current_source_hash = hashes
                                .get(&(item.book_number, item.chapter_number))
                                .cloned();
                            EditorialItemView {
                                source_changed: current_source_hash.as_ref()
                                    != Some(&item.source_hash),
                                current_source_hash,
                                item,
                            }
                        })
                        .collect();
                    tx.commit()?;
                    Ok(EditorialQueueOutput {
                        items,
                        total,
                        next_offset: (offset.saturating_add(limit) < total)
                            .then_some(offset.saturating_add(limit)),
                    })
                })())
            })
            .await?
    }

    pub async fn decide_editorial_item(
        &self,
        input: DecideEditorialItemInput,
    ) -> Result<EditorialItem> {
        anyhow::ensure!(
            input.note.len() <= 4000,
            "editorial note is limited to 4000 bytes"
        );
        anyhow::ensure!(
            !matches!(
                input.status,
                EditorialStatus::Accepted | EditorialStatus::Resolved
            ) || !input.note.trim().is_empty(),
            "describe the requested revision or how it was resolved"
        );
        let signature = reader_model_signature(self)?;
        self.repository()
            .pool()
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    let mut item: EditorialItem = tx.query_row(
                        "SELECT payload FROM editorial_item WHERE id = ?1 AND project_id = ?2",
                        rusqlite::params![input.item_id, input.project_id],
                        |r| super::row::json(r, 0),
                    )?;
                    anyhow::ensure!(
                        item.revision == input.expected_revision,
                        "editorial decision changed; reload the queue"
                    );
                    let source = reader_source(
                        &tx,
                        &ReadEpisodeInput {
                            project_id: item.project_id.clone(),
                            branch_id: Some(item.branch_id.clone()),
                            book_number: item.book_number,
                            chapter_number: item.chapter_number,
                            force: false,
                        },
                        &signature,
                    )?;
                    let current_hash = source.hashes.get(&(item.book_number, item.chapter_number));
                    anyhow::ensure!(
                        current_hash == input.reviewed_source_hash.as_ref(),
                        "source changed since review; reload the queue and inspect the manuscript"
                    );
                    anyhow::ensure!(
                        current_hash.is_some()
                            || matches!(
                                input.status,
                                EditorialStatus::Dismissed | EditorialStatus::Resolved
                            ),
                        "source chapter was removed; only resolve or dismiss this item"
                    );
                    item.status = input.status;
                    item.revision = item
                        .revision
                        .checked_add(1)
                        .context("editorial revision overflow")?;
                    item.decisions.push(EditorialDecision {
                        status: input.status,
                        note: input.note.trim().into(),
                        reviewed_source_hash: input.reviewed_source_hash,
                        recorded_at: chrono::Utc::now().to_rfc3339(),
                    });
                    tx.execute(
                        "UPDATE editorial_item SET payload = ?2 WHERE id = ?1",
                        rusqlite::params![item.id, serde_json::to_string(&item)?],
                    )?;
                    tx.commit()?;
                    Ok(item)
                })())
            })
            .await?
    }

    pub(crate) async fn editorial_threads(
        &self,
        project_id: &str,
        branch_id: &str,
        book: i32,
        chapter: i32,
    ) -> Result<Vec<spindle_core::models::ActiveThreadSummary>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.repository()
            .pool()
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    Ok(editorial_items(conn, &project_id, &branch_id)?
                        .into_iter()
                        .rev()
                        .filter(|i| {
                            i.status == EditorialStatus::Accepted
                                && (i.book_number, i.chapter_number) <= (book, chapter)
                        })
                        .take(5)
                        .map(|i| spindle_core::models::ActiveThreadSummary {
                            id: i.id,
                            kind: "editorial".into(),
                            name: format!(
                                "Revision request from book {} chapter {}",
                                i.book_number, i.chapter_number
                            ),
                            statement: crate::format::truncate_to_bytes(&i.description, 300).into(),
                            status: "accepted; unresolved".into(),
                            next_expectation: i
                                .decisions
                                .last()
                                .map(|d| crate::format::truncate_to_bytes(&d.note, 400).into()),
                        })
                        .collect())
                })())
            })
            .await?
    }

    pub async fn read_episode(&self, input: ReadEpisodeInput) -> Result<ReadEpisodeOutput> {
        let model_signature = reader_model_signature(self)?;
        let source = self
            .repository()
            .pool()
            .read({
                let input = input.clone();
                let signature = model_signature.clone();
                move |conn| {
                    Ok((|| -> Result<_> {
                        let tx = conn.transaction()?;
                        let source = reader_source(&tx, &input, &signature)?;
                        tx.commit()?;
                        Ok(source)
                    })())
                }
            })
            .await??;
        let target = (input.book_number, input.chapter_number);
        let source_hash = source
            .hashes
            .get(&target)
            .context("reader source cursor missing")?
            .clone();
        let prior = &source.prior;
        let expected_parent_id = prior.as_ref().map(|p| p.id.clone());
        let mut trace = ReaderMemoryTrace {
            source_hash: source_hash.clone(),
            loaded_memory_id: prior.as_ref().map(|p| p.id.clone()),
            stale_records_ignored: source.stale_records,
            unread_prior_chapters: source
                .hashes
                .keys()
                .filter(|position| **position < target)
                .map(|(b, c)| format!("{b}:{c}"))
                .filter(|key| {
                    prior
                        .as_ref()
                        .is_none_or(|p| !p.chapters_read.contains(key))
                })
                .collect(),
            ..Default::default()
        };
        if let Some(prior) = prior
            && (prior.book_number, prior.chapter_number) == target
        {
            trace.cached = true;
            trace.stored_memory_id = Some(prior.id.clone());
            return Ok(ReadEpisodeOutput {
                outcome: prior.outcome.clone(),
                memory: trace,
            });
        }
        let mut prior_notes = prior
            .as_ref()
            .map(|p| p.outcome.notes.clone())
            .unwrap_or_default();
        if let Some(prior) = prior
            && !prior.open_questions.is_empty()
        {
            prior_notes.push_str(&format!(
                "\nOpen reader questions:\n{}",
                prior.open_questions.join("\n")
            ));
        }
        if !trace.unread_prior_chapters.is_empty() {
            prior_notes.push_str(&format!("\n{} earlier chapters have not been reviewed. Treat their events as unknown; do not fill gaps by invention.", trace.unread_prior_chapters.len()));
        }
        let rating = [
            Some(source.rating.clone()),
            prior.as_ref().map(|p| p.rating.clone()),
        ]
        .into_iter()
        .flatten()
        .max_by_key(|r| serial_rating_rank(r))
        .unwrap_or_else(|| "general".into());
        let mut outcome = self
            .reader_sim_chapter(spindle_core::models::ReaderSimChapterInput {
                project_id: input.project_id.clone(),
                scene_ids: source.scene_ids.clone(),
                rating: Some(rating.clone()),
                prior_notes,
            })
            .await?;
        if outcome.status != "read" {
            trace.persistence_note = Some(format!(
                "reading {}: previous memory preserved",
                outcome.status
            ));
            return Ok(ReadEpisodeOutput {
                outcome,
                memory: trace,
            });
        }
        outcome.notes = crate::format::truncate_to_bytes(&outcome.notes, 4000).to_string();
        let questions = outcome
            .open_questions
            .clone()
            .unwrap_or_else(|| {
                prior
                    .as_ref()
                    .map(|p| p.open_questions.clone())
                    .unwrap_or_default()
            })
            .into_iter()
            .filter(|q| !q.trim().is_empty())
            .take(12)
            .map(|q| crate::format::truncate_to_bytes(&q, 240).to_string())
            .collect::<Vec<_>>();
        outcome.open_questions = Some(questions.clone());
        let mut chapters_read = prior
            .as_ref()
            .map(|p| p.chapters_read.clone())
            .unwrap_or_default();
        chapters_read.push(format!("{}:{}", target.0, target.1));
        let record = ReaderMemoryRecord {
            id: format!("reader_memory:{}", ulid::Ulid::new()),
            project_id: input.project_id.clone(),
            branch_id: source.branch_id,
            book_number: target.0,
            chapter_number: target.1,
            source_hash: source_hash.clone(),
            chapters_read,
            open_questions: questions,
            rating,
            outcome: outcome.clone(),
        };
        let id = record.id.clone();
        if reader_model_signature(self)? != model_signature {
            trace.persistence_note =
                Some("Model configuration changed during reading; memory was not stored".into());
            return Ok(ReadEpisodeOutput {
                outcome,
                memory: trace,
            });
        }
        let stored = self.repository().pool().write(move |conn| {
            Ok((|| -> Result<bool> {
                let tx = conn.transaction()?;
                let mut check_input = input;
                check_input.branch_id = Some(record.branch_id.clone());
                let current = reader_source(&tx, &check_input, &model_signature)?;
                if current.hashes.get(&target) != Some(&source_hash) || current.prior.as_ref().map(|p| &p.id) != expected_parent_id.as_ref() { return Ok(false); }
                // Derived caches after this chapter depend on this reading.
                // Refreshing an earlier read invalidates that dependent suffix.
                tx.execute("DELETE FROM reader_memory WHERE project_id = ?1 AND branch_id = ?2 AND (book_number, chapter_number) >= (?3, ?4)",
                    rusqlite::params![record.project_id, record.branch_id, target.0, target.1])?;
                tx.execute("INSERT INTO reader_memory (id, project_id, branch_id, book_number, chapter_number, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![record.id, record.project_id, record.branch_id, target.0, target.1, serde_json::to_string(&record)?])?;
                stage_reader_concerns(&tx, &record, &source.scene_ids)?;
                tx.commit()?; Ok(true)
            })())
        }).await;
        match stored {
            Ok(Ok(true)) => trace.stored_memory_id = Some(id),
            Ok(Ok(false)) => {
                trace.persistence_note =
                    Some("Manuscript or prior reading changed; memory was not stored".into())
            }
            _ => {
                trace.persistence_note = Some(
                    "Reading completed, but memory could not be stored; retry the reading".into(),
                )
            }
        }
        Ok(ReadEpisodeOutput {
            outcome,
            memory: trace,
        })
    }

    pub async fn prepare_episode_release(
        &self,
        input: PrepareEpisodeReleaseInput,
    ) -> Result<EpisodeReleasePreview> {
        self.repository()
            .pool()
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    let preview = episode_preview(&tx, &input)?;
                    tx.commit()?;
                    Ok(preview)
                })())
            })
            .await?
    }

    pub async fn release_episode(&self, input: ReleaseEpisodeInput) -> Result<EpisodeRelease> {
        self.repository().pool().write(move |conn| {
            Ok((|| -> Result<_> {
                let tx = conn.transaction()?;
                let preview = episode_preview(&tx, &PrepareEpisodeReleaseInput {
                    project_id: input.project_id.clone(), book_number: input.book_number, chapter_number: input.chapter_number,
                })?;
                anyhow::ensure!(preview.blocking_issues.is_empty(), "episode is not ready: {}", preview.blocking_issues.join("; "));
                anyhow::ensure!(preview.source_hash == input.expected_source_hash, "episode changed since preview; prepare the release again");
                let previous = latest_episode_release(&tx, &input.project_id, input.book_number, input.chapter_number)?;
                if let Some(previous) = &previous && previous.source_hash == preview.source_hash {
                    return Ok(previous.clone()); // retry of an already-recorded release
                }
                anyhow::ensure!(preview.previous_release_id == input.previous_release_id, "release history changed; prepare the release again before correcting it");
                let release = EpisodeRelease {
                    id: format!("episode_release:{}", ulid::Ulid::new()),
                    revision: previous.map_or(Some(1), |r| r.revision.checked_add(1)).context("release revision overflow")?, source_hash: preview.source_hash,
                    previous_release_id: preview.previous_release_id, released_at: chrono::Utc::now().to_rfc3339(),
                    note: input.note, snapshot: preview.snapshot,
                };
                tx.execute("INSERT INTO episode_release (id, project_id, book_number, chapter_number, revision, source_hash, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![release.id, input.project_id, input.book_number, input.chapter_number, release.revision, release.source_hash, serde_json::to_string(&release)?])?;
                tx.commit()?;
                Ok(release)
            })())
        }).await?
    }

    pub async fn get_episode_release(
        &self,
        input: GetEpisodeReleaseInput,
    ) -> Result<EpisodeRelease> {
        self.repository()
            .pool()
            .read(move |conn| {
                conn.query_row(
                    "SELECT payload FROM episode_release WHERE id = ?1",
                    [&input.release_id],
                    |r| super::row::json(r, 0),
                )
            })
            .await
    }

    pub async fn get_series_status(
        &self,
        input: GetSeriesStatusInput,
    ) -> Result<SeriesStatusOutput> {
        let limit = input.limit.unwrap_or(50);
        anyhow::ensure!(
            (1..=200).contains(&limit),
            "limit must be between 1 and 200"
        );
        self.repository().pool().read(move |conn| {
            Ok((|| -> Result<_> {
                let tx = conn.transaction()?;
                let branch_id: String = tx.query_row("SELECT active_branch_id FROM project WHERE id = ?1", [&input.project_id], |r| r.get(0))?;
                let chapters = {
                    let mut stmt = tx.prepare("SELECT book_number, chapter_number FROM chapter WHERE project_id = ?1 ORDER BY book_number, chapter_number")?;
                    stmt.query_map([&input.project_id], |r| Ok((r.get::<_, i32>(0)?, r.get::<_, i32>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
                };
                let mut output = SeriesStatusOutput { project_id: input.project_id.clone(), branch_id, published_through: None,
                    released_episodes: 0, draft_backlog: 0, ready_backlog: 0, episodes: Vec::new(), next_offset: None };
                let mut contiguous = true;
                let mut previous_position = None;
                // ponytail: status validates current prose in O(manuscript size).
                // Cache per-chapter source hashes if this becomes a measured UI bottleneck.
                for (book_number, chapter_number) in chapters {
                    let preview = episode_preview(&tx, &PrepareEpisodeReleaseInput { project_id: input.project_id.clone(), book_number, chapter_number })?;
                    let release = latest_episode_release(&tx, &input.project_id, book_number, chapter_number)?;
                    let adjacent = match previous_position {
                        None => book_number == 1 && (0..=1).contains(&chapter_number),
                        Some((book, chapter)) => (book_number == book && chapter_number == chapter + 1)
                            || (book_number == book + 1 && (0..=1).contains(&chapter_number)),
                    };
                    contiguous &= adjacent && release.is_some();
                    if contiguous { output.published_through = Some(spindle_core::models::StoryPlacement { book_number, chapter_number, scene_order: None, note: None }); }
                    previous_position = Some((book_number, chapter_number));
                    if release.is_some() { output.released_episodes += 1; }
                    else if preview.snapshot.word_count > 0 { output.draft_backlog += 1; }
                    if release.is_none() && preview.blocking_issues.is_empty() { output.ready_backlog += 1; }
                    if input.book_number.is_some_and(|book| book != book_number) { continue; }
                    output.episodes.push(EpisodeStatus { book_number, chapter_number, title: preview.snapshot.title,
                        word_count: preview.snapshot.word_count, ready: preview.blocking_issues.is_empty(), blocking_issues: preview.blocking_issues,
                        latest_release_id: release.as_ref().map(|r| r.id.clone()), release_revision: release.as_ref().map(|r| r.revision),
                        changed_since_release: release.is_some_and(|r| r.source_hash != preview.source_hash),
                    });
                }
                let offset = input.offset.unwrap_or(0);
                output.next_offset = offset.checked_add(limit).filter(|next| *next < output.episodes.len());
                output.episodes = output.episodes.into_iter().skip(offset).take(limit).collect();
                tx.commit()?;
                Ok(output)
            })())
        }).await?
    }

    pub async fn get_model_usage(
        &self,
        input: spindle_core::models::GetModelUsageInput,
    ) -> Result<spindle_core::models::GetModelUsageOutput> {
        use spindle_core::models::GetModelUsageOutput;
        if let Some(project_id) = &input.project_id {
            self.repository().get_project(project_id).await?;
        }
        let limit = input.limit.unwrap_or(50);
        anyhow::ensure!(
            (1..=200).contains(&limit),
            "limit must be between 1 and 200"
        );
        self.repository().pool().read(move |conn| {
            let mut stmt = conn.prepare("SELECT payload FROM model_call WHERE (?1 IS NULL OR project_id = ?1) ORDER BY recorded_at DESC, id DESC LIMIT ?2")?;
            let calls = stmt.query_map(rusqlite::params![input.project_id, limit as i64], |row| super::row::json(row, 0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let (total_calls, calls_with_unknown_tokens, known_input_tokens, known_output_tokens, elapsed_ms) = conn.query_row(
                "SELECT count(*), coalesce(sum(json_extract(payload, '$.usage.input_tokens') IS NULL OR json_extract(payload, '$.usage.output_tokens') IS NULL), 0),
                 coalesce(sum(json_extract(payload, '$.usage.input_tokens')), 0), coalesce(sum(json_extract(payload, '$.usage.output_tokens')), 0),
                 coalesce(sum(json_extract(payload, '$.elapsed_ms')), 0)
                 FROM model_call WHERE (?1 IS NULL OR project_id = ?1)", [&input.project_id],
                 |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))?;
            let unattributed_calls = conn.query_row("SELECT count(*) FROM model_call WHERE project_id IS NULL", [], |row| row.get(0))?;
            Ok(GetModelUsageOutput { calls, total_calls, calls_with_unknown_tokens, known_input_tokens, known_output_tokens, elapsed_ms, unattributed_calls })
        }).await
    }

    pub(crate) async fn story_so_far(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<String> {
        let repo = self.repository();
        let mut books: BTreeMap<i32, Vec<(i32, String)>> = BTreeMap::new();
        for summary in repo.list_chapter_summaries_by_project(project_id).await? {
            if summary.branch_id == branch_id
                && (summary.book_number, summary.chapter_number) < (book_number, chapter_number)
            {
                books
                    .entry(summary.book_number)
                    .or_default()
                    .push((summary.chapter_number, summary.summary));
            }
        }
        for parts in books.values_mut() {
            parts.sort_by_key(|(chapter, _)| *chapter);
        }
        let cursor = crate::format::story_index(book_number, chapter_number, scene_order);
        let promises = repo
            .list_narrative_promises_by_project_and_branch(project_id, branch_id)
            .await?
            .into_iter()
            .filter(|p| p.archived_at.is_none())
            .filter_map(|mut p| {
                p.status = p.status_at(cursor)?.to_string();
                Some(p)
            })
            .collect::<Vec<_>>();
        let conflicts = repo
            .list_conflicts_by_project_and_branch(project_id, branch_id)
            .await?
            .into_iter()
            .filter(|c| c.archived_at.is_none())
            .collect::<Vec<_>>();
        let plots = repo
            .list_plot_lines_by_project_and_branch(project_id, branch_id)
            .await?
            .into_iter()
            .filter(|p| p.archived_at.is_none())
            .collect::<Vec<_>>();
        let threads = crate::format::build_open_threads(&promises, &conflicts, &plots, cursor);
        Ok(crate::format::render_story_so_far(&books, &threads))
    }
}

fn editorial_branch(
    conn: &rusqlite::Connection,
    project_id: &str,
    requested: Option<&str>,
) -> Result<String> {
    let branch: String = match requested {
        Some(branch) => branch.into(),
        None => conn.query_row(
            "SELECT active_branch_id FROM project WHERE id = ?1",
            [project_id],
            |r| r.get(0),
        )?,
    };
    let belongs: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM bible_branch WHERE id = ?1 AND project_id = ?2)",
        rusqlite::params![branch, project_id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(belongs, "editorial branch does not belong to project");
    Ok(branch)
}

fn editorial_items(
    conn: &rusqlite::Connection,
    project: &str,
    branch: &str,
) -> Result<Vec<EditorialItem>> {
    // ponytail: JSON queue scan; index status/cursor only if a measured queue outgrows this.
    let mut stmt = conn.prepare(
        "SELECT payload FROM editorial_item WHERE project_id = ?1 AND branch_id = ?2 ORDER BY id",
    )?;
    Ok(stmt
        .query_map(rusqlite::params![project, branch], |r| {
            super::row::json(r, 0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn stage_reader_concerns(
    conn: &rusqlite::Connection,
    memory: &ReaderMemoryRecord,
    scene_ids: &[String],
) -> Result<()> {
    for concern in memory
        .outcome
        .concerns
        .iter()
        .filter(|c| !c.description.trim().is_empty())
        .take(20)
    {
        let description =
            crate::format::truncate_to_bytes(concern.description.trim(), 2000).to_string();
        let key = format!(
            "{:x}",
            sha2::Sha256::digest(serde_json::to_vec(&(
                &memory.project_id,
                &memory.branch_id,
                memory.book_number,
                memory.chapter_number,
                &memory.source_hash,
                &description
            ))?)
        );
        let item = EditorialItem {
            id: format!("editorial_item:{}", ulid::Ulid::new()),
            project_id: memory.project_id.clone(),
            branch_id: memory.branch_id.clone(),
            book_number: memory.book_number,
            chapter_number: memory.chapter_number,
            source_hash: memory.source_hash.clone(),
            reader_memory_id: memory.id.clone(),
            scene_ids: scene_ids.to_vec(),
            severity: concern.severity.clone(),
            description,
            status: EditorialStatus::Open,
            revision: 0,
            decisions: vec![],
        };
        conn.execute("INSERT INTO editorial_item (id, project_id, branch_id, dedupe_key, payload) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(dedupe_key) DO NOTHING", rusqlite::params![item.id, item.project_id, item.branch_id, key, serde_json::to_string(&item)?])?;
    }
    Ok(())
}

fn serial_rating_rank(rating: &str) -> u8 {
    match rating.to_ascii_lowercase().as_str() {
        "general" => 0,
        "teen" => 1,
        "mature" => 2,
        _ => 3,
    }
}

fn reader_model_signature(svc: &SqliteSpindleService) -> Result<String> {
    let routes: Vec<_> = svc
        .repository()
        .model_router()
        .list_routes()
        .into_iter()
        .filter(|r| matches!(r.route_name.as_str(), "reader_sim" | "review"))
        .collect();
    Ok(format!(
        "{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&routes)?)
    ))
}

struct ReaderSource {
    branch_id: String,
    scene_ids: Vec<String>,
    hashes: BTreeMap<(i32, i32), String>,
    prior: Option<ReaderMemoryRecord>,
    stale_records: usize,
    rating: String,
}

fn reader_source(
    conn: &rusqlite::Connection,
    input: &ReadEpisodeInput,
    model_signature: &str,
) -> Result<ReaderSource> {
    use super::records::*;
    anyhow::ensure!(
        input.book_number > 0 && input.chapter_number >= 0,
        "invalid reader cursor"
    );
    let project = conn.query_row(
        &format!("SELECT {PROJECT_COLUMNS} FROM project WHERE id = ?1"),
        [&input.project_id],
        |r| Project::try_from(r),
    )?;
    let branch_id = input
        .branch_id
        .clone()
        .or(project.active_branch_id)
        .context("project has no active branch")?;
    let belongs: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM bible_branch WHERE id = ?1 AND project_id = ?2)",
        rusqlite::params![branch_id, input.project_id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(belongs, "reader branch does not belong to project");
    let mut source = ReaderSource {
        branch_id,
        scene_ids: Vec::new(),
        hashes: BTreeMap::new(),
        prior: None,
        stale_records: 0,
        rating: "general".into(),
    };
    let mut hash = sha2::Sha256::new();
    hash.update(serde_json::to_vec(&(
        "reader-memory-v1",
        &input.project_id,
        &source.branch_id,
        project.reader_contract,
        model_signature,
    ))?);
    let mut stmt = conn.prepare(&format!("SELECT {SCENE_COLUMNS} FROM scene WHERE project_id = ?1 AND branch_id = ?2 AND (book_number, chapter_number) <= (?3, ?4) ORDER BY book_number, chapter_number, scene_order"))?;
    let scenes = stmt.query_map(
        rusqlite::params![
            input.project_id,
            source.branch_id,
            input.book_number,
            input.chapter_number
        ],
        |r| Scene::try_from(r),
    )?;
    let mut chapters: BTreeMap<(i32, i32), Vec<Scene>> = BTreeMap::new();
    for scene in scenes {
        let scene = scene?;
        chapters
            .entry((scene.book_number, scene.chapter_number))
            .or_default()
            .push(scene);
    }
    let mut chapter_stmt = conn.prepare("SELECT book_number, chapter_number FROM chapter WHERE project_id = ?1 AND (book_number, chapter_number) <= (?2, ?3) ORDER BY book_number, chapter_number")?;
    for position in chapter_stmt.query_map(
        rusqlite::params![input.project_id, input.book_number, input.chapter_number],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )? {
        chapters.entry(position?).or_default();
    }
    for (position, scenes) in chapters {
        hash.update(serde_json::to_vec(&position)?);
        for scene in scenes {
            hash.update(serde_json::to_vec(&(
                &scene.id,
                scene.scene_order,
                &scene.full_text,
                &scene.content_rating,
                scene.updated_at,
            ))?);
            if position == (input.book_number, input.chapter_number) {
                if serial_rating_rank(&scene.content_rating) > serial_rating_rank(&source.rating) {
                    source.rating = scene.content_rating.clone();
                }
                source.scene_ids.push(scene.id);
            }
        }
        source
            .hashes
            .insert(position, format!("{:x}", hash.clone().finalize()));
    }
    let mut stmt = conn.prepare("SELECT payload FROM reader_memory WHERE project_id = ?1 AND branch_id = ?2 AND (book_number, chapter_number) <= (?3, ?4) ORDER BY book_number DESC, chapter_number DESC")?;
    let records = stmt.query_map(
        rusqlite::params![
            input.project_id,
            source.branch_id,
            input.book_number,
            input.chapter_number
        ],
        |r| super::row::json::<ReaderMemoryRecord>(r, 0),
    )?;
    for record in records {
        let record = record?;
        let position = (record.book_number, record.chapter_number);
        if source.hashes.get(&position) != Some(&record.source_hash) {
            source.stale_records += 1;
            continue;
        }
        if input.force && position == (input.book_number, input.chapter_number) {
            continue;
        }
        source.prior = Some(record);
        break;
    }
    Ok(source)
}

fn latest_episode_release(
    conn: &rusqlite::Connection,
    project_id: &str,
    book: i32,
    chapter: i32,
) -> Result<Option<EpisodeRelease>> {
    Ok(conn.query_row("SELECT payload FROM episode_release WHERE project_id = ?1 AND book_number = ?2 AND chapter_number = ?3 ORDER BY revision DESC LIMIT 1",
        rusqlite::params![project_id, book, chapter], |r| super::row::json(r, 0)).optional()?)
}

fn episode_preview(
    conn: &rusqlite::Connection,
    input: &PrepareEpisodeReleaseInput,
) -> Result<EpisodeReleasePreview> {
    use super::records::*;
    anyhow::ensure!(
        input.book_number > 0
            && (0..crate::format::CHAPTER_RADIX as i32).contains(&input.chapter_number),
        "invalid episode placement"
    );
    let project = conn.query_row(
        &format!("SELECT {PROJECT_COLUMNS} FROM project WHERE id = ?1"),
        [&input.project_id],
        |r| Project::try_from(r),
    )?;
    let branch_id = project
        .active_branch_id
        .context("project has no active branch")?;
    let chapter = conn.query_row(&format!("SELECT {CHAPTER_COLUMNS} FROM chapter WHERE project_id = ?1 AND book_number = ?2 AND chapter_number = ?3"),
        rusqlite::params![input.project_id, input.book_number, input.chapter_number], |r| Chapter::try_from(r))?;
    let scenes = {
        let mut stmt = conn.prepare(&format!("SELECT {SCENE_COLUMNS} FROM scene WHERE project_id = ?1 AND branch_id = ?2 AND book_number = ?3 AND chapter_number = ?4 ORDER BY scene_order"))?;
        stmt.query_map(
            rusqlite::params![
                input.project_id,
                branch_id,
                input.book_number,
                input.chapter_number
            ],
            |r| Scene::try_from(r),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let planned: Option<Vec<StoredPlannedScene>> = conn.query_row("SELECT scenes FROM chapter_plan WHERE project_id = ?1 AND branch_id = ?2 AND book_number = ?3 AND chapter_number = ?4",
        rusqlite::params![input.project_id, branch_id, input.book_number, input.chapter_number], |r| super::row::json(r, 0)).optional()?;
    let min_words = project
        .min_scene_word_count
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n > 0)
        .unwrap_or(super::service::DEFAULT_MIN_SCENE_WORD_COUNT);
    let mut blocking_issues = Vec::new();
    if scenes.is_empty() {
        blocking_issues.push("No drafted scenes".into());
    }
    if planned.as_ref().is_none_or(|p| p.is_empty()) {
        blocking_issues.push("No scene plan; plan the episode before releasing it".into());
    }
    for planned in planned.iter().flatten() {
        if !scenes.iter().any(|s| s.scene_order == planned.scene_order) {
            blocking_issues.push(format!("Scene {} is not drafted", planned.scene_order));
        }
    }
    for scene in &scenes {
        if scene.chapter_id != chapter.id || scene.book_id != chapter.book_id {
            blocking_issues.push(format!(
                "Scene {} has an inconsistent chapter reference",
                scene.scene_order
            ));
        }
        if let Some(reason) = super::service::scene_stub_reason(&scene.full_text, min_words) {
            blocking_issues.push(format!("Scene {}: {reason}", scene.scene_order));
        }
    }
    let title = chapter
        .title
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("Chapter {}", chapter.chapter_number));
    let snapshot = EpisodeSnapshot {
        project_id: input.project_id.clone(),
        branch_id,
        book_number: input.book_number,
        chapter_number: input.chapter_number,
        markdown: format!(
            "# {title}\n\n{}\n",
            scenes
                .iter()
                .map(|s| s.full_text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n***\n\n")
        ),
        title,
        word_count: scenes
            .iter()
            .map(|s| s.full_text.split_whitespace().count())
            .sum(),
        scenes: scenes
            .iter()
            .map(|s| ReleasedScene {
                scene_id: s.id.clone(),
                scene_order: s.scene_order,
                text_sha256: format!("{:x}", sha2::Sha256::digest(s.full_text.as_bytes())),
                content_rating: s.content_rating.clone(),
                draft_origin: s.draft_origin.clone(),
            })
            .collect(),
    };
    let source_hash = format!("{:x}", sha2::Sha256::digest(serde_json::to_vec(&snapshot)?));
    let previous_release_id = latest_episode_release(
        conn,
        &input.project_id,
        input.book_number,
        input.chapter_number,
    )?
    .map(|r| r.id);
    Ok(EpisodeReleasePreview {
        snapshot,
        source_hash,
        blocking_issues,
        previous_release_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use spindle_core::models::*;

    async fn service() -> (tempfile::TempDir, SqliteSpindleService, String) {
        let tmp = tempfile::tempdir().unwrap();
        let pool = super::super::SqlitePool::open(&tmp.path().join("serial.db"))
            .await
            .unwrap();
        let svc = SqliteSpindleService::new(super::super::Repository::with_model_router(
            pool,
            tmp.path().into(),
            crate::ai::ModelRouter::local_only(),
        ));
        let project = svc
            .create_project(CreateProjectInput {
                name: "Serial".into(),
                project_type: "webserial".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Choices have lasting consequences".into(),
                    style_notes: vec![],
                    boundaries: vec![],
                },
            })
            .await
            .unwrap();
        (tmp, svc, project.project_id)
    }

    async fn plan(svc: &SqliteSpindleService, project_id: &str, chapter: i32, orders: &[i32]) {
        svc.plan_chapter(PlanChapterInput {
            project_id: project_id.into(),
            book_number: 1,
            chapter_number: chapter,
            pov_character_id: None,
            synopsis: "A consequential choice".into(),
            target_theme_ids: vec![],
            target_conflict_ids: vec![],
            target_plot_line_ids: vec![],
            scenes: orders
                .iter()
                .map(|order| PlanChapterSceneInput {
                    scene_order: *order,
                    purpose: "Make a choice".into(),
                    ..Default::default()
                })
                .collect(),
        })
        .await
        .unwrap();
    }

    async fn draft(
        svc: &SqliteSpindleService,
        project_id: &str,
        chapter: i32,
        order: i32,
        text: &str,
    ) {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.into(),
            book_number: 1,
            chapter_number: chapter,
            scene_order: order,
            full_text: text.into(),
            summary: "A choice was made".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    }

    fn release_input(preview: &EpisodeReleasePreview) -> ReleaseEpisodeInput {
        ReleaseEpisodeInput {
            project_id: preview.snapshot.project_id.clone(),
            book_number: preview.snapshot.book_number,
            chapter_number: preview.snapshot.chapter_number,
            expected_source_hash: preview.source_hash.clone(),
            previous_release_id: preview.previous_release_id.clone(),
            note: None,
        }
    }

    #[tokio::test]
    async fn editorial_decisions_require_reviewed_source_and_preserve_author_revision_intent() {
        let (_tmp, svc, project_id) = service().await;
        let text = "MOCK_READER_DIP. Mara went back to the market and bargained for the same key. Nobody remembered yesterday, and the gate was still locked when she returned.";
        draft(&svc, &project_id, 1, 1, text).await;
        let read_input = ReadEpisodeInput {
            project_id: project_id.clone(),
            branch_id: None,
            book_number: 1,
            chapter_number: 1,
            force: false,
        };
        let reading = svc.read_episode(read_input.clone()).await.unwrap();
        assert!(reading.memory.stored_memory_id.is_some());
        let queue_input = GetEditorialQueueInput {
            project_id: project_id.clone(),
            branch_id: None,
            status: None,
            offset: None,
            limit: None,
        };
        let queue = svc.get_editorial_queue(queue_input.clone()).await.unwrap();
        assert_eq!(queue.total, 1);
        let view = &queue.items[0];
        assert!(!view.source_changed);
        assert_eq!(view.item.scene_ids.len(), 1);
        let decision = DecideEditorialItemInput {
            project_id: project_id.clone(),
            item_id: view.item.id.clone(),
            expected_revision: 0,
            reviewed_source_hash: view.current_source_hash.clone(),
            status: EditorialStatus::Accepted,
            note: "Make the failed bargain change Mara's next choice".into(),
        };
        let accepted = svc.decide_editorial_item(decision.clone()).await.unwrap();
        assert_eq!(accepted.status, EditorialStatus::Accepted);
        assert!(
            svc.decide_editorial_item(decision.clone()).await.is_err(),
            "stale decision cannot overwrite author intent"
        );
        assert_eq!(
            svc.editorial_threads(&project_id, &accepted.branch_id, 1, 1)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            svc.editorial_threads(&project_id, &accepted.branch_id, 1, 0)
                .await
                .unwrap()
                .is_empty(),
            "future editorial work stays out of historical drafting"
        );
        svc.read_episode(ReadEpisodeInput {
            force: true,
            ..read_input
        })
        .await
        .unwrap();
        assert_eq!(
            svc.get_editorial_queue(queue_input.clone())
                .await
                .unwrap()
                .total,
            1,
            "same concern is idempotent and keeps its decision"
        );
        draft(
            &svc,
            &project_id,
            1,
            1,
            &format!("The revision changes her choice. {text}"),
        )
        .await;
        let resolve = DecideEditorialItemInput {
            expected_revision: 1,
            status: EditorialStatus::Resolved,
            note: "The failed bargain now forces a costly choice".into(),
            ..decision
        };
        assert!(
            svc.decide_editorial_item(resolve.clone()).await.is_err(),
            "an old preview cannot approve newly changed prose"
        );
        let changed = svc.get_editorial_queue(queue_input.clone()).await.unwrap();
        assert!(changed.items[0].source_changed);
        let resolved = svc
            .decide_editorial_item(DecideEditorialItemInput {
                reviewed_source_hash: changed.items[0].current_source_hash.clone(),
                ..resolve
            })
            .await
            .unwrap();
        assert_eq!(resolved.decisions.len(), 2);
        assert!(
            svc.editorial_threads(&project_id, &accepted.branch_id, 1, 1)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            svc.get_editorial_queue(queue_input.clone())
                .await
                .unwrap()
                .total,
            0
        );
        assert_eq!(
            svc.get_editorial_queue(GetEditorialQueueInput {
                status: Some(EditorialStatus::Resolved),
                ..queue_input
            })
            .await
            .unwrap()
            .total,
            1
        );
        let scene = svc
            .repository()
            .get_scene(&accepted.scene_ids[0])
            .await
            .unwrap();
        assert!(
            scene.full_text.starts_with("The revision changes"),
            "editorial decisions never rewrite prose"
        );
    }

    #[tokio::test]
    async fn reader_memory_survives_restart_and_books_but_invalidates_retcons_and_contract_changes()
    {
        let (tmp, svc, project_id) = service().await;
        let text = "Mara wondered who had left the key. MOCK_READER_NOTES_ECHO. She waited by the gate until the guard returned with a letter from her brother.";
        draft(&svc, &project_id, 1, 1, text).await;
        let input = ReadEpisodeInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            branch_id: None,
            force: false,
        };
        let first = svc.read_episode(input.clone()).await.unwrap();
        assert_eq!(first.outcome.status, "read");
        assert!(first.memory.stored_memory_id.is_some());
        let pool = super::super::SqlitePool::open(&tmp.path().join("serial.db"))
            .await
            .unwrap();
        let reopened = SqliteSpindleService::new(super::super::Repository::with_model_router(
            pool,
            tmp.path().into(),
            crate::ai::ModelRouter::local_only(),
        ));
        let log = reopened
            .repository()
            .model_router()
            .install_dispatch_recorder();
        let cached = reopened.read_episode(input.clone()).await.unwrap();
        assert!(cached.memory.cached);
        assert_eq!(
            cached.memory.stored_memory_id,
            first.memory.stored_memory_id
        );
        assert!(
            log.lock().unwrap().is_empty(),
            "unchanged reading avoids another model call"
        );
        reopened
            .create_book(CreateBookInput {
                project_id: project_id.clone(),
                title: Some("Book two".into()),
            })
            .await
            .unwrap();
        reopened
            .create_chapter(CreateChapterInput {
                project_id: project_id.clone(),
                book_number: Some(2),
                book_id: None,
                chapter_number: Some(1),
                title: None,
            })
            .await
            .unwrap();
        reopened
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project_id.clone(),
                book_number: 2,
                chapter_number: 1,
                scene_order: 1,
                full_text: text.into(),
                summary: "Next book".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let next_input = ReadEpisodeInput {
            book_number: 2,
            ..input.clone()
        };
        let next = reopened.read_episode(next_input.clone()).await.unwrap();
        assert_eq!(next.memory.loaded_memory_id, first.memory.stored_memory_id);
        assert!(next.memory.unread_prior_chapters.is_empty());
        assert!(
            next.outcome
                .notes
                .contains(&first.outcome.notes.chars().take(40).collect::<String>())
        );
        let historical = reopened.read_episode(input).await.unwrap();
        assert_eq!(
            historical.outcome.notes, first.outcome.notes,
            "a historical read cannot load later-book memory"
        );
        draft(&reopened, &project_id, 1, 1, &format!("RETCON. {text}")).await;
        let revised = reopened.read_episode(next_input.clone()).await.unwrap();
        assert!(!revised.memory.cached);
        assert!(revised.memory.loaded_memory_id.is_none());
        assert_eq!(revised.memory.stale_records_ignored, 2);
        assert_eq!(revised.memory.unread_prior_chapters, vec!["1:1"]);
        reopened
            .update_entity(UpdateEntityInput {
                entity_type: "project".into(),
                entity_id: project_id.clone(),
                changes: serde_json::json!({"promise": "A comic story about mistaken loyalty"}),
                allow_rename: None,
            })
            .await
            .unwrap();
        let changed_contract = reopened.read_episode(next_input.clone()).await.unwrap();
        assert!(changed_contract.memory.loaded_memory_id.is_none());
        assert_eq!(changed_contract.memory.stale_records_ignored, 2);
        let main = reopened
            .repository()
            .get_active_branch(&project_id)
            .await
            .unwrap();
        let fork = reopened
            .create_branch(CreateBranchInput {
                project_id: project_id.clone(),
                name: "Other reader".into(),
                branch_type: "draft".into(),
                description: None,
                parent_branch_id: Some(main.id),
            })
            .await
            .unwrap();
        let isolated = reopened
            .read_episode(ReadEpisodeInput {
                branch_id: Some(fork.branch_id),
                ..next_input
            })
            .await
            .unwrap();
        assert!(
            isolated.memory.loaded_memory_id.is_none()
                && isolated.memory.stored_memory_id.is_none()
        );
        assert_eq!(isolated.outcome.status, "skipped");
    }

    #[tokio::test]
    async fn episode_release_guards_gaps_stubs_stale_previews_and_preserves_corrections() {
        let (_tmp, svc, project_id) = service().await;
        let main = svc
            .repository()
            .get_active_branch(&project_id)
            .await
            .unwrap();
        let input = PrepareEpisodeReleaseInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
        };
        let prose = "Mara left the key beside the sleeping guard and walked back into the rain. Tomorrow someone else would have to decide whether the gate should open.";
        plan(&svc, &project_id, 1, &[1, 2]).await;
        draft(&svc, &project_id, 1, 1, prose).await;
        let preview = svc.prepare_episode_release(input.clone()).await.unwrap();
        assert!(
            preview
                .blocking_issues
                .iter()
                .any(|issue| issue.contains("not drafted"))
        );
        assert!(svc.release_episode(release_input(&preview)).await.is_err());
        draft(&svc, &project_id, 1, 2, "placeholder").await;
        let preview = svc.prepare_episode_release(input.clone()).await.unwrap();
        assert!(
            preview
                .blocking_issues
                .iter()
                .any(|issue| issue.contains("placeholder"))
        );
        draft(&svc, &project_id, 1, 2, prose).await;
        let stale = svc.prepare_episode_release(input.clone()).await.unwrap();
        draft(&svc, &project_id, 1, 1, &format!("Changed. {prose}")).await;
        assert!(
            svc.release_episode(release_input(&stale))
                .await
                .unwrap_err()
                .to_string()
                .contains("changed since preview")
        );
        let preview = svc.prepare_episode_release(input.clone()).await.unwrap();
        let first = svc.release_episode(release_input(&preview)).await.unwrap();
        let retry = svc.release_episode(release_input(&preview)).await.unwrap();
        assert_eq!(first.id, retry.id);
        assert_eq!(first.revision, 1);
        draft(&svc, &project_id, 1, 1, &format!("CORRECTED. {prose}")).await;
        let preview = svc.prepare_episode_release(input.clone()).await.unwrap();
        let mut wrong_parent = release_input(&preview);
        wrong_parent.previous_release_id = None;
        assert!(svc.release_episode(wrong_parent).await.is_err());
        let second = svc.release_episode(release_input(&preview)).await.unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(
            second.previous_release_id.as_deref(),
            Some(first.id.as_str())
        );
        let original = svc
            .get_episode_release(GetEpisodeReleaseInput {
                release_id: first.id.clone(),
            })
            .await
            .unwrap();
        assert!(!original.snapshot.markdown.contains("CORRECTED"));
        assert_eq!(original.snapshot.markdown, first.snapshot.markdown);
        assert!(
            svc.repository()
                .pool()
                .write(|conn| conn.execute("UPDATE episode_release SET revision = 99", []))
                .await
                .is_err()
        );

        let branch = svc
            .create_branch(CreateBranchInput {
                project_id: project_id.clone(),
                name: "Alternative".into(),
                branch_type: "draft".into(),
                description: None,
                parent_branch_id: Some(main.id.clone()),
            })
            .await
            .unwrap();
        svc.switch_branch(SwitchBranchInput {
            project_id: project_id.clone(),
            branch_id: branch.branch_id,
        })
        .await
        .unwrap();
        let fork = svc.prepare_episode_release(input).await.unwrap();
        assert!(
            !fork.blocking_issues.is_empty(),
            "missing branch prose cannot borrow the main release"
        );
        svc.switch_branch(SwitchBranchInput {
            project_id: project_id.clone(),
            branch_id: main.id,
        })
        .await
        .unwrap();
        plan(&svc, &project_id, 2, &[1]).await;
        plan(&svc, &project_id, 3, &[1]).await;
        draft(&svc, &project_id, 3, 1, prose).await;
        let later = svc
            .prepare_episode_release(PrepareEpisodeReleaseInput {
                project_id: project_id.clone(),
                book_number: 1,
                chapter_number: 3,
            })
            .await
            .unwrap();
        svc.release_episode(release_input(&later)).await.unwrap();
        plan(&svc, &project_id, 4, &[1]).await;
        draft(&svc, &project_id, 4, 1, prose).await;
        let series = svc
            .get_series_status(GetSeriesStatusInput {
                project_id,
                book_number: None,
                offset: None,
                limit: Some(2),
            })
            .await
            .unwrap();
        assert_eq!(
            series.published_through.unwrap().chapter_number,
            1,
            "unreleased chapter 2 stops the cursor"
        );
        assert_eq!(
            (
                series.released_episodes,
                series.draft_backlog,
                series.ready_backlog
            ),
            (2, 1, 1)
        );
        assert_eq!(series.next_offset, Some(2));
    }
}
