//! Layered derived-memory build, invalidation, and retrieval services (P2 PR G).

use super::*;
use crate::sqlite::row::timestamp_to_micros;
use sha2::{Digest, Sha256};
use spindle_core::memory::*;
use spindle_core::models::StoryPlacement;
use spindle_core::subject::{Subject, SubjectTable};
use spindle_core::subject_snapshot::SubjectKindSpecific;
use std::collections::{BTreeMap, BTreeSet};

fn layer_str(layer: MemoryLayerV1) -> &'static str {
    match layer {
        MemoryLayerV1::Scene => "scene",
        MemoryLayerV1::Chapter => "chapter",
        MemoryLayerV1::Arc => "arc",
        MemoryLayerV1::Book => "book",
    }
}

fn deterministic_summary_id(
    project_id: &str,
    branch_id: &str,
    layer: MemoryLayerV1,
    scope_key: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(b"|");
    hasher.update(branch_id.as_bytes());
    hasher.update(b"|");
    hasher.update(layer_str(layer).as_bytes());
    hasher.update(b"|");
    hasher.update(scope_key.as_bytes());
    format!("memory_summary:{:x}", hasher.finalize())
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    out
}

fn contains_name(text_lower: &str, name: &str) -> bool {
    let name = name.trim().to_lowercase();
    !name.is_empty() && text_lower.contains(&name)
}

fn open_thread_ids_at(
    project_id: &str,
    branch_id: &str,
    index: i64,
    promises: &[crate::sqlite::records::NarrativePromise],
    conflicts: &[crate::sqlite::records::Conflict],
    plots: &[crate::sqlite::records::PlotLine],
) -> Vec<String> {
    let mut out = Vec::new();
    for promise in promises {
        if promise.project_id != project_id || promise.branch_id != branch_id {
            continue;
        }
        let Some(status) = promise.status_at(index) else {
            continue;
        };
        if status != "paid_off" && status != "abandoned" {
            out.push(promise.id.clone());
        }
    }
    for conflict in conflicts {
        if conflict.project_id != project_id || conflict.branch_id != branch_id {
            continue;
        }
        if conflict
            .stated_consequences
            .iter()
            .any(|consequence| !consequence.delivered)
        {
            out.push(conflict.id.clone());
        }
    }
    for plot in plots {
        if plot.project_id != project_id || plot.branch_id != branch_id {
            continue;
        }
        if plot.status != "complete" {
            out.push(plot.id.clone());
        }
    }
    out
}

impl SqliteSpindleService {
    /// Rebuild scene -> chapter -> arc -> book derived summaries for a branch.
    /// Arc membership defaults to the explicit `unassigned` group until P3
    /// supplies mappings; no arc is invented.
    pub async fn rebuild_layered_memory(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<LayeredMemoryBuildReportV1> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let scenes = self
            .repository
            .list_scenes_resolved_for_branch(&project_id, &branch_id)
            .await?;
        let chapter_summaries = self
            .repository
            .list_chapter_summaries_by_project_and_branch(&project_id, &branch_id)
            .await?;
        let book_digests = self
            .repository
            .list_book_digests_by_project_and_branch(&project_id, &branch_id)
            .await?;
        let characters = self
            .repository
            .list_characters_by_project_and_branch(&project_id, &branch_id)
            .await?;
        let facts = self
            .repository
            .list_active_canonical_facts_by_project_and_branch(&project_id, &branch_id)
            .await?;
        let promises = self
            .repository
            .list_narrative_promises_by_project_and_branch(&project_id, &branch_id)
            .await?;
        let conflicts = self
            .repository
            .list_conflicts_by_project_and_branch(&project_id, &branch_id)
            .await?;
        let plots = self
            .repository
            .list_plot_lines_by_project_and_branch(&project_id, &branch_id)
            .await?;
        let secret_facts: Vec<_> = facts.iter().filter(|fact| fact.secret).collect();
        let chapter_summary_by_key: BTreeMap<(i32, i32), &crate::sqlite::records::ChapterSummary> =
            chapter_summaries
                .iter()
                .map(|summary| ((summary.book_number, summary.chapter_number), summary))
                .collect();
        let digest_by_book: BTreeMap<i32, &crate::sqlite::records::StoredBookDigest> = book_digests
            .iter()
            .map(|digest| (digest.book_number, digest))
            .collect();

        let now = timestamp_to_micros(chrono::Utc::now());
        let mut summaries = Vec::new();
        let mut edges = Vec::new();
        // (book, chapter) -> scene summary ids
        let mut scenes_by_chapter: BTreeMap<(i32, i32), Vec<String>> = BTreeMap::new();

        for scene in &scenes {
            let scene_lower = scene.full_text.to_lowercase();
            let entity_ids: Vec<String> = characters
                .iter()
                .filter(|character| {
                    contains_name(&scene_lower, &character.name)
                        || character
                            .aliases
                            .iter()
                            .any(|alias| contains_name(&scene_lower, alias))
                })
                .map(|character| character.id.clone())
                .collect();
            let secret_fact_ids: Vec<String> = secret_facts
                .iter()
                .filter(|fact| {
                    let value = crate::format::canonical_fact_value(fact);
                    !value.trim().is_empty() && scene_lower.contains(&value.to_lowercase())
                })
                .map(|fact| fact.id.clone())
                .collect();
            let index = crate::format::story_index(
                scene.book_number,
                scene.chapter_number,
                scene.scene_order,
            );
            let thread_ids = open_thread_ids_at(
                &project_id,
                &branch_id,
                index,
                &promises,
                &conflicts,
                &plots,
            );
            let text = if scene.summary.trim().is_empty() {
                truncate_chars(&scene.full_text, 600)
            } else {
                scene.summary.clone()
            };
            let scope_key = format!("scene:{}", scene.id);
            let summary_id =
                deterministic_summary_id(&project_id, &branch_id, MemoryLayerV1::Scene, &scope_key);
            summaries.push(MemorySummaryV1 {
                summary_id: summary_id.clone(),
                project_id: project_id.clone(),
                branch_id: branch_id.clone(),
                layer: MemoryLayerV1::Scene,
                scope_key,
                arc_id: None,
                coverage: MemoryCoverageV1 {
                    start: MemoryPositionV1::new(
                        scene.book_number,
                        scene.chapter_number,
                        scene.scene_order,
                    ),
                    end: MemoryPositionV1::new(
                        scene.book_number,
                        scene.chapter_number,
                        scene.scene_order,
                    ),
                },
                text,
                child_summary_ids: Vec::new(),
                entity_ids,
                thread_ids,
                source_revision_hashes: vec![spindle_core::canon::exact_text_sha256(
                    &scene.full_text,
                )],
                derived_event_ids: Vec::new(),
                secret_fact_ids,
                tier: MemoryTierV1::Ratified,
                status: MemorySummaryStatusV1::Fresh,
                reader_visible: true,
                model: "derived-scene-v1".into(),
                prompt_version: LAYERED_MEMORY_VERSION.into(),
                reducer_version: 1,
                updated_at_micros: now,
            });
            scenes_by_chapter
                .entry((scene.book_number, scene.chapter_number))
                .or_default()
                .push(summary_id);
        }

        // Chapter layer.
        let mut chapter_ids_by_book: BTreeMap<i32, Vec<String>> = BTreeMap::new();
        let mut chapter_coverage: BTreeMap<i32, (i32, i32)> = BTreeMap::new();
        for ((book, chapter), scene_ids) in &scenes_by_chapter {
            let chapter_text = chapter_summary_by_key
                .get(&(*book, *chapter))
                .map(|summary| summary.summary.clone())
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| {
                    let parts: Vec<String> = summaries
                        .iter()
                        .filter(|summary| scene_ids.contains(&summary.summary_id))
                        .map(|summary| summary.text.clone())
                        .collect();
                    truncate_chars(&parts.join(" "), 900)
                });
            let mut entity_ids = BTreeSet::new();
            let mut thread_ids = BTreeSet::new();
            let mut secret_fact_ids = BTreeSet::new();
            let mut source_hashes = BTreeSet::new();
            for scene_id in scene_ids {
                if let Some(scene) = summaries
                    .iter()
                    .find(|summary| &summary.summary_id == scene_id)
                {
                    entity_ids.extend(scene.entity_ids.iter().cloned());
                    thread_ids.extend(scene.thread_ids.iter().cloned());
                    secret_fact_ids.extend(scene.secret_fact_ids.iter().cloned());
                    source_hashes.extend(scene.source_revision_hashes.iter().cloned());
                }
            }
            let index = crate::format::story_index(*book, *chapter + 1, 1) - 1;
            thread_ids.extend(open_thread_ids_at(
                &project_id,
                &branch_id,
                index,
                &promises,
                &conflicts,
                &plots,
            ));
            let scope_key = format!("chapter:{book}:{chapter}");
            let summary_id = deterministic_summary_id(
                &project_id,
                &branch_id,
                MemoryLayerV1::Chapter,
                &scope_key,
            );
            summaries.push(MemorySummaryV1 {
                summary_id: summary_id.clone(),
                project_id: project_id.clone(),
                branch_id: branch_id.clone(),
                layer: MemoryLayerV1::Chapter,
                scope_key,
                arc_id: None,
                coverage: MemoryCoverageV1 {
                    start: MemoryPositionV1::new(*book, *chapter, 1),
                    end: MemoryPositionV1::new(*book, *chapter, i32::MAX),
                },
                text: chapter_text,
                child_summary_ids: scene_ids.clone(),
                entity_ids: entity_ids.into_iter().collect(),
                thread_ids: thread_ids.into_iter().collect(),
                source_revision_hashes: source_hashes.into_iter().collect(),
                derived_event_ids: Vec::new(),
                secret_fact_ids: secret_fact_ids.into_iter().collect(),
                tier: MemoryTierV1::Ratified,
                status: MemorySummaryStatusV1::Fresh,
                reader_visible: true,
                model: "derived-chapter-v1".into(),
                prompt_version: LAYERED_MEMORY_VERSION.into(),
                reducer_version: 1,
                updated_at_micros: now,
            });
            chapter_ids_by_book
                .entry(*book)
                .or_default()
                .push(summary_id);
            let entry = chapter_coverage
                .entry(*book)
                .or_insert((*chapter, *chapter));
            entry.0 = entry.0.min(*chapter);
            entry.1 = entry.1.max(*chapter);
            for scene_id in scene_ids {
                edges.push(MemoryDependencyEdgeV1 {
                    summary_id: deterministic_summary_id(
                        &project_id,
                        &branch_id,
                        MemoryLayerV1::Chapter,
                        &format!("chapter:{book}:{chapter}"),
                    ),
                    depends_on_summary_id: scene_id.clone(),
                    source_kind: "scene".into(),
                });
            }
        }

        // Arc layer: explicit unassigned group per book until P3 supplies arcs.
        let mut arc_ids_by_book: BTreeMap<i32, String> = BTreeMap::new();
        for (book, chapter_ids) in &chapter_ids_by_book {
            let (first_chapter, last_chapter) =
                chapter_coverage.get(book).copied().unwrap_or((1, 1));
            let text = truncate_chars(
                &chapter_ids
                    .iter()
                    .filter_map(|id| {
                        summaries
                            .iter()
                            .find(|summary| &summary.summary_id == id)
                            .map(|summary| summary.text.clone())
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                1600,
            );
            let mut entity_ids = BTreeSet::new();
            let mut thread_ids = BTreeSet::new();
            let mut secret_fact_ids = BTreeSet::new();
            let mut source_hashes = BTreeSet::new();
            for chapter_id in chapter_ids {
                if let Some(chapter) = summaries
                    .iter()
                    .find(|summary| &summary.summary_id == chapter_id)
                {
                    entity_ids.extend(chapter.entity_ids.iter().cloned());
                    thread_ids.extend(chapter.thread_ids.iter().cloned());
                    secret_fact_ids.extend(chapter.secret_fact_ids.iter().cloned());
                    source_hashes.extend(chapter.source_revision_hashes.iter().cloned());
                }
            }
            let scope_key = format!("arc:unassigned:{book}");
            let summary_id =
                deterministic_summary_id(&project_id, &branch_id, MemoryLayerV1::Arc, &scope_key);
            summaries.push(MemorySummaryV1 {
                summary_id: summary_id.clone(),
                project_id: project_id.clone(),
                branch_id: branch_id.clone(),
                layer: MemoryLayerV1::Arc,
                scope_key,
                arc_id: None,
                coverage: MemoryCoverageV1 {
                    start: MemoryPositionV1::new(*book, first_chapter, 1),
                    end: MemoryPositionV1::new(*book, last_chapter, i32::MAX),
                },
                text,
                child_summary_ids: chapter_ids.clone(),
                entity_ids: entity_ids.into_iter().collect(),
                thread_ids: thread_ids.into_iter().collect(),
                source_revision_hashes: source_hashes.into_iter().collect(),
                derived_event_ids: Vec::new(),
                secret_fact_ids: secret_fact_ids.into_iter().collect(),
                tier: MemoryTierV1::Ratified,
                status: MemorySummaryStatusV1::Fresh,
                reader_visible: true,
                model: "derived-arc-v1".into(),
                prompt_version: LAYERED_MEMORY_VERSION.into(),
                reducer_version: 1,
                updated_at_micros: now,
            });
            arc_ids_by_book.insert(*book, summary_id.clone());
            for chapter_id in chapter_ids {
                edges.push(MemoryDependencyEdgeV1 {
                    summary_id: summary_id.clone(),
                    depends_on_summary_id: chapter_id.clone(),
                    source_kind: "chapter".into(),
                });
            }
        }

        // Book layer.
        let mut book_count = 0usize;
        for (book, arc_id) in &arc_ids_by_book {
            let (_, last_chapter) = chapter_coverage.get(book).copied().unwrap_or((1, 1));
            let text = digest_by_book
                .get(book)
                .map(|digest| digest.synopsis.clone())
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| {
                    summaries
                        .iter()
                        .find(|summary| &summary.summary_id == arc_id)
                        .map(|summary| summary.text.clone())
                        .unwrap_or_default()
                });
            let mut entity_ids = BTreeSet::new();
            let mut thread_ids = BTreeSet::new();
            let mut secret_fact_ids = BTreeSet::new();
            let mut source_hashes = BTreeSet::new();
            if let Some(arc) = summaries
                .iter()
                .find(|summary| &summary.summary_id == arc_id)
            {
                entity_ids.extend(arc.entity_ids.iter().cloned());
                thread_ids.extend(arc.thread_ids.iter().cloned());
                secret_fact_ids.extend(arc.secret_fact_ids.iter().cloned());
                source_hashes.extend(arc.source_revision_hashes.iter().cloned());
            }
            let scope_key = format!("book:{book}");
            let summary_id =
                deterministic_summary_id(&project_id, &branch_id, MemoryLayerV1::Book, &scope_key);
            summaries.push(MemorySummaryV1 {
                summary_id: summary_id.clone(),
                project_id: project_id.clone(),
                branch_id: branch_id.clone(),
                layer: MemoryLayerV1::Book,
                scope_key,
                arc_id: None,
                coverage: MemoryCoverageV1 {
                    start: MemoryPositionV1::new(*book, 1, 1),
                    end: MemoryPositionV1::new(*book, last_chapter, i32::MAX),
                },
                text: truncate_chars(&text, 2200),
                child_summary_ids: vec![arc_id.clone()],
                entity_ids: entity_ids.into_iter().collect(),
                thread_ids: thread_ids.into_iter().collect(),
                source_revision_hashes: source_hashes.into_iter().collect(),
                derived_event_ids: Vec::new(),
                secret_fact_ids: secret_fact_ids.into_iter().collect(),
                tier: MemoryTierV1::Ratified,
                status: MemorySummaryStatusV1::Fresh,
                reader_visible: true,
                model: "derived-book-v1".into(),
                prompt_version: LAYERED_MEMORY_VERSION.into(),
                reducer_version: 1,
                updated_at_micros: now,
            });
            edges.push(MemoryDependencyEdgeV1 {
                summary_id,
                depends_on_summary_id: arc_id.clone(),
                source_kind: "arc".into(),
            });
            book_count += 1;
        }

        let scene_count = summaries
            .iter()
            .filter(|summary| summary.layer == MemoryLayerV1::Scene)
            .count();
        let chapter_count = summaries
            .iter()
            .filter(|summary| summary.layer == MemoryLayerV1::Chapter)
            .count();
        let arc_count = summaries
            .iter()
            .filter(|summary| summary.layer == MemoryLayerV1::Arc)
            .count();
        let dependency_count = edges.len();
        self.repository
            .replace_layered_memory(&project_id, &branch_id, summaries, edges)
            .await?;
        self.repository
            .complete_all_memory_recompute_jobs(&project_id, &branch_id)
            .await?;
        let pending_job_count = if self
            .repository
            .has_pending_memory_recompute_jobs(&project_id, &branch_id)
            .await?
        {
            1
        } else {
            0
        };
        Ok(LayeredMemoryBuildReportV1 {
            project_id,
            branch_id,
            scene_count,
            chapter_count,
            arc_count,
            book_count,
            dependency_count,
            stale_count: 0,
            pending_job_count,
        })
    }

    /// Lazy recompute entry point: rebuild the branch and clear the queued
    /// invalidation jobs. The rebuild is deterministic, so replaying it is safe.
    pub async fn recompute_layered_memory(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<LayeredMemoryBuildReportV1> {
        self.rebuild_layered_memory(project_id, branch_id).await
    }

    pub async fn rebuild_layered_memory_for_project(
        &self,
        project_id: &str,
    ) -> Result<LayeredMemoryBuildReportV1> {
        let branch = self.repository.get_active_branch(project_id).await?;
        self.rebuild_layered_memory(project_id, &branch.id).await
    }

    /// Mark the scene summary and every ancestor stale, enqueue lazy jobs, and
    /// leave the stale rows visible to callers until a rebuild runs.
    pub async fn invalidate_layered_memory_for_scene(
        &self,
        project_id: &str,
        branch_id: &str,
        scene_id: &str,
        reason: &str,
    ) -> Result<LayeredMemoryInvalidationReportV1> {
        let summaries = self
            .repository
            .list_layered_memory(project_id, branch_id)
            .await?;
        let edges = self
            .repository
            .list_layered_memory_dependencies(project_id, branch_id)
            .await?;
        let roots: Vec<String> = summaries
            .iter()
            .filter(|summary| {
                summary.layer == MemoryLayerV1::Scene
                    && summary.scope_key == format!("scene:{scene_id}")
            })
            .map(|summary| summary.summary_id.clone())
            .collect();
        let plan = plan_memory_invalidation(&roots, &summaries, &edges, reason);
        self.repository
            .invalidate_layered_memory(project_id, branch_id, &plan)
            .await?;
        let pending_job_count = if self
            .repository
            .has_pending_memory_recompute_jobs(project_id, branch_id)
            .await?
        {
            1
        } else {
            0
        };
        Ok(LayeredMemoryInvalidationReportV1 {
            plan,
            pending_job_count,
        })
    }

    pub async fn layered_memory_selection(
        &self,
        project_id: &str,
        branch_id: &str,
        placement: &StoryPlacement,
        policy: &MemoryVisibilityPolicyV1,
    ) -> Result<LayeredMemorySelectionV1> {
        let Some(cursor) = MemoryPositionV1::from_placement(placement) else {
            return Ok(LayeredMemorySelectionV1 {
                cursor: MemoryPositionV1::new(1, 1, 1),
                items: Vec::new(),
                omissions: Vec::new(),
                secret_filtered_count: 0,
                stale_count: 0,
                after_cursor_count: 0,
            });
        };
        let summaries = self
            .repository
            .list_layered_memory(project_id, branch_id)
            .await?;
        Ok(select_layered_memory(&summaries, &cursor, policy))
    }

    /// Budgeted layered context in the design order (recent scene/chapter
    /// detail before arc/book), with hard constraints reserved and explicit
    /// truncation/omission metadata.
    pub async fn layered_memory_budgeted_context(
        &self,
        project_id: &str,
        branch_id: &str,
        placement: &StoryPlacement,
        visibility: &MemoryVisibilityPolicyV1,
        budget: &MemoryBudgetPolicyV1,
    ) -> Result<MemoryBudgetedContextV1> {
        let selection = self
            .layered_memory_selection(project_id, branch_id, placement, visibility)
            .await?;
        Ok(build_memory_budgeted_context(&selection, budget))
    }

    pub async fn retry_layered_memory_jobs(
        &self,
        project_id: &str,
        branch_id: &str,
        limit: usize,
    ) -> Result<Vec<String>> {
        let jobs = self
            .repository
            .claim_memory_recompute_jobs(project_id, branch_id, limit)
            .await?;
        Ok(jobs.into_iter().map(|job| job.job_id).collect())
    }

    /// Deterministic thread retrieval with reserved class capacity and explicit
    /// overflow. Planned promise deadlines stay labeled separately.
    pub async fn retrieve_open_threads_layered(
        &self,
        project_id: &str,
        branch_id: &str,
        cursor: &StoryPlacement,
        scene_relevant_thread_ids: &BTreeSet<String>,
        policy: &ThreadRetrievalPolicyV1,
    ) -> Result<ThreadRetrievalOutputV1> {
        let cursor_index = crate::format::story_index(
            cursor.book_number,
            cursor.chapter_number,
            cursor.scene_order.unwrap_or(0),
        );
        let promises = self
            .repository
            .list_narrative_promises_by_project_and_branch(project_id, branch_id)
            .await?;
        let conflicts = self
            .repository
            .list_conflicts_by_project_and_branch(project_id, branch_id)
            .await?;
        let plots = self
            .repository
            .list_plot_lines_by_project_and_branch(project_id, branch_id)
            .await?;
        let mut threads = Vec::new();
        for promise in promises {
            let Some(status) = promise.status_at(cursor_index) else {
                continue;
            };
            let verdict = crate::format::promise_timing_verdict(&promise, cursor_index);
            let urgency = match verdict.urgency {
                crate::format::PromiseUrgency::Overdue => ThreadUrgencyV1::Overdue,
                crate::format::PromiseUrgency::Due | crate::format::PromiseUrgency::Soon => {
                    ThreadUrgencyV1::DueSoon
                }
                crate::format::PromiseUrgency::Watch => ThreadUrgencyV1::Open,
                crate::format::PromiseUrgency::Resolved => ThreadUrgencyV1::Resolved,
            };
            let due_position = promise.planned_payoff.as_ref().map(|placement| {
                MemoryPositionV1::new(
                    placement.book_number,
                    placement.chapter_number,
                    placement.scene_order.unwrap_or(0),
                )
            });
            threads.push(ThreadDigestV1 {
                thread_id: promise.id.clone(),
                kind: ThreadKindV1::Promise,
                label: truncate_chars(&promise.description, 140),
                urgency: if status == "paid_off" || status == "abandoned" {
                    ThreadUrgencyV1::Resolved
                } else {
                    urgency
                },
                due_position,
                planned: promise.planned_payoff.is_some(),
                blocked: false,
                scene_relevant: scene_relevant_thread_ids.contains(&promise.id),
                age_scene_count: verdict.chapters_since_plant.max(0) as u32,
                source_event_ids: promise
                    .status_history
                    .iter()
                    .map(|event| event.id.clone())
                    .collect(),
            });
        }
        for conflict in conflicts {
            let blocked = conflict
                .stated_consequences
                .iter()
                .any(|consequence| !consequence.delivered);
            threads.push(ThreadDigestV1 {
                thread_id: conflict.id.clone(),
                kind: ThreadKindV1::Conflict,
                label: truncate_chars(&conflict.name, 140),
                urgency: if blocked {
                    ThreadUrgencyV1::BlockingContradiction
                } else {
                    ThreadUrgencyV1::Open
                },
                due_position: None,
                planned: false,
                blocked,
                scene_relevant: scene_relevant_thread_ids.contains(&conflict.id),
                age_scene_count: 0,
                source_event_ids: Vec::new(),
            });
        }
        for plot in plots {
            threads.push(ThreadDigestV1 {
                thread_id: plot.id.clone(),
                kind: ThreadKindV1::PlotLine,
                label: truncate_chars(&plot.name, 140),
                urgency: if plot.status == "complete" {
                    ThreadUrgencyV1::Resolved
                } else {
                    ThreadUrgencyV1::Open
                },
                due_position: None,
                planned: false,
                blocked: false,
                scene_relevant: scene_relevant_thread_ids.contains(&plot.id),
                age_scene_count: 0,
                source_event_ids: Vec::new(),
            });
        }
        Ok(retrieve_threads(&threads, policy))
    }

    /// Stable-id entity retrieval from the existing assembled snapshot. It
    /// never calls an embedding provider; the direct-state lane is explicit.
    pub async fn retrieve_entity_state_layered(
        &self,
        project_id: &str,
        branch_id: &str,
        entity_id: &str,
        placement: Option<&StoryPlacement>,
    ) -> Result<EntityRetrievalV1> {
        let character = self.repository.get_character(entity_id).await?;
        if character.project_id != project_id {
            anyhow::bail!("entity does not belong to the requested project");
        }
        let placement = match placement {
            Some(placement) => placement.clone(),
            None => self
                .repository
                .list_scenes_by_project_and_branch(project_id, branch_id)
                .await?
                .last()
                .map(|scene| StoryPlacement {
                    book_number: scene.book_number,
                    chapter_number: scene.chapter_number,
                    scene_order: Some(scene.scene_order),
                    note: None,
                })
                .unwrap_or(StoryPlacement {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: Some(1),
                    note: None,
                }),
        };
        let subject = Subject::new(SubjectTable::Character, character.id.clone())
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let snapshot = self
            .repository
            .assemble_subject_snapshot(project_id, branch_id, &subject, &placement)
            .await?;
        let (names, aliases) = match snapshot.kind_specific() {
            SubjectKindSpecific::Character(details) => (
                vec![snapshot.display_name().to_string()],
                details.aliases.clone(),
            ),
            _ => (vec![snapshot.display_name().to_string()], Vec::new()),
        };
        let mut location = None;
        let mut possessions = Vec::new();
        let mut injuries = Vec::new();
        let mut knowledge = Vec::new();
        for fact in snapshot.canonical_facts() {
            let lower = fact.fact.to_lowercase();
            if lower.contains("locat") || lower.contains("resid") {
                location = Some(fact.fact.clone());
            } else if lower.contains("injur") || lower.contains("wound") {
                injuries.push(fact.fact.clone());
            } else if lower.contains("possess")
                || lower.contains("holding")
                || lower.contains("item")
            {
                possessions.push(fact.fact.clone());
            } else {
                knowledge.push(fact.fact.clone());
            }
        }
        for fact in snapshot.knowledge() {
            knowledge.push(fact.fact.clone());
        }
        let neighbors: Vec<String> = snapshot
            .relationships()
            .iter()
            .filter_map(|relationship| relationship.counterpart.clone())
            .collect();
        Ok(EntityRetrievalV1 {
            subject_id: character.id,
            names,
            aliases,
            location,
            possessions,
            injuries,
            knowledge,
            neighbors,
            source_event_ids: Vec::new(),
            provider: EntityRetrievalProviderV1::DirectState,
        })
    }
}
