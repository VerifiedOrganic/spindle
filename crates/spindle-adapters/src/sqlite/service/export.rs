use super::*;

impl SqliteSpindleService {
    /// Run the pre-publish sanity check used before `export_epub` /
    /// `export_bible`. Returns a structured list of blocking issues
    /// (missing scenes, duplicate scene_order, empty scene text) and
    /// warnings (missing chapter title). Mirrors the reference contract
    /// exactly so MCP clients see identical issue codes and severities.
    pub async fn preflight_book_export(
        &self,
        input: spindle_core::models::PreflightBookExportInput,
    ) -> Result<spindle_core::models::PreflightBookExportOutput> {
        use spindle_core::models::PreflightBookExportOutput;

        self.repository.get_project(&input.project_id).await?;
        let chapter_scope = resolve_export_chapter_scope(
            input.book_number,
            input.start_chapter_number,
            input.end_chapter_number,
        )?;
        let issues = self
            .validate_book_for_export(&input.project_id, input.book_number, chapter_scope)
            .await?;
        Ok(PreflightBookExportOutput {
            project_id: input.project_id,
            book_number: input.book_number,
            start_chapter_number: input.start_chapter_number,
            end_chapter_number: input.end_chapter_number,
            issues,
        })
    }

    /// Shared book-export validator. Returns the structured issue list
    /// used by both `preflight_book_export` (which surfaces it directly)
    /// and `export_epub` / `export_bible` (which gate publishing on the
    /// presence of any Blocking entries). Mirrors the SurrealDB reference
    /// `validate_book_for_export` at services/mod.rs:1023..1139 in 705b835^.
    pub(super) async fn validate_book_for_export(
        &self,
        project_id: &str,
        book_number: Option<i32>,
        chapter_scope: Option<ExportChapterScope>,
    ) -> Result<Vec<spindle_core::models::ExportIssue>> {
        use spindle_core::models::{ExportIssue, ExportIssueSeverity};
        use std::collections::BTreeSet;

        let project = self.repository.get_project(project_id).await?;
        let min_scene_words = project
            .min_scene_word_count
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_MIN_SCENE_WORD_COUNT);

        let all_books = self.repository.list_books_by_project(project_id).await?;
        let books_to_export: Vec<_> = match book_number {
            Some(number) => all_books
                .iter()
                .filter(|book| book.book_number == number)
                .collect(),
            None => all_books.iter().collect(),
        };
        if books_to_export.is_empty() {
            anyhow::bail!("no books found for export");
        }

        let active_branch = self.repository.get_active_branch(project_id).await?;
        let all_scenes = self
            .repository
            .list_scenes_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut issues = Vec::new();

        for book in books_to_export {
            let chapters = self
                .repository
                .list_chapters_by_book(&book.id)
                .await?
                .into_iter()
                .filter(|chapter| {
                    export_scope_contains_chapter(chapter_scope, chapter.chapter_number)
                })
                .collect::<Vec<_>>();

            if chapters.is_empty() {
                anyhow::bail!(
                    "no chapters found for export in book {} for the requested range",
                    book.book_number
                );
            }

            for chapter in chapters {
                let chapter_scenes: Vec<_> = all_scenes
                    .iter()
                    .filter(|scene| {
                        scene.book_number == book.book_number
                            && scene.chapter_number == chapter.chapter_number
                    })
                    .collect();

                if chapter_scenes.is_empty() {
                    issues.push(ExportIssue {
                        severity: ExportIssueSeverity::Blocking,
                        code: "chapter_without_scenes".to_string(),
                        message: format!(
                            "Book {} chapter {} has no scenes.",
                            book.book_number, chapter.chapter_number
                        ),
                        book_number: Some(book.book_number),
                        chapter_number: Some(chapter.chapter_number),
                        scene_order: None,
                    });
                    continue;
                }

                if chapter
                    .title
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(str::is_empty)
                {
                    issues.push(ExportIssue {
                        severity: ExportIssueSeverity::Warning,
                        code: "chapter_missing_title".to_string(),
                        message: format!(
                            "Book {} chapter {} does not have a title.",
                            book.book_number, chapter.chapter_number
                        ),
                        book_number: Some(book.book_number),
                        chapter_number: Some(chapter.chapter_number),
                        scene_order: None,
                    });
                }

                let mut seen_scene_orders = BTreeSet::new();
                for scene in chapter_scenes {
                    if !seen_scene_orders.insert(scene.scene_order) {
                        issues.push(ExportIssue {
                            severity: ExportIssueSeverity::Blocking,
                            code: "duplicate_scene_order".to_string(),
                            message: format!(
                                "Book {} chapter {} has duplicate scene_order {}.",
                                book.book_number, chapter.chapter_number, scene.scene_order
                            ),
                            book_number: Some(book.book_number),
                            chapter_number: Some(chapter.chapter_number),
                            scene_order: Some(scene.scene_order),
                        });
                    }
                    if scene.full_text.trim().is_empty() {
                        issues.push(ExportIssue {
                            severity: ExportIssueSeverity::Blocking,
                            code: "scene_empty_text".to_string(),
                            message: format!(
                                "Book {} chapter {} scene {} has empty text.",
                                book.book_number, chapter.chapter_number, scene.scene_order
                            ),
                            book_number: Some(book.book_number),
                            chapter_number: Some(chapter.chapter_number),
                            scene_order: Some(scene.scene_order),
                        });
                    } else if let Some(reason) =
                        scene_stub_reason(&scene.full_text, min_scene_words)
                    {
                        // Blocking on purpose: the live-run failure shipped an
                        // EPUB whose scene bodies were literally "placeholder"
                        // because no gate looked past empty text. The floor is
                        // per-project configurable, so a legitimately short
                        // scene lowers the floor rather than weakening the gate.
                        issues.push(ExportIssue {
                            severity: ExportIssueSeverity::Blocking,
                            code: "scene_stub_text".to_string(),
                            message: format!(
                                "Book {} chapter {} scene {} looks like a stub: {reason}.",
                                book.book_number, chapter.chapter_number, scene.scene_order
                            ),
                            book_number: Some(book.book_number),
                            chapter_number: Some(chapter.chapter_number),
                            scene_order: Some(scene.scene_order),
                        });
                    }
                }
            }
        }

        Ok(issues)
    }

    /// Export every project-scoped row to a single JSON file. Mirrors
    /// `services/mod.rs:1141..1162` in 705b835^. The SurrealDB reference
    /// walked the same set of tables via `db().select()` / `db().query()`;
    /// the SQLite implementation runs `SELECT *` per table through
    /// `Repository::dump_project_table` / `dump_table_by_column[_in]` and
    /// emits each row as a JSON object keyed by column name. The wire
    /// format ("schema-by-name JSON") matches the SurrealDB output for
    /// columns whose values are scalars/strings; JSON-payload columns are
    /// re-parsed back into nested `Value` (`looks_like_json_column`) so the
    /// round-trip preserves the original SurrealDB-side shape for the
    /// hot-path tables (`scene.dynamics`, `character.voice_profile`, etc.).
    pub async fn export_bible(
        &self,
        input: spindle_core::models::ExportBibleInput,
    ) -> Result<spindle_core::models::ExportBibleOutput> {
        use spindle_core::models::ExportBibleOutput;
        use std::collections::BTreeMap;

        let artifact = self
            .build_project_export_payload(
                &input.project_id,
                "spindle-bible-export-v1",
                BTreeMap::new(),
            )
            .await?;

        let filename = format!(
            "{}-bible-export.json",
            slugify_filename_component(&artifact.project_name)
        );
        let export_dir = export_dir_for_data_dir(self.repository.data_dir());
        std::fs::create_dir_all(&export_dir)?;
        let file_path = export_dir.join(&filename);
        std::fs::write(&file_path, serde_json::to_vec_pretty(&artifact.payload)?)?;

        Ok(ExportBibleOutput {
            file_path: file_path.to_string_lossy().to_string(),
            filename,
            exported_tables: artifact.exported_tables,
            exported_records: artifact.exported_records,
        })
    }

    /// Walk every project-scoped table and assemble a serializable export
    /// payload. Returns the row counts (per-table and total) alongside the
    /// JSON value so callers can persist both the file and the metadata
    /// (e.g. `save_point.snapshot_record_count`). Mirrors
    /// `build_project_export_payload` (`services/mod.rs:1162..~1616`) in
    /// 705b835^ — same table list, same per-table filter shape.
    ///
    /// Phase 6: the typed-record path. Each table's rows go through
    /// `serde_json::to_value(record)` so timestamps render as RFC-3339
    /// strings (matching the SurrealDB reference output shape) rather than
    /// raw unix-microsecond integers. The downstream restore path
    /// (`restore_branch_snapshot`) parses those RFC-3339 strings back into
    /// integer micros via `JsonParam::from_value_for_column`, so save-point
    /// round-trips still work.
    pub(super) async fn build_project_export_payload(
        &self,
        project_id: &str,
        format: &str,
        extra_metadata: std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<ProjectExportArtifact> {
        use serde::Serialize;
        use serde_json::{Value, json};
        use std::collections::BTreeMap;

        /// Map a `Vec<R>` of typed records into `Vec<Value>` for the
        /// table-keyed JSON payload. Each record goes through
        /// `serde_json::to_value`, which preserves serde-derived field
        /// names and uses `chrono::serde` to emit timestamps as RFC-3339.
        fn to_values<R: Serialize>(records: Vec<R>) -> Result<Vec<Value>> {
            records
                .into_iter()
                .map(|r| serde_json::to_value(&r).map_err(anyhow::Error::from))
                .collect()
        }

        let project = self.repository.get_project(project_id).await?;
        let project_value = serde_json::to_value(&project)?;

        let branches = self.repository.list_branches_by_project(project_id).await?;
        let branch_ids: Vec<String> = branches.iter().map(|b| b.id.clone()).collect();

        // import_session has nullable project_id; the SurrealDB reference
        // also includes sessions whose target_branch_id is one of this
        // project's branches. Walk project_id first, then target_branch_id
        // (deduped on id).
        let mut import_sessions = self
            .repository
            .list_import_sessions_by_project(project_id)
            .await?;
        let seen_session_ids: std::collections::BTreeSet<String> =
            import_sessions.iter().map(|s| s.id.clone()).collect();
        let mut extra_by_branch = self
            .repository
            .list_import_sessions_by_target_branch_ids(&branch_ids)
            .await?;
        extra_by_branch.retain(|s| !seen_session_ids.contains(&s.id));
        import_sessions.append(&mut extra_by_branch);
        let import_session_ids: Vec<String> =
            import_sessions.iter().map(|s| s.id.clone()).collect();

        let mut tables: BTreeMap<String, Value> = BTreeMap::new();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();

        insert_export_object(&mut tables, &mut counts, "project", project_value);
        insert_export_rows(
            &mut tables,
            &mut counts,
            "bible_branch",
            to_values(branches)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "save_point",
            to_values(self.repository.list_all_save_points(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "book",
            to_values(self.repository.list_books_by_project(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "chapter",
            to_values(self.repository.list_chapters_by_project(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "character",
            to_values(self.repository.list_all_characters(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "character_voice_profile",
            to_values(
                self.repository
                    .list_all_character_voice_profiles(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "character_emotional_profile",
            to_values(
                self.repository
                    .list_all_character_emotional_profiles(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "character_state",
            to_values(
                self.repository
                    .list_all_character_states(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "location",
            to_values(self.repository.list_all_locations(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "world_state",
            to_values(self.repository.list_all_world_states(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "world_rule",
            to_values(self.repository.list_all_world_rules(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "revision_marker",
            to_values(
                self.repository
                    .list_all_revision_markers(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "scene",
            to_values(self.repository.list_all_scenes(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "scene_version",
            to_values(self.repository.list_all_scene_versions(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "relates_to",
            to_values(
                self.repository
                    .list_all_relationships_by_branch_ids(&branch_ids)
                    .await?,
            )?,
        );
        // Export the delta ledger with the totals it explains. Older save
        // points omit this table; restore treats an absent table as empty.
        insert_export_rows(
            &mut tables,
            &mut counts,
            "relationship_scene_delta",
            self.repository
                .dump_table_by_column_in("relationship_scene_delta", "branch_id", &branch_ids)
                .await?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "faction",
            to_values(self.repository.list_all_factions(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "religion",
            to_values(self.repository.list_all_religions(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "economy",
            to_values(self.repository.list_all_economies(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "term",
            to_values(self.repository.list_all_terms(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "plot_line",
            to_values(self.repository.list_all_plot_lines(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "conflict",
            to_values(self.repository.list_all_conflicts(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "theme",
            to_values(self.repository.list_all_themes(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "motif",
            to_values(self.repository.list_all_motifs(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "narrative_promise",
            to_values(
                self.repository
                    .list_all_narrative_promises(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "character_arc",
            to_values(self.repository.list_all_character_arcs(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "pacing_config",
            to_values(
                self.repository
                    .list_pacing_configs_by_project(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "pacing_curve",
            to_values(
                self.repository
                    .list_pacing_curves_by_project(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "pacing_tracker",
            to_values(self.repository.list_all_pacing_trackers(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "book_outline",
            to_values(
                self.repository
                    .list_all_book_outlines_by_branch_ids(&branch_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "chapter_outline",
            to_values(
                self.repository
                    .list_all_chapter_outlines_by_branch_ids(&branch_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "chapter_plan",
            to_values(self.repository.list_all_chapter_plans(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "scene_beat_annotation",
            to_values(
                self.repository
                    .list_all_scene_beat_annotations(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "chapter_summary",
            to_values(
                self.repository
                    .list_all_chapter_summaries(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "future_knowledge",
            to_values(
                self.repository
                    .list_all_future_knowledge(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "timeline_event",
            to_values(self.repository.list_all_timeline_events(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "temporal_intervention",
            to_values(
                self.repository
                    .list_all_temporal_interventions(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "system_overlay",
            to_values(self.repository.list_all_system_overlays(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "progression_event",
            to_values(
                self.repository
                    .list_all_progression_events(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "dual_persona_review",
            to_values(
                self.repository
                    .list_all_dual_persona_reviews(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "canonical_fact",
            to_values(self.repository.list_all_canonical_facts(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "scene_source_link",
            to_values(
                self.repository
                    .list_scene_source_links_by_project(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "knowledge_fact",
            to_values(
                self.repository
                    .list_knowledge_facts_by_project(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "research_log",
            to_values(
                self.repository
                    .list_research_logs_by_project(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "knows",
            to_values(self.repository.list_all_knows(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_session",
            to_values(import_sessions)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_source_document",
            to_values(
                self.repository
                    .list_import_source_documents_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_segment",
            to_values(
                self.repository
                    .list_import_segments_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_entity_mention",
            to_values(
                self.repository
                    .list_import_entity_mentions_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_entity_cluster",
            to_values(
                self.repository
                    .list_import_entity_clusters_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_character_dossier",
            to_values(
                self.repository
                    .list_import_character_dossiers_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_world_dossier",
            to_values(
                self.repository
                    .list_import_world_dossiers_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_narrative_dossier",
            to_values(
                self.repository
                    .list_import_narrative_dossiers_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_resume_snapshot",
            to_values(
                self.repository
                    .list_import_resume_snapshots_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_review_item",
            to_values(
                self.repository
                    .list_import_review_items_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );

        let mut payload_fields = serde_json::Map::new();
        payload_fields.insert("format".to_string(), Value::String(format.to_string()));
        payload_fields.insert(
            "exported_at".to_string(),
            Value::String(chrono::Utc::now().to_rfc3339()),
        );
        payload_fields.insert(
            "project_id".to_string(),
            Value::String(project_id.to_string()),
        );
        payload_fields.insert(
            "project_name".to_string(),
            Value::String(project.name.clone()),
        );
        for (key, value) in extra_metadata {
            payload_fields.insert(key, value);
        }
        payload_fields.insert(
            "omitted_tables".to_string(),
            json!([
                "search_embedding",
                "validator_finding",
                "session_activity",
                "writer_position"
            ]),
        );
        payload_fields.insert("record_counts".to_string(), serde_json::to_value(&counts)?);
        payload_fields.insert("tables".to_string(), serde_json::to_value(&tables)?);

        let exported_tables = counts.len();
        let exported_records = counts.values().copied().sum();

        Ok(ProjectExportArtifact {
            project_name: project.name,
            payload: Value::Object(payload_fields),
            exported_tables,
            exported_records,
        })
    }

    /// Render an EPUB from the project's active branch. Runs the same
    /// preflight as `preflight_book_export`; bails on any Blocking issue
    /// with a formatted summary; passes Warnings through to the output.
    /// Mirrors services/mod.rs:832..995 in 705b835^.
    ///
    /// `divergence_warnings` is populated by walking
    /// `list_scene_source_links_by_project` and running each in-scope
    /// link through `SourceBridge::evaluate_scene_divergence`. Risk #7 is
    /// closed.
    pub async fn export_epub(
        &self,
        input: spindle_core::models::ExportEpubInput,
    ) -> Result<spindle_core::models::ExportEpubOutput> {
        use crate::export::{EpubBook, EpubChapter, EpubSource, build_epub};
        use spindle_core::models::{ExportEpubOutput, ExportIssueSeverity};

        let project = self.repository.get_project(&input.project_id).await?;
        let chapter_scope = resolve_export_chapter_scope(
            input.book_number,
            input.start_chapter_number,
            input.end_chapter_number,
        )?;
        let preflight_issues = self
            .validate_book_for_export(&input.project_id, input.book_number, chapter_scope)
            .await?;
        let preflight_warnings = preflight_issues
            .iter()
            .filter(|issue| matches!(issue.severity, ExportIssueSeverity::Warning))
            .cloned()
            .collect::<Vec<_>>();
        let blocking_issues = preflight_issues
            .iter()
            .filter(|issue| matches!(issue.severity, ExportIssueSeverity::Blocking))
            .cloned()
            .collect::<Vec<_>>();
        if !blocking_issues.is_empty() {
            anyhow::bail!(
                "book export preflight failed:\n{}",
                format_export_issues(&blocking_issues)
            );
        }

        let all_books = self
            .repository
            .list_books_by_project(&input.project_id)
            .await?;
        let books_to_export: Vec<_> = match input.book_number {
            Some(n) => all_books.iter().filter(|b| b.book_number == n).collect(),
            None => all_books.iter().collect(),
        };
        if books_to_export.is_empty() {
            anyhow::bail!("no books found for export");
        }

        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let all_scenes = self
            .repository
            .list_scenes_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?;

        let mut total_scenes = 0usize;
        let mut total_chapters = 0usize;
        let mut epub_books = Vec::new();

        for book in &books_to_export {
            let chapters = self
                .repository
                .list_chapters_by_book(&book.id)
                .await?
                .into_iter()
                .filter(|chapter| {
                    export_scope_contains_chapter(chapter_scope, chapter.chapter_number)
                })
                .collect::<Vec<_>>();

            let mut epub_chapters = Vec::new();
            for chapter in &chapters {
                let chapter_scenes: Vec<_> = all_scenes
                    .iter()
                    .filter(|s| {
                        s.book_number == book.book_number
                            && s.chapter_number == chapter.chapter_number
                    })
                    .collect();
                if chapter_scenes.is_empty() {
                    continue;
                }
                total_scenes += chapter_scenes.len();
                let body = chapter_scenes
                    .iter()
                    .map(|s| s.full_text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                epub_chapters.push(EpubChapter {
                    number: chapter.chapter_number,
                    title: chapter.title.clone(),
                    body,
                });
            }
            if !epub_chapters.is_empty() {
                total_chapters += epub_chapters.len();
                epub_books.push(EpubBook {
                    number: book.book_number,
                    title: book.title.clone(),
                    chapters: epub_chapters,
                });
            }
        }

        if epub_books.is_empty() {
            anyhow::bail!("no scenes found for export — nothing to publish");
        }

        let source = EpubSource {
            title: project.name.clone(),
            author: input.author,
            language: "en".to_string(),
            books: epub_books,
        };

        let epub_bytes = build_epub(&source)?;
        let slug = project.name.to_lowercase().replace(' ', "-");
        let filename = match (input.book_number, chapter_scope) {
            (Some(n), Some(scope)) => format!(
                "{slug}-book-{n}-chapters-{}-{}.epub",
                scope.start_chapter_number, scope.end_chapter_number
            ),
            (Some(n), None) => format!("{slug}-book-{n}.epub"),
            (None, _) => format!("{slug}.epub"),
        };

        // Divergence detection at export time (Gates 1 + 3). Mirrors
        // services/mod.rs:947..986 in 705b835^.
        let mut divergence_warnings: Vec<spindle_core::models::DivergenceWarning> = Vec::new();
        let source_links = self
            .repository
            .list_scene_source_links_by_project(&input.project_id)
            .await
            .unwrap_or_default();
        if !source_links.is_empty() {
            let scene_map: std::collections::BTreeMap<String, &crate::sqlite::records::Scene> =
                all_scenes.iter().map(|s| (s.id.clone(), s)).collect();
            for link in &source_links {
                let Some(scene) = scene_map.get(&link.scene_id) else {
                    continue;
                };
                let in_book_scope = match input.book_number {
                    Some(n) => scene.book_number == n,
                    None => true,
                };
                if !in_book_scope
                    || !export_scope_contains_chapter(chapter_scope, scene.chapter_number)
                {
                    continue;
                }
                if let Some(observation) =
                    crate::sqlite::source_bridge::evaluate_scene_divergence(link, scene)
                {
                    divergence_warnings.push(spindle_core::models::DivergenceWarning {
                        scene_id: scene.id.clone(),
                        book_number: scene.book_number,
                        chapter_number: scene.chapter_number,
                        scene_order: scene.scene_order,
                        source_path: link.source_path.clone(),
                        kind: observation.kind,
                        detail: observation.detail,
                    });
                }
            }
        }

        let export_dir = export_dir_for_data_dir(self.repository.data_dir());
        std::fs::create_dir_all(&export_dir)?;
        let file_path = export_dir.join(&filename);
        std::fs::write(&file_path, &epub_bytes)?;

        Ok(ExportEpubOutput {
            file_path: file_path.to_string_lossy().to_string(),
            filename,
            total_chapters,
            total_scenes,
            preflight_warnings,
            divergence_warnings,
        })
    }

    /// Assemble the committed prose of a book (or a chapter range within it) on
    /// the active branch into a single Markdown "read-so-far" document. Unlike
    /// `export_epub` (an end-product path) this is a mid-run review view:
    /// planned-but-undrafted scenes are rendered as explicit placeholders and
    /// reported in `missing_scenes` rather than silently omitted. Read-only
    /// over scene/plan data; the workspace write is opt-in via
    /// `write_to_workspace`. Branch resolution is implicit (active branch),
    /// matching the other read tools.
    pub async fn compile_manuscript(
        &self,
        input: spindle_core::models::CompileManuscriptInput,
    ) -> Result<spindle_core::models::CompileManuscriptOutput> {
        use spindle_core::models::CompileManuscriptOutput;

        let project = self.repository.get_project(&input.project_id).await?;
        let min_scene_words = project
            .min_scene_word_count
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_MIN_SCENE_WORD_COUNT);
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;

        // Chapters of the requested book, in order, filtered to the (optional)
        // inclusive range. Absent book → empty chapter set → structured empty.
        let chapters = match self
            .repository
            .find_book_by_number(&input.project_id, input.book_number)
            .await?
        {
            Some(book) => self
                .repository
                .list_chapters_by_book(&book.id)
                .await?
                .into_iter()
                .filter(|chapter| {
                    input
                        .start_chapter
                        .is_none_or(|start| chapter.chapter_number >= start)
                        && input
                            .end_chapter
                            .is_none_or(|end| chapter.chapter_number <= end)
                })
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };

        // Active-branch scenes for this book, indexed by (chapter, scene_order).
        let scenes = self
            .repository
            .list_scenes_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?;
        let mut scene_by_position: std::collections::BTreeMap<
            (i32, i32),
            &crate::sqlite::records::Scene,
        > = std::collections::BTreeMap::new();
        for scene in &scenes {
            if scene.book_number == input.book_number {
                scene_by_position.insert((scene.chapter_number, scene.scene_order), scene);
            }
        }

        // Chapter-plan spines (active-branch scoped) keyed by chapter number, so
        // a planned scene with no persisted prose is enumerated as a gap.
        let plans = self
            .repository
            .list_chapter_plans_by_project(&input.project_id)
            .await?;
        let plan_orders_by_chapter: std::collections::BTreeMap<i32, Vec<i32>> = plans
            .into_iter()
            .filter(|plan| plan.book_number == input.book_number)
            .map(|plan| {
                (
                    plan.chapter_number,
                    plan.scenes
                        .iter()
                        .map(|scene| scene.scene_order)
                        .collect::<Vec<_>>(),
                )
            })
            .collect();

        let mut markdown = String::new();
        let mut prose_parts: Vec<String> = Vec::new();
        let mut scene_count = 0usize;
        let mut missing_scenes: Vec<String> = Vec::new();
        let mut stub_scenes: Vec<String> = Vec::new();

        for chapter in &chapters {
            let chapter_number = chapter.chapter_number;
            match chapter.title.as_deref().map(str::trim) {
                Some(title) if !title.is_empty() => {
                    markdown.push_str(&format!("## Chapter {chapter_number}: {title}\n\n"));
                }
                _ => markdown.push_str(&format!("## Chapter {chapter_number}\n\n")),
            }

            // Union of persisted scene_orders and plan-spine scene_orders for
            // this chapter, ascending, deduplicated.
            let mut orders: std::collections::BTreeSet<i32> = scene_by_position
                .keys()
                .filter(|(ch, _)| *ch == chapter_number)
                .map(|(_, order)| *order)
                .collect();
            if let Some(plan_orders) = plan_orders_by_chapter.get(&chapter_number) {
                orders.extend(plan_orders.iter().copied());
            }

            for scene_order in orders {
                markdown.push_str(&format!("### Scene {chapter_number}.{scene_order}\n\n"));
                let drafted = scene_by_position
                    .get(&(chapter_number, scene_order))
                    .filter(|scene| !scene.full_text.trim().is_empty());
                match drafted {
                    Some(scene) => {
                        markdown.push_str(&scene.full_text);
                        markdown.push_str("\n\n");
                        prose_parts.push(scene.full_text.clone());
                        scene_count += 1;
                        // Stub scenes still render — but the compiled document
                        // must name them so they are never read as finished
                        // prose (the live run shipped "placeholder" bodies).
                        if scene_stub_reason(&scene.full_text, min_scene_words).is_some() {
                            stub_scenes.push(format!("{chapter_number}.{scene_order}"));
                        }
                    }
                    None => {
                        markdown.push_str(&format!(
                            "> [scene {chapter_number}.{scene_order} not yet drafted]\n\n"
                        ));
                        missing_scenes.push(format!("{chapter_number}.{scene_order}"));
                    }
                }
            }
        }

        let word_count = prose_parts
            .iter()
            .map(|part| part.split_whitespace().count())
            .sum();

        let artifact_path = if input.write_to_workspace {
            let artifacts_dir = artifacts_dir_for_data_dir(self.repository.data_dir());
            std::fs::create_dir_all(&artifacts_dir)?;
            let start = input
                .start_chapter
                .or_else(|| chapters.first().map(|chapter| chapter.chapter_number))
                .unwrap_or(0);
            let end = input
                .end_chapter
                .or_else(|| chapters.last().map(|chapter| chapter.chapter_number))
                .unwrap_or(start);
            let filename = format!("book-{}-ch{start}-{end}.md", input.book_number);
            let file_path = artifacts_dir.join(&filename);
            std::fs::write(&file_path, markdown.as_bytes())?;
            Some(file_path.to_string_lossy().to_string())
        } else {
            None
        };

        Ok(CompileManuscriptOutput {
            markdown,
            scene_count,
            word_count,
            missing_scenes,
            stub_scenes,
            artifact_path,
        })
    }

    // ── Reader-facing artifacts (export_recap / export_series_bible — §3.8) ──

    /// Resolve the set of secret-`canonical_fact` ids that a **reader artifact**
    /// may name at `cursor` (a [`crate::format::story_index`] value).
    ///
    /// This is the ONE reader-secret resolver shared by [`Self::export_recap`]
    /// and [`Self::export_series_bible`], and it is deliberately *stricter* than
    /// the scene-context circle-of-trust gate ([`Self::resolve_scene_secret_gate`]).
    ///
    /// The two gates protect different audiences. Scene-context gating protects
    /// the *characters* on the page from knowing something they haven't learned —
    /// so it turns on who is present and what the POV knows. Reader artifacts
    /// instead protect the *reader* from spoilers, so the authorial dial is
    /// `knowledge_fact.reader_visible` and the reveal date is the linked reveal
    /// row's `learned_at` placement.
    ///
    /// A secret fact (`canonical_fact.secret = 1`) is includable IFF at least one
    /// linked reveal row (`knowledge_fact.secret_of_fact_id = fact.id`) has
    /// `reader_visible = true` AND a dated `learned_at` placement that is at or
    /// before the cursor. With no qualifying reveal the fact — and any
    /// thread/summary/page line that names it — is withheld. Public facts
    /// (`secret = 0`) are never gated here. Pass `i64::MAX` for the whole-project
    /// (+infinity) cursor so only never-revealed secrets are held.
    ///
    /// Placement discipline (a reconciliation with the raw data model): the
    /// declaration path (`register_canonical_fact { secrecy }`) already writes a
    /// circle-of-trust holder row with `reader_visible = true` and
    /// `learned_at = NULL` — that row is standing DRAMATIC IRONY (a *character*
    /// holds the secret), not a *reader* reveal. Because the spec's own rationale
    /// makes "the placement is the reveal date", a NULL-placement holder row is
    /// NOT treated as a reader reveal here; only a row carrying a concrete
    /// `learned_at` (the author's dated reveal, e.g. via `record_knowledge` with
    /// `secret_of_fact_id`) lifts the spoiler veil. This keeps `reader_visible`
    /// as the authorial dial while preventing every held secret from leaking to
    /// the reader the moment it is declared.
    ///
    /// Returns the display values of secret facts that MUST be withheld (the gate
    /// set), so callers strip any output line whose text contains one of them.
    pub(super) async fn reader_withheld_secret_values(
        &self,
        project_id: &str,
        branch_id: &str,
        cursor: i64,
    ) -> Result<Vec<String>> {
        use crate::format::{canonical_fact_value_display, story_index_from_placement};

        let facts = self
            .repository
            .list_canonical_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let secret_facts: Vec<&crate::sqlite::records::CanonicalFact> =
            facts.iter().filter(|fact| fact.secret).collect();
        if secret_facts.is_empty() {
            return Ok(Vec::new());
        }

        // All knowledge rows on the branch; a reveal is one linked to the secret,
        // reader-visible, and carrying a dated placement at/under the cursor.
        let knowledge = self
            .repository
            .list_knowledge_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let mut withheld = Vec::new();
        for fact in secret_facts {
            let reader_reached = knowledge.iter().any(|row| {
                row.secret_of_fact_id.as_deref() == Some(fact.id.as_str())
                    && row.reader_visible
                    && row
                        .learned_at
                        .as_ref()
                        .is_some_and(|placement| story_index_from_placement(placement) <= cursor)
            });
            if !reader_reached {
                // Withhold every rendering of the secret's value: read models
                // print `canonical_fact_value_display`, so that display string is
                // the sentinel any recap/bible line would carry.
                withheld.push(canonical_fact_value_display(fact));
            }
        }
        Ok(withheld)
    }

    /// Spoiler-bounded "previously on" recap of a book up to (and including)
    /// `through_chapter` (evolution §3.8). Pure read model, zero model calls.
    ///
    /// Cursor = end of `through_chapter` in `book_number` — packed as
    /// `story_index(book, chapter, SCENE_RADIX - 1)` so every scene of that
    /// chapter is inside the window but the next chapter is not. Sources:
    /// per-chapter `chapter_summary` rows (chosen over `book_digest` because the
    /// digest condenses across chapters and can lag the cursor); `narrative_promise`
    /// rows for the paid-off / still-open sections. The reader-secret rule is
    /// applied to every rendered line.
    pub async fn export_recap(
        &self,
        input: spindle_core::models::ExportRecapInput,
    ) -> Result<spindle_core::models::ExportRecapOutput> {
        use crate::format::{SCENE_RADIX, story_index};
        use spindle_core::models::ExportRecapOutput;

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;

        // Inclusive end-of-chapter cursor (scene_order below the radix ceiling).
        let cursor = story_index(
            input.book_number,
            input.through_chapter,
            (SCENE_RADIX - 1) as i32,
        );

        let withheld = self
            .reader_withheld_secret_values(&input.project_id, &active_branch.id, cursor)
            .await?;
        let names_secret = |text: &str| withheld.iter().any(|value| text.contains(value.as_str()));

        // Story-so-far from per-chapter summaries at/under the cursor, in order.
        let mut summaries = self
            .repository
            .list_chapter_summaries_by_project(&input.project_id)
            .await?;
        summaries.retain(|summary| {
            summary.branch_id == active_branch.id
                && summary.book_number == input.book_number
                && summary.chapter_number <= input.through_chapter
        });
        summaries.sort_by_key(|summary| summary.chapter_number);

        let mut markdown = String::new();
        markdown.push_str("# Previously on\n\n");
        markdown.push_str("## The story so far\n\n");
        let mut chapter_count = 0usize;
        if summaries.is_empty() {
            markdown.push_str("_Nothing to recap yet._\n\n");
        } else {
            for summary in &summaries {
                chapter_count += 1;
                // A summary line that names a still-secret value is withheld
                // wholesale (the reader must not learn it early).
                let body = if names_secret(&summary.summary) {
                    "_(a development here is being kept under wraps)_"
                } else {
                    summary.summary.as_str()
                };
                markdown.push_str(&format!(
                    "**Chapter {}.** {}\n\n",
                    summary.chapter_number, body
                ));
            }
        }

        // Promises on the active branch, split into paid-off / still-open.
        let promises = self
            .repository
            .list_narrative_promises_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?;

        // Only recorded actual events can establish a payoff at this cursor.
        let mut paid: Vec<_> = promises
            .iter()
            .filter(|promise| {
                promise.archived_at.is_none() && promise.status_at(cursor) == Some("paid_off")
            })
            .filter(|promise| !names_secret(&promise.description))
            .collect();
        paid.sort_by(|a, b| a.id.cmp(&b.id));

        let mut open: Vec<_> = promises
            .iter()
            .filter(|promise| promise.archived_at.is_none())
            .filter(|promise| {
                promise
                    .status_at(cursor)
                    .is_some_and(|status| status != "paid_off" && status != "abandoned")
            })
            .filter(|promise| !names_secret(&promise.description))
            .collect();
        open.sort_by(|a, b| a.id.cmp(&b.id));

        markdown.push_str("## Paid off\n\n");
        if paid.is_empty() {
            markdown.push_str("_No confirmed payoffs at this point._\n\n");
        } else {
            for promise in &paid {
                markdown.push_str(&format!("- {}\n", promise.description));
            }
            markdown.push('\n');
        }

        markdown.push_str("## Questions still hanging\n\n");
        if open.is_empty() {
            markdown.push_str("_No confirmed open questions at this point._\n\n");
        } else {
            for promise in &open {
                markdown.push_str(&format!("- {}\n", promise.description));
            }
            markdown.push('\n');
        }

        let word_count = markdown.split_whitespace().count();

        let artifact_path = if input.write_to_workspace {
            let artifacts_dir = artifacts_dir_for_data_dir(self.repository.data_dir());
            std::fs::create_dir_all(&artifacts_dir)?;
            let filename = format!(
                "recap-book-{}-through-ch{}.md",
                input.book_number, input.through_chapter
            );
            let file_path = artifacts_dir.join(&filename);
            std::fs::write(&file_path, markdown.as_bytes())?;
            Some(file_path.to_string_lossy().to_string())
        } else {
            None
        };

        Ok(ExportRecapOutput {
            markdown,
            chapter_count,
            word_count,
            artifact_path,
        })
    }

    /// Spoiler-bounded, reader-facing series bible as-of an optional cursor
    /// (evolution §3.8). Pure read model, zero model calls.
    ///
    /// Cursor: `through` present → packed placement (chapter-level cursors use
    /// scene_order `SCENE_RADIX - 1` so the whole chapter is inside the window);
    /// absent → `i64::MAX` (+infinity, whole project). Sections, deterministically
    /// ordered: characters (name, role, summary, status/emotional state taken
    /// from the latest placement-stamped `character_state` at/under the cursor,
    /// plus banded relationships), locations, a sorted glossary of `term`s, and
    /// factions/religions when present. The reader-secret rule gates every line.
    pub async fn export_series_bible(
        &self,
        input: spindle_core::models::ExportSeriesBibleInput,
    ) -> Result<spindle_core::models::ExportSeriesBibleOutput> {
        use crate::format::{SCENE_RADIX, story_index};
        use spindle_core::models::ExportSeriesBibleOutput;

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let branch_id = active_branch.id.clone();

        // Cursor: a chapter-level `through` (no scene_order) covers the whole
        // chapter; absent `through` is +infinity.
        let cursor = match input.through.as_ref() {
            Some(placement) => story_index(
                placement.book_number,
                placement.chapter_number,
                placement.scene_order.unwrap_or((SCENE_RADIX - 1) as i32),
            ),
            None => i64::MAX,
        };

        let withheld = self
            .reader_withheld_secret_values(&input.project_id, &branch_id, cursor)
            .await?;
        let names_secret = |text: &str| withheld.iter().any(|value| text.contains(value.as_str()));

        let mut markdown = String::new();
        markdown.push_str("# Series Bible\n\n");

        // ── Characters (deterministic by id) ────────────────────────────────
        let mut characters = self
            .repository
            .list_characters_by_project(&input.project_id)
            .await?;
        characters.sort_by(|a, b| a.id.cmp(&b.id));

        // Placement-stamped states (character_state carries book/chapter/scene),
        // so the state as-of the cursor is the latest state at/under it.
        let states = self
            .repository
            .list_character_states_by_project_and_branch(&input.project_id, &branch_id)
            .await?;
        // Relationship edges on the branch, for the reader-voice banding.
        let relationships = self
            .repository
            .list_relationships_by_branch(&branch_id)
            .await?;
        let name_of: std::collections::HashMap<&str, &str> = characters
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str()))
            .collect();

        markdown.push_str("## Characters\n\n");
        let mut character_count = 0usize;
        for character in &characters {
            character_count += 1;
            markdown.push_str(&format!("### {}\n\n", character.name));
            markdown.push_str(&format!("- Role: {}\n", character.role));
            if !character.summary.trim().is_empty() {
                markdown.push_str(&format!("- Summary: {}\n", character.summary));
            }

            // Latest state at/under the cursor (states are placement-stamped).
            let latest = states
                .iter()
                .filter(|state| state.character_id == character.id)
                .filter(|state| {
                    story_index(state.book_number, state.chapter_number, state.scene_order)
                        <= cursor
                })
                .max_by_key(|state| {
                    story_index(state.book_number, state.chapter_number, state.scene_order)
                });
            if let Some(state) = latest {
                if !state.status.is_empty() {
                    markdown.push_str(&format!("- Status: {}\n", state.status.join(", ")));
                }
                if !state.emotional_state.is_empty() {
                    let mood: Vec<String> = state
                        .emotional_state
                        .iter()
                        .map(|(key, value)| format!("{key}: {value}"))
                        .collect();
                    markdown.push_str(&format!("- Mood: {}\n", mood.join(", ")));
                }
            }

            // Relationships in reader-voice bands (never raw trust/tension). Bands
            // (documented, deterministic): "allies" when trust exceeds tension,
            // "strained" when tension exceeds trust, "entangled" when they are
            // level. Edges are deduplicated by the pair and named by the other id.
            let mut rel_lines: Vec<String> = relationships
                .iter()
                .filter(|edge| edge.in_id == character.id || edge.out_id == character.id)
                .map(|edge| {
                    let other = if edge.in_id == character.id {
                        edge.out_id.as_str()
                    } else {
                        edge.in_id.as_str()
                    };
                    let other_name = name_of.get(other).copied().unwrap_or(other);
                    let band = if edge.trust > edge.tension {
                        "allies"
                    } else if edge.tension > edge.trust {
                        "strained"
                    } else {
                        "entangled"
                    };
                    format!("{other_name} ({band})")
                })
                .collect();
            rel_lines.sort();
            rel_lines.dedup();
            if !rel_lines.is_empty() {
                markdown.push_str(&format!("- Relationships: {}\n", rel_lines.join(", ")));
            }
            markdown.push('\n');
        }
        if characters.is_empty() {
            markdown.push_str("_No characters yet._\n\n");
        }

        // ── Locations (kind, then name) ─────────────────────────────────────
        let mut locations = self
            .repository
            .list_locations_by_project(&input.project_id)
            .await?;
        locations.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
        markdown.push_str("## Locations\n\n");
        if locations.is_empty() {
            markdown.push_str("_No locations yet._\n\n");
        } else {
            for location in &locations {
                markdown.push_str(&format!(
                    "- **{}** ({}): {}\n",
                    location.name, location.kind, location.summary
                ));
            }
            markdown.push('\n');
        }

        // ── Glossary (terms, sorted by term text) ───────────────────────────
        let mut terms = self
            .repository
            .list_terms_by_project(&input.project_id)
            .await?;
        terms.retain(|term| !names_secret(&term.term_text) && !names_secret(&term.definition));
        terms.sort_by(|a, b| a.term_text.cmp(&b.term_text));
        markdown.push_str("## Glossary\n\n");
        if terms.is_empty() {
            markdown.push_str("_No glossary entries yet._\n\n");
        } else {
            for term in &terms {
                markdown.push_str(&format!("- **{}**: {}\n", term.term_text, term.definition));
            }
            markdown.push('\n');
        }

        // ── Factions (optional; by name) ────────────────────────────────────
        let mut factions = self
            .repository
            .list_factions_by_project(&input.project_id)
            .await?;
        factions.retain(|faction| faction.archived_at.is_none());
        factions.sort_by(|a, b| a.name.cmp(&b.name));
        if !factions.is_empty() {
            markdown.push_str("## Factions\n\n");
            for faction in &factions {
                markdown.push_str(&format!(
                    "- **{}** ({}): {}\n",
                    faction.name, faction.faction_type, faction.summary
                ));
            }
            markdown.push('\n');
        }

        // ── Religions (optional; by name) ───────────────────────────────────
        let mut religions = self
            .repository
            .list_religions_by_project(&input.project_id)
            .await?;
        religions.retain(|religion| religion.archived_at.is_none());
        religions.sort_by(|a, b| a.name.cmp(&b.name));
        if !religions.is_empty() {
            markdown.push_str("## Religions\n\n");
            for religion in &religions {
                markdown.push_str(&format!("- **{}**: {}\n", religion.name, religion.summary));
            }
            markdown.push('\n');
        }

        let word_count = markdown.split_whitespace().count();

        let artifact_path = if input.write_to_workspace {
            let artifacts_dir = artifacts_dir_for_data_dir(self.repository.data_dir());
            std::fs::create_dir_all(&artifacts_dir)?;
            let filename = match input.through.as_ref() {
                Some(placement) => format!(
                    "series-bible-through-b{}c{}.md",
                    placement.book_number, placement.chapter_number
                ),
                None => "series-bible-full.md".to_string(),
            };
            let file_path = artifacts_dir.join(&filename);
            std::fs::write(&file_path, markdown.as_bytes())?;
            Some(file_path.to_string_lossy().to_string())
        } else {
            None
        };

        Ok(ExportSeriesBibleOutput {
            markdown,
            chapter_count: character_count,
            word_count,
            artifact_path,
        })
    }
}
