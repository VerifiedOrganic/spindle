use super::*;
use crate::sqlite::repository::{legacy_claim, legacy_snapshot_claim};
use spindle_core::canon::*;

impl SqliteSpindleService {
    // Outline setters are deferred — they return spindle_core's BookOutline /
    // ChapterOutline shapes (distinct from my sqlite::records types), and the
    // conversion adds avoidable complexity that isn't on the MCP critical path
    // for first-pass swap testing. Land here in a follow-up commit.

    // =========================================================================
    // Scene mutation: delete, operator_delete + listings
    // =========================================================================

    /// Move a scene to a new (book, chapter, scene_order) slot. The
    /// SurrealDB service first checks scene_move_impact for blockers
    /// (downstream canonical facts that would invalidate) — that helper
    /// hasn't been translated yet, so this version does the bare repo
    /// call without the readiness check.
    ///
    /// Output reports whether the source position was left with a numeric
    /// gap (i.e., a later scene_order at the source position still exists).
    /// Find entities by name across the searchable surface. Uses FTS5 over
    /// the character/location/world_rule indexes for exact matches and
    /// supplements with vec0 kNN for fuzzy/semantic resolution. Simplified
    /// version of the SurrealDB resolve_subject_by_name — uses the existing
    /// search backends rather than a dedicated repository helper.
    pub async fn find_entity(&self, input: FindEntityInput) -> Result<FindEntityOutput> {
        let query = input.query.trim();
        if query.is_empty() {
            anyhow::bail!("find_entity query must not be empty");
        }
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = match input.branch_id.as_deref() {
            Some(id) => id.to_string(),
            None => project
                .active_branch_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?,
        };
        let limit = input.limit.unwrap_or(5).clamp(1, 20);

        // Collect FTS5 hits across character / location / world_rule.
        // table_filter narrows the search to a single table if specified.
        let table_filter = input.table.map(|t| t.as_str().to_string());

        let mut matches: Vec<FindEntityMatch> = Vec::new();
        if table_filter.is_none() || table_filter.as_deref() == Some("character") {
            for (id, _rank, _snippet) in self
                .repository
                .fts_search_characters(&input.project_id, Some(&branch_id), query, limit)
                .await?
            {
                matches.push(FindEntityMatch {
                    entity_id: id,
                    confidence: EntityResolutionConfidence::ExactName,
                    score: 1.0,
                });
            }
        }
        if table_filter.is_none() || table_filter.as_deref() == Some("location") {
            for (id, _rank, _snippet) in self
                .repository
                .fts_search_locations(&input.project_id, Some(&branch_id), query, limit)
                .await?
            {
                matches.push(FindEntityMatch {
                    entity_id: id,
                    confidence: EntityResolutionConfidence::ExactName,
                    score: 1.0,
                });
            }
        }
        if table_filter.is_none() || table_filter.as_deref() == Some("world_rule") {
            for (id, _rank, _snippet) in self
                .repository
                .fts_search_world_rules(&input.project_id, Some(&branch_id), query, limit)
                .await?
            {
                matches.push(FindEntityMatch {
                    entity_id: id,
                    confidence: EntityResolutionConfidence::ExactName,
                    score: 1.0,
                });
            }
        }

        // If FTS turns up no hits, fall back to vec0 semantic search.
        if matches.is_empty() {
            let embedding_session = self.repository.model_router().embedding_session()?;
            let query_embedding = embedding_session.embed_text(query).await?;
            let knn = self
                .repository
                .knn_search_embeddings(&input.project_id, Some(&branch_id), &query_embedding, limit)
                .await?;
            for (embedding, distance) in knn {
                if let Some(ref table) = table_filter
                    && &embedding.entity_table != table
                {
                    continue;
                }
                matches.push(FindEntityMatch {
                    entity_id: embedding.entity_id,
                    confidence: EntityResolutionConfidence::SemanticMatch,
                    score: (1.0 / (1.0 + distance)) as f32,
                });
            }
        }

        // Sort by score descending and cap at limit.
        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        matches.truncate(limit);
        Ok(FindEntityOutput { matches })
    }

    /// Find scenes that mention a phrase or reference a subject id. Phrase
    /// queries use FTS5; subject_id queries use a LIKE on full_text for
    /// scenes that literally contain the id string. Returns SceneReferenceItem
    /// with scene placement + snippet.
    /// Clear and rebuild the search index for a project's active branch.
    /// The SurrealDB version walked every searchable entity and re-embedded
    /// content; the SQLite version delegates the clear to the repository and
    /// re-embeds the entities it knows about — characters, locations, world
    /// rules, plus all narrative-entity tables that have search-worthy text.
    /// Set the markdown/JSON outline for a book on the active branch.
    /// Resolves the book from either book_id or book_number, upserts the
    /// (book_id, branch_id) row, and returns the public BookOutline shape
    /// with the new updated_at as RFC3339.
    pub async fn set_book_outline(
        &self,
        input: SetBookOutlineInput,
    ) -> Result<SetBookOutlineOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let book = if let Some(id) = input.book_id.as_deref() {
            self.repository.get_book(id).await?
        } else {
            self.repository
                .find_book_by_number(&input.project_id, input.book_number)
                .await?
                .ok_or_else(|| anyhow::anyhow!("book not found"))?
        };
        let format = input.format.unwrap_or_else(|| "markdown".to_string());
        let stored = self
            .repository
            .set_book_outline(&book.id, &branch_id, &format, &input.content)
            .await?;
        Ok(SetBookOutlineOutput {
            outline: BookOutline {
                book_id: stored.book_id,
                branch_id: stored.branch_id,
                format: stored.format,
                content: stored.content,
                updated_at: stored.updated_at.to_rfc3339(),
            },
        })
    }

    /// Set the chapter outline (content + beats) for a chapter on the active
    /// branch. Resolves the chapter from entity_id or (book_number,
    /// chapter_number), upserts the (chapter_id, branch_id) row, and returns
    /// the public ChapterOutline shape.
    pub async fn set_chapter_outline(
        &self,
        input: SetChapterOutlineInput,
    ) -> Result<SetChapterOutlineOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let chapter = if let Some(id) = input.entity_id.as_deref() {
            self.repository.get_chapter(id).await?
        } else {
            self.repository
                .find_chapter_by_number(&input.project_id, input.book_number, input.chapter_number)
                .await?
                .ok_or_else(|| anyhow::anyhow!("chapter not found"))?
        };
        let format = input.format.unwrap_or_else(|| "markdown".to_string());
        let content = input.content.unwrap_or_default();
        let stored = self
            .repository
            .set_chapter_outline(&chapter.id, &branch_id, &format, &content, input.beats)
            .await?;
        // stored.beats is Vec<StoredChapterOutlineBeat> — Phase 6 native
        // SQLite shape uses Option<String> for scene_id, identical to the
        // public ChapterOutlineBeat shape.
        let beats: Vec<ChapterOutlineBeat> = stored
            .beats
            .into_iter()
            .map(|b| ChapterOutlineBeat {
                order: b.order,
                summary: b.summary,
                scene_id: b.scene_id,
                status: b.status,
            })
            .collect();
        Ok(SetChapterOutlineOutput {
            outline: ChapterOutline {
                chapter_id: stored.chapter_id,
                branch_id: stored.branch_id,
                format: stored.format,
                content: stored.content,
                beats,
                updated_at: stored.updated_at.to_rfc3339(),
            },
        })
    }

    pub async fn rebuild_search_index(
        &self,
        input: RebuildSearchIndexInput,
    ) -> Result<RebuildSearchIndexOutput> {
        self.repository.get_project(&input.project_id).await?;
        let branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;

        // 1. Clear everything on the active branch.
        let _cleared = self
            .repository
            .rebuild_search_index_clear(&input.project_id)
            .await?;

        // 2. Re-embed each searchable entity. Each refresh_entity_index call
        //    upserts one row into search_embedding which the AFTER trigger
        //    mirrors into vec_search_embedding.
        let mut indexed = 0usize;

        for character in self
            .repository
            .list_characters_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            let document = character_search_document(&character);
            self.repository
                .refresh_search_embedding_for_entity(
                    &input.project_id,
                    &branch_id,
                    &character.id,
                    &document,
                )
                .await?;
            indexed += 1;
        }
        for location in self
            .repository
            .list_locations_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &location.id,
                "location",
                &location.name,
                &location.summary,
            )
            .await?;
            indexed += 1;
        }
        for rule in self
            .repository
            .list_world_rules_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &rule.id,
                "world_rule",
                &rule.rule_name,
                &rule.description,
            )
            .await?;
            indexed += 1;
        }
        for faction in self
            .repository
            .list_factions_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &faction.id,
                "faction",
                &faction.name,
                &faction.summary,
            )
            .await?;
            indexed += 1;
        }
        for plot in self
            .repository
            .list_plot_lines_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &plot.id,
                "plot_line",
                &plot.name,
                &plot.summary,
            )
            .await?;
            indexed += 1;
        }
        for conflict in self
            .repository
            .list_conflicts_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &conflict.id,
                "conflict",
                &conflict.name,
                &conflict.stakes,
            )
            .await?;
            indexed += 1;
        }
        for theme in self
            .repository
            .list_themes_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &theme.id,
                "theme",
                &theme.theme_statement,
                &theme.thesis_antithesis,
            )
            .await?;
            indexed += 1;
        }
        for motif in self
            .repository
            .list_motifs_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &motif.id,
                "motif",
                &motif.name,
                &motif.description,
            )
            .await?;
            indexed += 1;
        }
        for promise in self
            .repository
            .list_narrative_promises_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &promise.id,
                "narrative_promise",
                &promise.promise_type,
                &promise.description,
            )
            .await?;
            indexed += 1;
        }

        Ok(RebuildSearchIndexOutput {
            indexed_records: indexed,
            embedding_version: self.repository.current_embedding_version()?,
        })
    }

    pub async fn find_scenes_referencing(
        &self,
        input: FindScenesReferencingInput,
    ) -> Result<FindScenesReferencingOutput> {
        use spindle_core::models::{SceneReferenceItem, SceneReferenceQuery};
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let limit = input.limit.unwrap_or(20).min(100);
        let results = match input.query {
            SceneReferenceQuery::Phrase { phrase } => {
                let hits = self
                    .repository
                    .fts_search_scenes(&input.project_id, Some(&branch_id), &phrase, limit)
                    .await?;
                let mut items = Vec::with_capacity(hits.len());
                for (scene_id, _rank, snippet) in hits {
                    let scene = self.repository.get_scene(&scene_id).await?;
                    items.push(SceneReferenceItem {
                        scene_id,
                        book_number: scene.book_number,
                        chapter_number: scene.chapter_number,
                        scene_order: scene.scene_order,
                        snippet,
                        byte_range: None,
                    });
                }
                items
            }
            SceneReferenceQuery::Subject { subject_id } => {
                // Subject references are stored as the literal id string in
                // scene full_text. Use FTS5 since the id is a single token.
                let hits = self
                    .repository
                    .fts_search_scenes(&input.project_id, Some(&branch_id), &subject_id, limit)
                    .await?;
                let mut items = Vec::with_capacity(hits.len());
                for (scene_id, _rank, snippet) in hits {
                    let scene = self.repository.get_scene(&scene_id).await?;
                    items.push(SceneReferenceItem {
                        scene_id,
                        book_number: scene.book_number,
                        chapter_number: scene.chapter_number,
                        scene_order: scene.scene_order,
                        snippet,
                        byte_range: None,
                    });
                }
                items
            }
        };
        Ok(FindScenesReferencingOutput { results })
    }

    /// Resolve the active-branch scene a destructive tool acts on.
    ///
    /// `scene_id` takes precedence: it names the exact row and can never
    /// act on a different scene sharing a position — the hazard of
    /// position-only addressing when duplicates or orphans exist. The row
    /// must belong to the project and sit on the ACTIVE branch (these tools
    /// are documented active-branch tools); position fields, when also
    /// supplied, must agree with the row's actual placement. Position-only
    /// resolution keeps the pre-existing natural-key lookup.
    pub(super) async fn resolve_active_branch_scene(
        &self,
        project_id: &str,
        active_branch_id: &str,
        scene_id: Option<&str>,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<crate::sqlite::records::Scene> {
        match scene_id {
            Some(scene_id) => {
                let scene = self.repository.get_scene(scene_id).await?;
                if scene.project_id != project_id {
                    anyhow::bail!("scene does not belong to the requested project");
                }
                if scene.branch_id != active_branch_id {
                    anyhow::bail!(
                        "scene {} is on branch {}, not the active branch; \
                         this tool only acts on active-branch scenes",
                        scene_id,
                        scene.branch_id
                    );
                }
                if book_number > 0 && book_number != scene.book_number {
                    anyhow::bail!(
                        "scene_id {} is in book {}, which does not match book_number {}",
                        scene_id,
                        scene.book_number,
                        book_number
                    );
                }
                if chapter_number > 0 && chapter_number != scene.chapter_number {
                    anyhow::bail!(
                        "scene_id {} is in chapter {}, which does not match chapter_number {}",
                        scene_id,
                        scene.chapter_number,
                        chapter_number
                    );
                }
                if scene_order > 0 && scene_order != scene.scene_order {
                    anyhow::bail!(
                        "scene_id {} is scene {}, which does not match scene_order {}",
                        scene_id,
                        scene.scene_order,
                        scene_order
                    );
                }
                Ok(scene)
            }
            None => self
                .repository
                .find_scene_by_natural_key(
                    project_id,
                    active_branch_id,
                    book_number,
                    chapter_number,
                    scene_order,
                )
                .await?
                .ok_or_else(|| anyhow::anyhow!("scene not found at requested position")),
        }
    }

    pub async fn move_scene(&self, input: MoveSceneInput) -> Result<MoveSceneOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let scene = self
            .resolve_active_branch_scene(
                &input.project_id,
                &branch_id,
                input.scene_id.as_deref(),
                input.from_book_number,
                input.from_chapter_number,
                input.from_scene_order,
            )
            .await
            .map_err(|err| {
                // Preserve the historical message for position lookups that
                // simply miss.
                if input.scene_id.is_none()
                    && err.to_string() == "scene not found at requested position"
                {
                    anyhow::anyhow!("source scene not found")
                } else {
                    err
                }
            })?;
        // All downstream logic keys off the scene's ACTUAL placement, so a
        // scene_id-addressed move stays correct with the from_* fields
        // omitted.
        let from_book_number = scene.book_number;
        let from_chapter_number = scene.chapter_number;
        let from_scene_order = scene.scene_order;
        let dest_book = self
            .repository
            .ensure_book(&input.project_id, input.to_book_number)
            .await?;
        let dest_chapter = self
            .repository
            .ensure_chapter(
                &input.project_id,
                input.to_book_number,
                input.to_chapter_number,
            )
            .await?;
        let moved = self
            .repository
            .move_scene(
                &scene.id,
                &input.project_id,
                &branch_id,
                &dest_book.id,
                &dest_chapter.id,
                input.to_book_number,
                input.to_chapter_number,
                input.to_scene_order,
            )
            .await?;

        // Moving a scene changes its placement, so placement-keyed validator
        // findings (canonical_fact_prose_drift, retcon_reachability) for the
        // moved scene and everything at/after the earlier of the source and
        // destination positions are now stale. Resolve them so they recompute.
        let from_index =
            crate::format::story_index(from_book_number, from_chapter_number, from_scene_order);
        let to_index = crate::format::story_index(
            input.to_book_number,
            input.to_chapter_number,
            input.to_scene_order,
        );
        let (earliest_book, earliest_chapter, earliest_order) = if from_index <= to_index {
            (from_book_number, from_chapter_number, from_scene_order)
        } else {
            (
                input.to_book_number,
                input.to_chapter_number,
                input.to_scene_order,
            )
        };
        let mut affected_scene_ids = vec![moved.id.clone()];
        affected_scene_ids.extend(
            self.repository
                .list_scenes_after_position(
                    &input.project_id,
                    &branch_id,
                    earliest_book,
                    earliest_chapter,
                    earliest_order - 1,
                )
                .await?
                .into_iter()
                .map(|scene| scene.id),
        );
        self.repository
            .resolve_validator_findings_for_scenes(&branch_id, &affected_scene_ids)
            .await?;

        // Did the source position get a gap? Check whether a scene now
        // exists at from_scene_order + 1 in the source chapter.
        let later = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &branch_id,
                from_book_number,
                from_chapter_number,
                from_scene_order + 1,
            )
            .await?;
        Ok(MoveSceneOutput {
            scene_id: moved.id,
            branch_id,
            status: "moved".to_string(),
            from_book_number,
            from_chapter_number,
            from_scene_order,
            to_book_number: input.to_book_number,
            to_chapter_number: input.to_chapter_number,
            to_scene_order: input.to_scene_order,
            left_source_scene_order_gap: later.is_some(),
        })
    }

    pub async fn delete_scene(&self, input: DeleteSceneInput) -> Result<DeleteSceneOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let scene = self
            .resolve_active_branch_scene(
                &input.project_id,
                &branch_id,
                input.scene_id.as_deref(),
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
            .await?;
        // Downstream sweep and gap detection key off the row's ACTUAL
        // placement, so scene_id-addressed deletes stay correct with the
        // position fields omitted.
        let book_number = scene.book_number;
        let chapter_number = scene.chapter_number;
        let scene_order = scene.scene_order;
        let deleted_id = scene.id.clone();
        let scene_branch_id = scene.branch_id.clone();
        self.repository.delete_scene(&scene.id).await?;

        // Deleting a scene shifts the relative order of everything after it, so
        // placement-keyed validator findings for the deleted scene and the
        // scenes at/after its position are now stale. Resolve them.
        let mut affected_scene_ids = vec![scene.id.clone()];
        affected_scene_ids.extend(
            self.repository
                .list_scenes_after_position(
                    &input.project_id,
                    &branch_id,
                    book_number,
                    chapter_number,
                    scene_order,
                )
                .await?
                .into_iter()
                .map(|later_scene| later_scene.id),
        );
        self.repository
            .resolve_validator_findings_for_scenes(&branch_id, &affected_scene_ids)
            .await?;

        // Did deleting this scene leave a numeric gap? Check whether the next
        // higher scene_order still exists.
        let later = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &branch_id,
                book_number,
                chapter_number,
                scene_order + 1,
            )
            .await?;
        Ok(DeleteSceneOutput {
            scene_id: deleted_id,
            branch_id: scene_branch_id,
            status: "deleted".to_string(),
            left_scene_order_gap: later.is_some(),
        })
    }

    /// Operator-level delete — identical to delete_scene but used by the
    /// operator-only MCP tool that has fewer guard rails. For SQLite they
    /// share the same implementation.
    pub async fn operator_delete_scene(
        &self,
        input: OperatorDeleteSceneInput,
    ) -> Result<OperatorDeleteSceneOutput> {
        let standard = self
            .delete_scene(DeleteSceneInput {
                project_id: input.project_id,
                scene_id: input.scene_id,
                book_number: input.book_number,
                chapter_number: input.chapter_number,
                scene_order: input.scene_order,
            })
            .await?;
        Ok(OperatorDeleteSceneOutput {
            scene_id: standard.scene_id,
            branch_id: standard.branch_id,
            status: standard.status,
            left_scene_order_gap: standard.left_scene_order_gap,
            removed_scene_source_link_ids: Vec::new(),
            invalidated_chapter_plan_ids: Vec::new(),
            invalidated_chapter_summary_ids: Vec::new(),
        })
    }

    /// Read-only audit of every downstream reference that would be
    /// orphaned, made stale, or invalidated by deleting the scene at
    /// the requested `(book, chapter, scene_order)` on the project's
    /// active branch. Mirrors `services/mod.rs::get_scene_delete_impact`
    /// from 705b835^. Returns a structured report partitioned into
    /// hard blockers, semantic risks, and chapter artifacts, alongside
    /// an overall `delete_readiness` verdict.
    ///
    /// Relationship rows are reported via the synthetic id
    /// `relates_to:{in_id}->{out_id}` because SQLite stores relates_to
    /// rows under a composite primary key rather than a record-id
    /// column (the SurrealDB schema had an explicit `id`).
    pub async fn get_scene_delete_impact(
        &self,
        input: GetSceneDeleteImpactInput,
    ) -> Result<GetSceneDeleteImpactOutput> {
        use crate::format::{push_scene_delete_impact_group, scene_delete_placement_matches};

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let scene = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &active_branch.id,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
            .await?
            .with_context(|| {
                format!(
                    "scene not found on active branch {} at book {} chapter {} scene {}",
                    active_branch.name, input.book_number, input.chapter_number, input.scene_order
                )
            })?;

        let mut hard_blockers: Vec<SceneDeleteImpactGroup> = Vec::new();
        let mut semantic_risks: Vec<SceneDeleteImpactGroup> = Vec::new();
        let mut chapter_artifacts: Vec<SceneDeleteImpactGroup> = Vec::new();

        push_scene_delete_impact_group(
            &mut hard_blockers,
            "character_state",
            self.repository
                .list_character_states_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|state| {
                    state
                        .scene_id
                        .as_ref()
                        .is_some_and(|scene_id| scene_id == &scene.id)
                })
                .map(|state| state.id)
                .collect(),
            "Character states are committed against the exact scene id and must be removed or remapped before delete.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "revision_marker",
            self.repository
                .list_revision_markers_for_scene(&active_branch.id, &scene.id)
                .await?
                .into_iter()
                .map(|marker| marker.id)
                .collect(),
            "Revision markers point directly at this scene and would be orphaned by delete.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "dual_persona_review",
            self.repository
                .list_dual_persona_reviews_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|review| review.scene_id == scene.id)
                .map(|review| review.id)
                .collect(),
            "Dual-persona reviews are keyed to the exact scene id and need explicit cleanup.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "scene_version",
            self.repository
                .list_scene_versions(&scene.id)
                .await?
                .into_iter()
                .map(|version| version.id)
                .collect(),
            "Scene history snapshots belong to this scene id and need an explicit retention or purge policy.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "scene_beat_annotation",
            self.repository
                .list_scene_beat_annotations_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|annotation| annotation.scene_id == scene.id)
                .map(|annotation| annotation.id)
                .collect(),
            "Beat annotations point directly at this scene and would be orphaned by delete.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "canonical_fact",
            self.repository
                .list_canonical_facts_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|fact| fact.scene_id.as_deref() == Some(scene.id.as_str()))
                .map(|fact| fact.id)
                .collect(),
            "Canonical facts cite this scene as their source and must be removed, superseded, or re-sourced.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "scene_source_link",
            self.repository
                .list_scene_source_links_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|link| link.scene_id == scene.id)
                .map(|link| link.id)
                .collect(),
            "Source links point directly at this scene id and would become dangling references.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "relationship_last_scene",
            self.repository
                .list_relationships_by_branch(&active_branch.id)
                .await?
                .into_iter()
                .filter(|relationship| {
                    relationship
                        .last_scene_id
                        .as_ref()
                        .is_some_and(|scene_id| scene_id == &scene.id)
                })
                .map(|relationship| {
                    format!("relates_to:{}->{}", relationship.in_id, relationship.out_id)
                })
                .collect(),
            "Relationship recency is anchored to this scene id and would need repair before delete.",
        );

        push_scene_delete_impact_group(
            &mut semantic_risks,
            "narrative_promise_planted_at",
            self.repository
                .list_narrative_promises_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|promise| scene_delete_placement_matches(&promise.planted_at, &scene))
                .map(|promise| promise.id)
                .collect(),
            "Narrative promises planted at this story position would become semantically stale after delete.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "narrative_promise_planned_payoff",
            self.repository
                .list_narrative_promises_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|promise| {
                    promise
                        .planned_payoff
                        .as_ref()
                        .is_some_and(|placement| scene_delete_placement_matches(placement, &scene))
                })
                .map(|promise| promise.id)
                .collect(),
            "Planned promise payoffs scheduled at this scene would need manual repositioning.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "future_knowledge_learned_at",
            self.repository
                .list_future_knowledge_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|knowledge| scene_delete_placement_matches(&knowledge.learned_at, &scene))
                .map(|knowledge| knowledge.id)
                .collect(),
            "Future-knowledge acquisition tied to this scene position would become stale.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "future_knowledge_expires_at",
            self.repository
                .list_future_knowledge_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|knowledge| {
                    knowledge
                        .expires_at
                        .as_ref()
                        .is_some_and(|placement| scene_delete_placement_matches(placement, &scene))
                })
                .map(|knowledge| knowledge.id)
                .collect(),
            "Future-knowledge expiry anchored to this scene would need manual repositioning.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "timeline_event_placement",
            self.repository
                .list_timeline_events_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|event| scene_delete_placement_matches(&event.placement, &scene))
                .map(|event| event.id)
                .collect(),
            "Timeline events placed at this scene would become semantically wrong after delete.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "character_arc_milestone",
            self.repository
                .list_character_arcs_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|arc| {
                    arc.milestones.iter().any(|milestone| {
                        milestone.placement.as_ref().is_some_and(|placement| {
                            scene_delete_placement_matches(placement, &scene)
                        })
                    })
                })
                .map(|arc| arc.id)
                .collect(),
            "Character-arc milestones scheduled at this scene would need manual repositioning.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "plot_line_convergence_point",
            self.repository
                .list_plot_lines_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|plot_line| {
                    plot_line
                        .convergence_points
                        .iter()
                        .any(|placement| scene_delete_placement_matches(placement, &scene))
                })
                .map(|plot_line| plot_line.id)
                .collect(),
            "Plot-line convergence points anchored to this scene would become stale.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "theme_introduction_point",
            self.repository
                .list_themes_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|theme| {
                    theme
                        .introduction_point
                        .as_ref()
                        .is_some_and(|placement| scene_delete_placement_matches(placement, &scene))
                })
                .map(|theme| theme.id)
                .collect(),
            "Theme introductions placed at this scene would need manual repositioning.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "theme_resolution_point",
            self.repository
                .list_themes_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|theme| {
                    theme
                        .resolution_point
                        .as_ref()
                        .is_some_and(|placement| scene_delete_placement_matches(placement, &scene))
                })
                .map(|theme| theme.id)
                .collect(),
            "Theme resolutions placed at this scene would need manual repositioning.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "conflict_stated_consequence",
            self.repository
                .list_conflicts_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|conflict| {
                    conflict.stated_consequences.iter().any(|consequence| {
                        consequence.stated_at.as_ref().is_some_and(|placement| {
                            scene_delete_placement_matches(placement, &scene)
                        })
                    })
                })
                .map(|conflict| conflict.id)
                .collect(),
            "Conflict consequences first stated at this scene would become semantically stale.",
        );

        push_scene_delete_impact_group(
            &mut chapter_artifacts,
            "chapter_plan_scene",
            self.repository
                .list_chapter_plans_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|plan| {
                    plan.book_number == scene.book_number
                        && plan.chapter_number == scene.chapter_number
                        && plan
                            .scenes
                            .iter()
                            .any(|planned_scene| planned_scene.scene_order == scene.scene_order)
                })
                .map(|plan| plan.id)
                .collect(),
            "Chapter plans that still target this scene order would need manual refresh.",
        );
        push_scene_delete_impact_group(
            &mut chapter_artifacts,
            "chapter_summary",
            self.repository
                .list_chapter_summaries_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|summary| {
                    summary.book_number == scene.book_number
                        && summary.chapter_number == scene.chapter_number
                })
                .map(|summary| summary.id)
                .collect(),
            "Chapter summaries for the affected chapter would become stale after delete.",
        );

        let delete_readiness = if !hard_blockers.is_empty() {
            SceneDeleteReadiness::Blocked
        } else if !semantic_risks.is_empty() || !chapter_artifacts.is_empty() {
            SceneDeleteReadiness::NeedsFollowup
        } else {
            SceneDeleteReadiness::Clear
        };

        let mut notes = vec![
            "This is a read-only audit against the current active-branch view.".to_string(),
            "The audit assumes scene deletion leaves a gap in scene_order; later scene positions are not auto-renumbered.".to_string(),
        ];
        if active_branch.name != "main" {
            notes.push(
                "Active-branch fallback records inherited from main can appear here because they remain visible in the current branch view."
                    .to_string(),
            );
        }
        if delete_readiness == SceneDeleteReadiness::Clear {
            notes.push(
                "No direct blockers or placement-based stale records were detected for this scene."
                    .to_string(),
            );
        }

        Ok(GetSceneDeleteImpactOutput {
            active_branch_id: active_branch.id.clone(),
            active_branch_name: active_branch.name.clone(),
            scene: SceneDeleteImpactTarget {
                scene_id: scene.id.clone(),
                branch_id: scene.branch_id.clone(),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: scene.scene_order,
                summary: scene.summary,
            },
            delete_readiness,
            hard_blockers,
            semantic_risks,
            chapter_artifacts,
            notes,
        })
    }

    /// Read-only audit of every downstream reference that would be
    /// orphaned, made stale, or invalidated by moving the scene at
    /// `(from_book, from_chapter, from_scene_order)` to
    /// `(to_book, to_chapter, to_scene_order)` on the project's active
    /// branch. Mirrors `services/mod.rs::get_scene_move_impact` from
    /// 705b835^.
    ///
    /// Mechanically reuses `get_scene_delete_impact` as the source-side
    /// dependency probe: anything that's a delete-blocker at the source
    /// is also a move-blocker (the scene id stays the same, but its
    /// position changes). Each delete group is then re-narrated into a
    /// move-shaped reason via the helpers in `crate::format`. The
    /// destination-occupancy check fires when another scene already
    /// occupies the requested target position; the chapter-artifact
    /// probes fire for both the source and destination chapters when
    /// the move crosses chapters.
    pub async fn get_scene_move_impact(
        &self,
        input: GetSceneMoveImpactInput,
    ) -> Result<GetSceneMoveImpactOutput> {
        use crate::format::{
            push_scene_move_impact_group, scene_move_hard_blocker_from_delete_group,
            scene_move_semantic_risk_from_delete_group,
        };

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let source_scene = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &active_branch.id,
                input.from_book_number,
                input.from_chapter_number,
                input.from_scene_order,
            )
            .await?
            .with_context(|| {
                format!(
                    "scene not found on active branch {} at book {} chapter {} scene {}",
                    active_branch.name,
                    input.from_book_number,
                    input.from_chapter_number,
                    input.from_scene_order
                )
            })?;

        let scene = SceneDeleteImpactTarget {
            scene_id: source_scene.id.clone(),
            branch_id: source_scene.branch_id.clone(),
            book_number: source_scene.book_number,
            chapter_number: source_scene.chapter_number,
            scene_order: source_scene.scene_order,
            summary: source_scene.summary.clone(),
        };

        if input.from_book_number == input.to_book_number
            && input.from_chapter_number == input.to_chapter_number
            && input.from_scene_order == input.to_scene_order
        {
            return Ok(GetSceneMoveImpactOutput {
                active_branch_id: active_branch.id.clone(),
                active_branch_name: active_branch.name.clone(),
                scene: scene.clone(),
                destination: SceneMoveImpactDestination {
                    book_number: input.to_book_number,
                    chapter_number: input.to_chapter_number,
                    scene_order: input.to_scene_order,
                    existing_scene_id: Some(scene.scene_id.clone()),
                    existing_summary: Some(scene.summary.clone()),
                },
                move_readiness: SceneMoveReadiness::Clear,
                hard_blockers: vec![],
                semantic_risks: vec![],
                chapter_artifacts: vec![],
                notes: vec![
                    "This is a read-only audit against the current active-branch view.".to_string(),
                    "Source and destination are identical; no move would be performed.".to_string(),
                ],
            });
        }

        let source_delete_impact = self
            .get_scene_delete_impact(GetSceneDeleteImpactInput {
                project_id: input.project_id.clone(),
                book_number: input.from_book_number,
                chapter_number: input.from_chapter_number,
                scene_order: input.from_scene_order,
            })
            .await?;
        let destination_scene = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &active_branch.id,
                input.to_book_number,
                input.to_chapter_number,
                input.to_scene_order,
            )
            .await?;

        let mut hard_blockers: Vec<SceneMoveImpactGroup> = source_delete_impact
            .hard_blockers
            .into_iter()
            .map(scene_move_hard_blocker_from_delete_group)
            .collect();
        if let Some(destination_scene) = destination_scene.as_ref()
            && destination_scene.id != source_scene.id
        {
            push_scene_move_impact_group(
                &mut hard_blockers,
                "destination_scene",
                vec![destination_scene.id.clone()],
                "Another scene already occupies the requested destination; a safe move would need a full position rebase rather than an overwrite.",
            );
        }

        let semantic_risks: Vec<SceneMoveImpactGroup> = source_delete_impact
            .semantic_risks
            .into_iter()
            .map(scene_move_semantic_risk_from_delete_group)
            .collect();

        let mut chapter_artifacts: Vec<SceneMoveImpactGroup> = Vec::new();
        push_scene_move_impact_group(
            &mut chapter_artifacts,
            "source_chapter_plan",
            self.repository
                .list_chapter_plans_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|plan| {
                    plan.book_number == input.from_book_number
                        && plan.chapter_number == input.from_chapter_number
                })
                .map(|plan| plan.id)
                .collect(),
            "Chapter plans for the source chapter would become stale after moving a scene out of that chapter position.",
        );
        push_scene_move_impact_group(
            &mut chapter_artifacts,
            "source_chapter_summary",
            self.repository
                .list_chapter_summaries_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|summary| {
                    summary.book_number == input.from_book_number
                        && summary.chapter_number == input.from_chapter_number
                })
                .map(|summary| summary.id)
                .collect(),
            "Chapter summaries for the source chapter would need refresh after a scene move.",
        );
        if input.from_book_number != input.to_book_number
            || input.from_chapter_number != input.to_chapter_number
        {
            push_scene_move_impact_group(
                &mut chapter_artifacts,
                "destination_chapter_plan",
                self.repository
                    .list_chapter_plans_by_project(&input.project_id)
                    .await?
                    .into_iter()
                    .filter(|plan| {
                        plan.book_number == input.to_book_number
                            && plan.chapter_number == input.to_chapter_number
                    })
                    .map(|plan| plan.id)
                    .collect(),
                "Chapter plans for the destination chapter would become stale after inserting a moved scene there.",
            );
            push_scene_move_impact_group(
                &mut chapter_artifacts,
                "destination_chapter_summary",
                self.repository
                    .list_chapter_summaries_by_project(&input.project_id)
                    .await?
                    .into_iter()
                    .filter(|summary| {
                        summary.book_number == input.to_book_number
                            && summary.chapter_number == input.to_chapter_number
                    })
                    .map(|summary| summary.id)
                    .collect(),
                "Chapter summaries for the destination chapter would need refresh after a scene move.",
            );
        }

        let move_readiness = if !hard_blockers.is_empty() {
            SceneMoveReadiness::Blocked
        } else if !semantic_risks.is_empty() || !chapter_artifacts.is_empty() {
            SceneMoveReadiness::NeedsFollowup
        } else {
            SceneMoveReadiness::Clear
        };

        let mut notes = vec![
            "This is a read-only audit against the current active-branch view.".to_string(),
            "The audit does not move or renumber scenes; it only reports the visible dependency and destination-conflict surface.".to_string(),
            "Only audit-clear moves are safe for the current tool; anything else still requires a dependency-aware position rebase rather than a direct scene_order rewrite.".to_string(),
        ];
        if active_branch.name != "main" {
            notes.push(
                "Active-branch fallback records inherited from main can appear here because they remain visible in the current branch view."
                    .to_string(),
            );
        }
        if move_readiness == SceneMoveReadiness::Clear {
            notes.push(
                "No direct blockers, destination conflicts, or chapter-level stale records were detected for this move."
                    .to_string(),
            );
        }

        Ok(GetSceneMoveImpactOutput {
            active_branch_id: active_branch.id.clone(),
            active_branch_name: active_branch.name.clone(),
            scene,
            destination: SceneMoveImpactDestination {
                book_number: input.to_book_number,
                chapter_number: input.to_chapter_number,
                scene_order: input.to_scene_order,
                existing_scene_id: destination_scene.as_ref().map(|s| s.id.clone()),
                existing_summary: destination_scene.map(|s| s.summary),
            },
            move_readiness,
            hard_blockers,
            semantic_risks,
            chapter_artifacts,
            notes,
        })
    }

    pub async fn list_chapter_scenes(
        &self,
        input: ListChapterScenesInput,
    ) -> Result<ListChapterScenesOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;

        // Resolve the chapter row from either chapter_id or (book_number, chapter_number).
        let chapter = if let Some(id) = input.chapter_id.as_deref() {
            self.repository.get_chapter(id).await?
        } else {
            self.repository
                .find_chapter_by_number(&input.project_id, input.book_number, input.chapter_number)
                .await?
                .ok_or_else(|| anyhow::anyhow!("chapter not found"))?
        };
        // The spine read shares ONE branch-scoped predicate with
        // `compile_manuscript` and the position resolver
        // (`Repository::list_scenes_by_chapter_and_branch`). A chapter_id-only
        // read here used to leak other branches' variant rows into the listing
        // — the live bug that showed three scenes at one `scene_order` and led
        // an operator to position-delete the real scene.
        let scenes = self
            .repository
            .list_scenes_by_chapter_and_branch(&chapter.id, &branch_id)
            .await?;
        let scene_entries = scenes
            .into_iter()
            .map(|s| spindle_core::models::SceneSpineEntry {
                scene_id: s.id,
                scene_order: s.scene_order,
                word_count: s.full_text.split_whitespace().count(),
                summary_first_line: s.summary.lines().next().unwrap_or("").to_string(),
                has_canonical_facts: false, // Set in a later commit that joins canonical_fact.
                content_rating: match s.content_rating.as_str() {
                    "General" => spindle_core::models::ContentRating::General,
                    "Teen" => spindle_core::models::ContentRating::Teen,
                    "Mature" => spindle_core::models::ContentRating::Mature,
                    "Explicit" => spindle_core::models::ContentRating::Explicit,
                    _ => spindle_core::models::ContentRating::General,
                },
                tone: s.tone,
            })
            .collect();
        Ok(ListChapterScenesOutput {
            project_id: input.project_id.clone(),
            branch_id: branch_id.clone(),
            book_id: chapter.book_id,
            chapter_id: chapter.id.clone(),
            book_number: chapter.book_number,
            chapter_number: chapter.chapter_number,
            title: chapter.title,
            scenes: scene_entries,
            unresolved_alternatives: self
                .unresolved_alternatives_for_chapter(&input.project_id, &chapter.id, &branch_id)
                .await?,
        })
    }

    /// Scenes in this chapter that live on `alternative`-type branches other
    /// than the active branch — the never-promoted output of
    /// `generate_alternatives`. Reported beside the spine listing so the
    /// operator can see abandoned alternatives without those rows ever being
    /// mistaken for spine scenes (they are not rendered by
    /// `compile_manuscript` and not addressable by position).
    pub(super) async fn unresolved_alternatives_for_chapter(
        &self,
        project_id: &str,
        chapter_id: &str,
        active_branch_id: &str,
    ) -> Result<Vec<spindle_core::models::UnresolvedAlternativeEntry>> {
        let alternative_branches: std::collections::BTreeMap<String, String> = self
            .repository
            .list_branches_by_project(project_id)
            .await?
            .into_iter()
            .filter(|branch| branch.branch_type.as_deref() == Some("alternative"))
            .map(|branch| (branch.id, branch.name))
            .collect();
        let mut entries = Vec::new();
        for scene in self
            .repository
            .list_scenes_by_chapter(chapter_id)
            .await?
            .into_iter()
            .filter(|scene| scene.branch_id != active_branch_id)
        {
            let Some(branch_name) = alternative_branches.get(&scene.branch_id) else {
                continue;
            };
            entries.push(spindle_core::models::UnresolvedAlternativeEntry {
                scene_id: scene.id,
                branch_id: scene.branch_id,
                branch_name: branch_name.clone(),
                scene_order: scene.scene_order,
                word_count: scene.full_text.split_whitespace().count(),
                summary_first_line: scene.summary.lines().next().unwrap_or("").to_string(),
            });
        }
        entries.sort_by(|a, b| {
            a.scene_order
                .cmp(&b.scene_order)
                .then_with(|| a.scene_id.cmp(&b.scene_id))
        });
        Ok(entries)
    }

    pub async fn list_book_chapters(
        &self,
        input: ListBookChaptersInput,
    ) -> Result<ListBookChaptersOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let book = if let Some(id) = input.book_id.as_deref() {
            self.repository.get_book(id).await?
        } else {
            self.repository
                .find_book_by_number(&input.project_id, input.book_number)
                .await?
                .ok_or_else(|| anyhow::anyhow!("book not found"))?
        };
        let chapters = self.repository.list_chapters_by_book(&book.id).await?;
        let mut chapter_entries = Vec::with_capacity(chapters.len());
        for ch in chapters {
            // Same branch-scoped spine predicate as `list_chapter_scenes` —
            // the book spine must never show other branches' variant rows.
            let scenes = self
                .repository
                .list_scenes_by_chapter_and_branch(&ch.id, &branch_id)
                .await?;
            chapter_entries.push(spindle_core::models::ChapterSpineEntry {
                chapter_id: ch.id,
                chapter_number: ch.chapter_number,
                title: ch.title,
                scene_count: scenes.len(),
                scenes: scenes
                    .into_iter()
                    .map(|s| spindle_core::models::SceneSpineEntry {
                        scene_id: s.id,
                        scene_order: s.scene_order,
                        word_count: s.full_text.split_whitespace().count(),
                        summary_first_line: s.summary.lines().next().unwrap_or("").to_string(),
                        has_canonical_facts: false,
                        content_rating: match s.content_rating.as_str() {
                            "General" => spindle_core::models::ContentRating::General,
                            "Teen" => spindle_core::models::ContentRating::Teen,
                            "Mature" => spindle_core::models::ContentRating::Mature,
                            "Explicit" => spindle_core::models::ContentRating::Explicit,
                            _ => spindle_core::models::ContentRating::General,
                        },
                        tone: s.tone,
                    })
                    .collect(),
            });
        }
        Ok(ListBookChaptersOutput {
            project_id: input.project_id,
            branch_id,
            book_id: book.id,
            book_number: book.book_number,
            title: book.title,
            chapters: chapter_entries,
        })
    }

    /// Register a canonical fact, optionally superseding a prior fact.
    /// Maps the wide RegisterCanonicalFactInput shape onto the repository's
    /// CreateCanonicalFactParams. Defaults to scope=Invariant, value_kind
    /// derived from which value field is set.
    pub async fn register_canonical_fact(
        &self,
        input: RegisterCanonicalFactInput,
    ) -> Result<RegisterCanonicalFactOutput> {
        self.repository.get_project(&input.project_id).await?;
        // scene_id is optional (V0038): when present the fact is anchored to
        // its dramatising scene (and inherits that scene's branch); when
        // absent the fact is planned-and-pending, placed by book/chapter only
        // on the project's active branch, to be bound later via
        // bind_canonical_fact_to_scene.
        let (branch_id, scene_id) = match input.scene_id.as_deref() {
            Some(scene_id) => {
                let scene = self.repository.get_scene(scene_id).await?;
                if scene.project_id != input.project_id {
                    anyhow::bail!("scene does not belong to the requested project");
                }
                let effective_branch_id = self
                    .repository
                    .resolve_effective_scene_branch(&input.project_id, &scene.branch_id)
                    .await?;
                (effective_branch_id, Some(scene.id.clone()))
            }
            None => (
                self.repository
                    .active_branch_id_public(&input.project_id)
                    .await?,
                None,
            ),
        };
        // Pick value_kind based on which value field is populated.
        let value_kind = input.value_kind.clone().unwrap_or_else(|| {
            if input.value_number.is_some() {
                "number".to_string()
            } else if input.value_json.is_some() {
                "json".to_string()
            } else {
                "string".to_string()
            }
        });
        let predicate = input
            .predicate
            .clone()
            .or_else(|| input.key.clone())
            .ok_or_else(|| anyhow::anyhow!("predicate or key is required"))?;
        let subject_table = input
            .subject_table
            .clone()
            .unwrap_or_else(|| "project".to_string());
        let scope = match input.scope {
            Some(CanonicalFactScope::Invariant) | None => "invariant",
            Some(CanonicalFactScope::Evolving) => "evolving",
            Some(CanonicalFactScope::Conditional) => "conditional",
        }
        .to_string();
        let params = crate::sqlite::repository::CreateCanonicalFactParams {
            project_id: input.project_id.clone(),
            branch_id,
            scene_id,
            book_number: input.book_number,
            chapter_number: input.chapter_number,
            subject_table,
            subject_id: input.subject_id,
            predicate,
            value_kind,
            value_text: input.value_text.or(input.value.clone()),
            value_number: input.value_number,
            unit: input.value_unit,
            value_json: input.value_json,
            aliases: input.aliases,
            scope,
            valid_from: input.valid_from,
            valid_until: input.valid_until,
            legacy_untyped: input.legacy_untyped.unwrap_or(false),
            secret: input.secrecy.is_some(),
            concealment_note: input
                .secrecy
                .as_ref()
                .and_then(|s| s.concealment_note.clone()),
        };
        let secrecy = input.secrecy.clone();
        let supersedes_fact_id = input.supersedes_fact_id.clone();
        let mirror_subject_id = params
            .subject_id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("project:{}", params.project_id));
        let mirror_predicate = params.predicate.clone();
        let mirror_scene_id = params.scene_id.clone();
        let mirror_placement = StoryPlacement {
            book_number: params.book_number,
            chapter_number: params.chapter_number,
            scene_order: None,
            note: None,
        };
        let mirror_revision_id = format!(
            "legacy:canonical_fact:{}:{}:{}",
            params.branch_id, mirror_subject_id, mirror_predicate
        );
        let mirror_nature = match params.scope.as_str() {
            "invariant" => AttributeNature::Invariant,
            "evolving" => AttributeNature::Evolving,
            "conditional" => AttributeNature::Conditional,
            _ => AttributeNature::Unknown,
        };
        let mirror_value =
            if let Some(number) = params.value_number.filter(|value| value.is_finite()) {
                Some(ClaimScalar::Decimal {
                    value: number.to_string(),
                    unit: params.unit.clone(),
                })
            } else if let Some(serde_json::Value::Bool(value)) = &params.value_json {
                Some(ClaimScalar::Boolean { value: *value })
            } else {
                params
                    .value_text
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| ClaimScalar::Text { value })
            };
        let (canonical_fact_id, superseded_fact_id) = self
            .repository
            .with_transaction(move |tx| {
                let fact = tx.create_canonical_fact(params)?;
                let mut mirror_claims = Vec::new();
                let snapshot_index = mirror_claims.len();
                mirror_claims.push(legacy_snapshot_claim(
                    &fact.project_id,
                    &fact.branch_id,
                    &mirror_revision_id,
                    mirror_scene_id.as_deref(),
                    &mirror_placement,
                    &mirror_subject_id,
                    format!("legacy.canonical_fact.{}", fact.id),
                    &fact,
                    Some(0),
                    snapshot_index,
                )?);
                if let Some(value) = mirror_value {
                    let index = mirror_claims.len();
                    mirror_claims.push(legacy_claim(
                        &fact.project_id,
                        &fact.branch_id,
                        &mirror_revision_id,
                        mirror_scene_id.as_deref(),
                        &mirror_placement,
                        ClaimEntity::Resolved {
                            id: mirror_subject_id,
                        },
                        ClaimAssertion::Attribute {
                            operation: AttributeOperation::Set,
                            key: mirror_predicate,
                            value,
                            nature: mirror_nature,
                        },
                        Some(0),
                        index,
                    ));
                }
                if let Some(secrecy) = secrecy.as_ref() {
                    let fact_text = crate::format::canonical_fact_hard_constraint(&fact).statement;
                    let normalized_fact = spindle_core::models::normalize_name(&fact_text);
                    for holder_id in &secrecy.holder_ids {
                        tx.link_secret_holder(crate::sqlite::repository::LinkSecretHolderParams {
                            project_id: fact.project_id.clone(),
                            branch_id: fact.branch_id.clone(),
                            character_id: holder_id.clone(),
                            fact_text: fact_text.clone(),
                            normalized_fact: normalized_fact.clone(),
                            source_summary: "secret declared".to_string(),
                            secret_of_fact_id: fact.id.clone(),
                        })?;
                    }
                }
                let superseded_fact_id = match supersedes_fact_id {
                    Some(old) => {
                        tx.supersede_canonical_fact(&fact.branch_id, &old, &fact.id)?;
                        Some(old)
                    }
                    None => None,
                };
                let mut keys = vec![("canonical_fact".to_string(), fact.id.clone())];
                if let Some(old) = &superseded_fact_id {
                    let old_fact = tx.get_canonical_fact(old)?;
                    let old_index = mirror_claims.len();
                    mirror_claims.push(legacy_snapshot_claim(
                        &fact.project_id,
                        &fact.branch_id,
                        &mirror_revision_id,
                        mirror_scene_id.as_deref(),
                        &mirror_placement,
                        old_fact
                            .subject_id
                            .as_deref()
                            .unwrap_or(fact.subject_id.as_deref().unwrap_or("project")),
                        format!("legacy.canonical_fact.{}", old_fact.id),
                        &old_fact,
                        Some(0),
                        old_index,
                    )?);
                    keys.push(("canonical_fact".to_string(), old.clone()));
                }
                let mirrored = tx.mirror_canon_claims(
                    &fact.project_id,
                    &fact.branch_id,
                    &mirror_revision_id,
                    mirror_scene_id.as_deref(),
                    mirror_claims,
                )?;
                if mirrored.is_some() {
                    tx.mark_canon_source_rows_mapped(&fact.project_id, &fact.branch_id, keys)?;
                }
                tx.resolve_validator_findings_for_validator(
                    &fact.project_id,
                    &fact.branch_id,
                    PhaseFourCacheId::CanonicalFactProseDrift.as_str(),
                )?;
                Ok((fact.id, superseded_fact_id))
            })
            .await?;
        Ok(RegisterCanonicalFactOutput {
            canonical_fact_id,
            superseded_fact_id,
        })
    }

    /// Attach a planned-and-pending canonical fact (registered without a
    /// scene, V0038) to the scene that dramatises it. The fact's book/chapter
    /// placement is untouched — binding only records where the fact was
    /// established, so authors can bind even if the scene landed in a
    /// different slot than originally planned.
    pub async fn bind_canonical_fact_to_scene(
        &self,
        input: BindCanonicalFactToSceneInput,
    ) -> Result<BindCanonicalFactToSceneOutput> {
        let fact = self
            .repository
            .get_canonical_fact(&input.canonical_fact_id)
            .await?;
        if fact.superseded_by.is_some() {
            anyhow::bail!(
                "canonical fact {} is superseded; bind the live fact, not its history",
                input.canonical_fact_id
            );
        }
        let scene = self.repository.get_scene(&input.scene_id).await?;
        if scene.project_id != fact.project_id {
            anyhow::bail!("scene does not belong to the fact's project");
        }
        let effective_scene_branch = self
            .repository
            .resolve_effective_scene_branch(&fact.project_id, &scene.branch_id)
            .await?;
        if scene.branch_id != fact.branch_id && effective_scene_branch != fact.branch_id {
            anyhow::bail!("scene is on a different branch than the fact");
        }
        self.repository
            .bind_canonical_fact_to_scene(&fact.id, &scene.id)
            .await?;
        // The establishing-scene skip in the prose-drift validator changed.
        self.resolve_phase_four_caches(
            &fact.project_id,
            &fact.branch_id,
            &[PhaseFourCacheId::CanonicalFactProseDrift],
        )
        .await?;
        Ok(BindCanonicalFactToSceneOutput {
            canonical_fact_id: fact.id,
            scene_id: scene.id,
        })
    }

    pub async fn list_scene_versions(
        &self,
        input: ListSceneVersionsInput,
    ) -> Result<ListSceneVersionsOutput> {
        self.repository.get_project(&input.project_id).await?;
        let versions = self.repository.list_scene_versions(&input.scene_id).await?;
        let summaries = versions
            .into_iter()
            .map(|v| SceneVersionSummary {
                scene_version_id: v.id,
                version_number: v.version_number,
                saved_at: v.created_at.to_rfc3339(),
                word_count: v.full_text.split_whitespace().count(),
                summary: v.summary,
            })
            .collect();
        Ok(ListSceneVersionsOutput {
            scene_id: input.scene_id,
            versions: summaries,
        })
    }

    pub async fn restore_scene_version(
        &self,
        input: RestoreSceneVersionInput,
    ) -> Result<RestoreSceneVersionOutput> {
        self.repository.get_project(&input.project_id).await?;
        let version = self
            .repository
            .get_scene_version(&input.scene_version_id)
            .await?;
        let scene = self
            .repository
            .restore_scene_version_and_mark_reviews_stale(&input.scene_id, &version)
            .await?;
        Ok(RestoreSceneVersionOutput {
            scene_id: scene.id,
            restored_from_version_id: version.id,
            restored_version_number: version.version_number,
            status: "restored".to_string(),
        })
    }

    pub async fn list_revision_markers(
        &self,
        input: ListRevisionMarkersInput,
    ) -> Result<ListRevisionMarkersOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let markers = self
            .repository
            .list_revision_markers_for_scene(&branch_id, &input.scene_id)
            .await?;
        let summaries = markers
            .into_iter()
            .map(|m| spindle_core::models::RevisionMarkerSummary {
                marker_id: m.id,
                marker_type: m.marker_type,
                target_record_id: m.target_record_id,
                position: m.position,
                note: m.note,
                status: m.status,
            })
            .collect();
        Ok(ListRevisionMarkersOutput {
            scene_id: input.scene_id,
            markers: summaries,
        })
    }
}
