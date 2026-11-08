use super::*;

impl SqliteSpindleService {
    // =========================================================================
    // Import-pipeline private helpers.
    // =========================================================================

    /// Resolve the project + branch the caller wants to associate with a
    /// new import session. Mirrors the SurrealDB-era helper of the same
    /// name; on the SQLite side ids are plain `String` rather than
    /// `RecordId`.
    pub(super) async fn resolve_import_target(
        &self,
        input: &spindle_core::models::ImportManuscriptInput,
    ) -> Result<(
        crate::sqlite::records::Project,
        String,
        spindle_core::models::ImportHydrationMode,
    )> {
        use crate::sqlite::import_service::default_import_project_name;
        use spindle_core::models::{CreateProjectInput, ImportHydrationMode, ReaderContract};

        let Some(project_id) = input.target_project_id.as_deref() else {
            let project_name = input
                .create_project_name
                .clone()
                .unwrap_or_else(|| default_import_project_name(input));
            let project_output = self
                .create_project(CreateProjectInput {
                    name: project_name,
                    project_type: "novel".to_string(),
                    genre: "fiction".to_string(),
                    reader_contract: ReaderContract {
                        promise: "Imported manuscript continuation".to_string(),
                        style_notes: vec![
                            "Preserve the voice already present in the manuscript.".to_string(),
                        ],
                        boundaries: vec![],
                    },
                })
                .await?;
            let project = self
                .repository
                .get_project(&project_output.project_id)
                .await?;
            let branch_id = project
                .active_branch_id
                .clone()
                .or_else(|| {
                    Some(
                        // Resolve project main branch by name.
                        "bible_branch:main".to_string(),
                    )
                })
                .unwrap_or_else(|| "bible_branch:main".to_string());
            let branch_id = self.resolve_project_main_branch(&project, &branch_id).await;
            return Ok((project, branch_id, ImportHydrationMode::NewProject));
        };

        let project = self.repository.get_project(project_id).await?;
        let branch_id = if let Some(branch_id) = input.target_branch_id.as_deref() {
            let branch = self.repository.get_branch(branch_id).await?;
            if branch.project_id.as_deref() != Some(project.id.as_str())
                && branch.id != "bible_branch:main"
            {
                anyhow::bail!("target branch does not belong to the requested project");
            }
            branch.id
        } else {
            self.resolve_project_main_branch(&project, "bible_branch:main")
                .await
        };

        Ok((project, branch_id, ImportHydrationMode::ExistingProject))
    }

    /// Resolve the project + branch the caller wants to hydrate into.
    /// Returns `(project, branch_id, created_project)`.
    pub(super) async fn resolve_hydration_target(
        &self,
        input: &spindle_core::models::ImportHydrateBibleInput,
        session: &crate::sqlite::records::ImportSession,
    ) -> Result<(crate::sqlite::records::Project, String, bool)> {
        use crate::sqlite::import_service::parse_import_hydration_mode;
        use spindle_core::models::{CreateProjectInput, ImportHydrationMode, ReaderContract};

        let requested_mode = input.hydrate_mode.clone().unwrap_or_else(|| {
            if input.target_project_id.is_some() {
                ImportHydrationMode::ExistingProject
            } else {
                parse_import_hydration_mode(&session.hydrate_mode)
                    .unwrap_or(ImportHydrationMode::ExistingProject)
            }
        });

        if matches!(requested_mode, ImportHydrationMode::NewProject)
            || (input.target_project_id.is_none() && input.create_project_name.is_some())
        {
            let project_output = self
                .create_project(CreateProjectInput {
                    name: input
                        .create_project_name
                        .clone()
                        .unwrap_or_else(|| "Imported Manuscript".to_string()),
                    project_type: "novel".to_string(),
                    genre: "fiction".to_string(),
                    reader_contract: ReaderContract {
                        promise: "Imported manuscript continuation".to_string(),
                        style_notes: vec!["Preserve the imported manuscript voice.".to_string()],
                        boundaries: vec![],
                    },
                })
                .await?;
            let project = self
                .repository
                .get_project(&project_output.project_id)
                .await?;
            let branch_id = self
                .resolve_project_main_branch(&project, "bible_branch:main")
                .await;
            return Ok((project, branch_id, true));
        }

        let project_id = if let Some(target_project_id) = input.target_project_id.as_deref() {
            target_project_id.to_string()
        } else if let Some(session_project_id) = session.project_id.as_ref() {
            session_project_id.clone()
        } else {
            anyhow::bail!("hydration target project is required");
        };
        let project = self.repository.get_project(&project_id).await?;
        let branch_id = if let Some(branch_id) = input.target_branch_id.as_deref() {
            let branch = self.repository.get_branch(branch_id).await?;
            if branch.project_id.as_deref() != Some(project.id.as_str())
                && branch.id != "bible_branch:main"
            {
                anyhow::bail!("target branch does not belong to the requested project");
            }
            branch.id
        } else {
            let fallback = session
                .target_branch_id
                .clone()
                .unwrap_or_else(|| "bible_branch:main".to_string());
            self.resolve_project_main_branch(&project, &fallback).await
        };

        Ok((project, branch_id, false))
    }

    /// Resolve the project's main branch id (per-project design). Falls
    /// back to `fallback` when no branch named "main" is found.
    pub(super) async fn resolve_project_main_branch(
        &self,
        project: &crate::sqlite::records::Project,
        fallback: &str,
    ) -> String {
        if let Some(active) = project.active_branch_id.as_deref() {
            return active.to_string();
        }
        self.repository
            .list_branches_by_project(&project.id)
            .await
            .ok()
            .and_then(|branches| {
                branches
                    .into_iter()
                    .find(|b| b.name == "main")
                    .map(|b| b.id)
            })
            .unwrap_or_else(|| fallback.to_string())
    }

    /// Walk the slicer's structural analysis result, persist source
    /// documents + segments, and emit review items for low-confidence
    /// boundaries. Returns the summary the import-manuscript caller
    /// renders, paired with the count of review items created.
    pub(super) async fn persist_structural_analysis(
        &self,
        session_id: &str,
        project_id: &str,
        analysis: &crate::sqlite::import::StructuralAnalysisResult,
    ) -> Result<(spindle_core::models::ImportStructuralAnalysisSummary, usize)> {
        use crate::sqlite::import_service::{
            import_segment_status_name, import_source_format_name, parse_import_source_format,
        };
        use crate::sqlite::repository::{
            UpsertImportSegmentParams, UpsertImportSourceDocumentParams,
        };
        use spindle_core::models::{
            ImportChapterSlice, ImportCorrectionPayload, ImportSceneSlice,
            ImportSourceDocumentSummary, ImportStructuralAnalysisSummary,
        };
        use std::collections::BTreeMap;

        let mut source_documents = Vec::with_capacity(analysis.source_documents.len());
        let mut persisted_documents_by_order = BTreeMap::new();

        for document in &analysis.source_documents {
            let persisted_document = self
                .repository
                .upsert_import_source_document(UpsertImportSourceDocumentParams {
                    session_id: session_id.to_string(),
                    project_id: Some(project_id.to_string()),
                    display_name: document.display_name.clone(),
                    source_path: document.source_path.display().to_string(),
                    copied_path: document.copied_path.display().to_string(),
                    source_format: import_source_format_name(&document.source_format),
                    original_sha256: document.original_sha256.clone(),
                    normalized_sha256: document.normalized_sha256.clone(),
                    normalized_text_ref: document.normalized_text_path.display().to_string(),
                    word_count: document.word_count,
                    chapter_hint: document.chapter_hint.clone(),
                    source_order: document.source_order,
                })
                .await?;
            persisted_documents_by_order.insert(
                persisted_document.source_order.max(0) as usize,
                persisted_document.clone(),
            );
            source_documents.push(ImportSourceDocumentSummary {
                document_id: persisted_document.id.clone(),
                display_name: persisted_document.display_name,
                source_path: persisted_document.source_path,
                copied_path: persisted_document.copied_path,
                source_format: parse_import_source_format(&persisted_document.source_format),
                original_sha256: persisted_document.original_sha256,
                normalized_sha256: persisted_document.normalized_sha256,
                word_count: persisted_document.word_count.max(0) as usize,
                chapter_hint: persisted_document.chapter_hint,
                source_order: persisted_document.source_order.max(0) as usize,
            });
        }

        let mut chapters = Vec::with_capacity(analysis.chapters.len());
        let mut review_count = 0usize;
        for chapter in &analysis.chapters {
            let source_document = analysis
                .source_documents
                .get(chapter.source_document_index)
                .context("structural analysis referenced an unknown source document")?;
            let persisted_document = persisted_documents_by_order
                .get(&source_document.source_order)
                .cloned()
                .context("persisted import source document not found")?;

            let chapter_segment = self
                .repository
                .upsert_import_segment(UpsertImportSegmentParams {
                    session_id: session_id.to_string(),
                    source_document_id: persisted_document.id.clone(),
                    parent_segment_id: None,
                    segment_type: "chapter".to_string(),
                    source_order: chapter.start_offset,
                    book_number: Some(chapter.book_number),
                    chapter_number: Some(chapter.chapter_number),
                    scene_order: None,
                    label: chapter.title.clone(),
                    start_offset: chapter.start_offset,
                    end_offset: chapter.end_offset,
                    word_count: chapter.word_count,
                    character_count: chapter.end_offset.saturating_sub(chapter.start_offset),
                    pov_guess: None,
                    confidence: chapter.confidence,
                    segment_status: import_segment_status_name(),
                })
                .await?;
            if let Some(reason) = chapter.review_reason.as_ref() {
                review_count += 1;
                self.persist_structural_review_item(
                    session_id,
                    reason,
                    format!("Review chapter {} structure", chapter.chapter_number),
                    Some(chapter.confidence),
                    vec![chapter_segment.id.clone()],
                    Some(ImportCorrectionPayload::Structural {
                        chapter_number: Some(chapter.chapter_number),
                        scene_order: None,
                        pov_character_name: None,
                        segment_ids: vec![chapter_segment.id.clone()],
                    }),
                )
                .await?;
            }

            let mut scenes = Vec::with_capacity(chapter.scenes.len());
            for scene in &chapter.scenes {
                let scene_segment = self
                    .repository
                    .upsert_import_segment(UpsertImportSegmentParams {
                        session_id: session_id.to_string(),
                        source_document_id: persisted_document.id.clone(),
                        parent_segment_id: Some(chapter_segment.id.clone()),
                        segment_type: "scene".to_string(),
                        source_order: scene.start_offset,
                        book_number: Some(chapter.book_number),
                        chapter_number: Some(chapter.chapter_number),
                        scene_order: Some(scene.scene_index as i32),
                        label: scene.label.clone(),
                        start_offset: scene.start_offset,
                        end_offset: scene.end_offset,
                        word_count: scene.word_count,
                        character_count: scene.character_count,
                        pov_guess: scene
                            .pov_guess
                            .as_ref()
                            .map(serde_json::to_value)
                            .transpose()?,
                        confidence: scene.confidence,
                        segment_status: import_segment_status_name(),
                    })
                    .await?;

                if let Some(reason) = scene.review_reason.as_ref() {
                    review_count += 1;
                    self.persist_structural_review_item(
                        session_id,
                        reason,
                        format!(
                            "Review chapter {} scene {} structure",
                            chapter.chapter_number, scene.scene_index
                        ),
                        Some(scene.confidence),
                        vec![chapter_segment.id.clone(), scene_segment.id.clone()],
                        Some(ImportCorrectionPayload::Structural {
                            chapter_number: Some(chapter.chapter_number),
                            scene_order: Some(scene.scene_index as i32),
                            pov_character_name: scene
                                .pov_guess
                                .as_ref()
                                .and_then(|guess| guess.character_name.clone()),
                            segment_ids: vec![chapter_segment.id.clone(), scene_segment.id.clone()],
                        }),
                    )
                    .await?;
                }

                scenes.push(ImportSceneSlice {
                    segment_id: scene_segment.id.clone(),
                    chapter_segment_id: Some(chapter_segment.id.clone()),
                    scene_index: scene.scene_index,
                    label: scene.label.clone(),
                    start_offset: scene.start_offset,
                    end_offset: scene.end_offset,
                    word_count: scene.word_count,
                    character_count: scene.character_count,
                    pov_guess: scene.pov_guess.clone(),
                    confidence: scene.confidence,
                    confidence_level: scene.confidence_level.clone(),
                });
            }

            chapters.push(ImportChapterSlice {
                segment_id: chapter_segment.id.clone(),
                book_number: chapter.book_number,
                chapter_number: chapter.chapter_number,
                title: chapter.title.clone(),
                start_offset: chapter.start_offset,
                end_offset: chapter.end_offset,
                word_count: chapter.word_count,
                confidence: chapter.confidence,
                confidence_level: chapter.confidence_level.clone(),
                scenes,
            });
        }

        Ok((
            ImportStructuralAnalysisSummary {
                source_documents,
                chapters,
                review_items_created: review_count,
            },
            review_count,
        ))
    }

    pub(super) async fn persist_structural_review_item(
        &self,
        session_id: &str,
        description: &str,
        title: String,
        confidence: Option<f64>,
        related_segment_ids: Vec<String>,
        correction: Option<spindle_core::models::ImportCorrectionPayload>,
    ) -> Result<()> {
        use crate::sqlite::import_service::{
            import_pass_name, import_review_item_kind_name, import_review_severity_name,
            import_review_status_name,
        };
        use crate::sqlite::repository::CreateImportReviewItemParams;
        use spindle_core::models::{
            ImportPassName, ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus,
        };

        self.repository
            .create_import_review_item(CreateImportReviewItemParams {
                session_id: session_id.to_string(),
                pass_name: import_pass_name(&ImportPassName::StructuralAnalysis),
                item_kind: import_review_item_kind_name(&ImportReviewItemKind::Structure),
                severity: import_review_severity_name(&ImportReviewSeverity::RequiresReview),
                status: import_review_status_name(&ImportReviewStatus::Open),
                title,
                description: description.to_string(),
                related_segment_ids,
                related_entity_ids: Vec::new(),
                confidence,
                proposed_correction: correction.map(serde_json::to_value).transpose()?,
                resolver_notes: None,
            })
            .await?;
        Ok(())
    }

    /// The full import-hydrate-bible implementation. Split out as a
    /// separate method to keep the surface stub `import_hydrate_bible`
    /// concise; the body itself is large because it walks every bible
    /// section the import pipeline can populate.
    ///
    /// Implementation note:
    ///
    /// * Existing relationships are updated in place via
    ///   `Repository::set_relationship_absolute`, which writes absolute
    ///   `trust` / `tension` values (distinct from `update_relationship`,
    ///   which applies deltas).
    /// * The "look up canonical book and chapter numbers per manuscript
    ///   book" logic uses `list_chapters_by_book_number` (project +
    ///   book_number natural key) instead of `list_chapters_by_book(id)`
    ///   because the SQLite chapter table is keyed by `book_id` rather
    ///   than book record-id round-trips.
    pub(super) async fn import_hydrate_bible_inner(
        &self,
        input: spindle_core::models::ImportHydrateBibleInput,
    ) -> Result<spindle_core::models::ImportHydrateBibleOutput> {
        use crate::sqlite::import_service::{
            character_dossier_summaries_from_records, detect_content_rating,
            find_scene_text_summary, hydration_mapped_book, hydration_mapped_chapter,
            import_pass_name, import_progress, import_review_item_kind_name,
            import_review_severity_name, import_review_status_name, import_session_status_name,
            imported_character_role, narrative_dossier_summary_from_record,
            resume_snapshot_summary_from_record, structural_summary_from_records,
            summarize_character_summary, summarize_text, world_dossier_summary_from_record,
        };
        use crate::sqlite::repository::{
            AppendCharacterStateParams, CreateImportReviewItemParams, UpsertKnowledgeFactParams,
        };
        use spindle_core::models::{
            CharacterStatePatch, CreateBookInput, CreateChapterInput, CreateCharacterArcInput,
            CreateCharacterInput, CreateConflictInput, CreateEconomyInput, CreateFactionInput,
            CreateFutureKnowledgeInput, CreateLocationInput, CreateMotifInput,
            CreateNarrativePromiseInput, CreatePacingConfigInput, CreatePacingCurveInput,
            CreatePlotLineInput, CreateRelationshipInput, CreateReligionInput,
            CreateSystemOverlayInput, CreateTermInput, CreateThemeInput, CreateTimelineEventInput,
            CreateWorldRuleInput, EstablishedIn, ImportChapterSlice, ImportConfidenceLevel,
            ImportCorrectionPayload, ImportEntityKind, ImportHydrationRecordCount,
            ImportHydrationReport, ImportHydrationStatus, ImportPassName, ImportReviewItemKind,
            ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus, SaveSceneDraftInput,
            SaveSummaryInput, StoryPlacement, WorldStateInput, normalize_name,
        };
        use std::collections::{BTreeMap, BTreeSet};

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let existing_report = session
            .hydration_report
            .clone()
            .map(serde_json::from_value)
            .transpose()?;

        let (target_project, target_branch_id, created_project) =
            self.resolve_hydration_target(&input, &session).await?;
        if self
            .repository
            .hydration_target_exists(&input.session_id, &target_project.id, &target_branch_id)
            .await?
            && let Some(report) = existing_report
        {
            return Ok(spindle_core::models::ImportHydrateBibleOutput {
                session_id: input.session_id,
                report,
            });
        }

        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let mut structure = structural_summary_from_records(&source_documents, &segments, 0)?;
        let character_dossiers = character_dossier_summaries_from_records(
            &self
                .repository
                .list_import_character_dossiers(&input.session_id)
                .await?,
        )?;
        let world = self
            .repository
            .find_import_world_dossier(&input.session_id)
            .await?
            .map(world_dossier_summary_from_record)
            .transpose()?;
        let mut narrative = self
            .repository
            .find_import_narrative_dossier(&input.session_id)
            .await?
            .map(narrative_dossier_summary_from_record)
            .transpose()?;
        let mut final_state = self
            .repository
            .find_import_resume_snapshot(&input.session_id)
            .await?
            .map(resume_snapshot_summary_from_record)
            .transpose()?;
        let review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;

        let target_project_id = target_project.id.clone();
        let main_branch_id = self
            .resolve_project_main_branch(&target_project, "bible_branch:main")
            .await;

        let mut record_counts = Vec::new();
        let mut skipped_sections = Vec::new();
        let mut notes = Vec::new();

        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::Hydration),
                serde_json::to_value(import_progress(
                    structure.source_documents.len(),
                    structure.source_documents.len(),
                    structure
                        .chapters
                        .iter()
                        .map(|chapter| chapter.scenes.len())
                        .sum(),
                    0,
                    review_items.len(),
                    review_items
                        .iter()
                        .filter(|item| {
                            item.status == import_review_status_name(&ImportReviewStatus::Open)
                        })
                        .count(),
                ))?,
                &import_session_status_name(&ImportSessionStatus::Running),
            )
            .await?;

        // ----- books -----
        let existing_books = self
            .repository
            .list_books_by_project(&target_project.id)
            .await?;
        let mut book_number_map: BTreeMap<i32, i32> = BTreeMap::new();
        for book in &existing_books {
            book_number_map.insert(book.book_number, book.book_number);
        }
        let mut created_books = 0usize;
        let mut skipped_books = 0usize;
        let manuscript_book_numbers: BTreeSet<i32> =
            structure.chapters.iter().map(|c| c.book_number).collect();
        for &ms_book in &manuscript_book_numbers {
            if book_number_map.contains_key(&ms_book) {
                skipped_books += 1;
                continue;
            }
            let created = self
                .create_book(CreateBookInput {
                    project_id: target_project_id.clone(),
                    title: if manuscript_book_numbers.len() == 1 {
                        Some("Imported Manuscript".to_string())
                    } else {
                        Some(format!("Imported Book {}", ms_book))
                    },
                })
                .await?;
            book_number_map.insert(ms_book, created.book_number);
            created_books += 1;
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "book".to_string(),
            created: created_books,
            updated: 0,
            skipped: skipped_books,
        });

        // ----- chapters -----
        let mut chapter_number_map: BTreeMap<(i32, i32), i32> = BTreeMap::new();
        let mut created_chapters = 0usize;
        let mut skipped_chapters = 0usize;
        let mut chapters_by_book: BTreeMap<i32, Vec<&ImportChapterSlice>> = BTreeMap::new();
        for chapter in &structure.chapters {
            chapters_by_book
                .entry(chapter.book_number)
                .or_default()
                .push(chapter);
        }
        for (&ms_book_number, manuscript_chapters) in &chapters_by_book {
            let Some(&canonical_book) = book_number_map.get(&ms_book_number) else {
                continue;
            };
            let existing_db_chapters = self
                .repository
                .list_chapters_by_book_number(&target_project.id, canonical_book)
                .await?;
            let to_create = manuscript_chapters
                .len()
                .saturating_sub(existing_db_chapters.len());
            for i in 0..to_create {
                let ms_idx = existing_db_chapters.len() + i;
                self.create_chapter(CreateChapterInput {
                    project_id: target_project_id.clone(),
                    book_number: Some(canonical_book),
                    book_id: None,
                    chapter_number: None,
                    title: manuscript_chapters
                        .get(ms_idx)
                        .and_then(|c| c.title.clone()),
                })
                .await?;
                created_chapters += 1;
            }
            skipped_chapters += existing_db_chapters.len().min(manuscript_chapters.len());
            let final_db_chapters = self
                .repository
                .list_chapters_by_book_number(&target_project.id, canonical_book)
                .await?;
            for (ms_ch, db_ch) in manuscript_chapters.iter().zip(final_db_chapters.iter()) {
                chapter_number_map.insert(
                    (ms_ch.book_number, ms_ch.chapter_number),
                    db_ch.chapter_number,
                );
            }
        }

        for chapter in &mut structure.chapters {
            let orig_book = chapter.book_number;
            let orig_chapter = chapter.chapter_number;
            chapter.book_number = hydration_mapped_book(&book_number_map, orig_book);
            chapter.chapter_number =
                hydration_mapped_chapter(&chapter_number_map, orig_book, orig_chapter);
        }
        if let Some(ref mut fs) = final_state {
            let orig_book = fs.book_number;
            let orig_chapter = fs.chapter_number;
            fs.book_number = hydration_mapped_book(&book_number_map, orig_book);
            fs.chapter_number =
                hydration_mapped_chapter(&chapter_number_map, orig_book, orig_chapter);
        }
        if let Some(ref mut narr) = narrative {
            for hint in &mut narr.pacing_hints {
                hint.book_number = hint
                    .book_number
                    .map(|b| hydration_mapped_book(&book_number_map, b));
            }
        }

        record_counts.push(ImportHydrationRecordCount {
            entity_type: "chapter".to_string(),
            created: created_chapters,
            updated: 0,
            skipped: skipped_chapters,
        });

        // ----- characters -----
        let mut characters_by_cluster = BTreeMap::new();
        let mut canonical_characters = self
            .repository
            .list_characters_by_project_and_branch(&target_project.id, &main_branch_id)
            .await?;
        let mut created_characters = 0usize;
        let mut skipped_characters = 0usize;
        for dossier in &character_dossiers {
            if let Some(existing) = canonical_characters.iter().find(|character| {
                character.normalized_name == normalize_name(&dossier.canonical_name)
            }) {
                characters_by_cluster.insert(dossier.cluster_id.clone(), existing.id.clone());
                skipped_characters += 1;
                continue;
            }

            let initial_state = dossier
                .state_trajectory
                .first()
                .map(|point| CharacterStatePatch {
                    emotional_state: point.emotional_state.clone(),
                    goals: Some(point.goals.clone()),
                    status: Some(point.status.clone()),
                    notes: Some(vec![point.summary.clone()]),
                    source_summary: Some("import baseline".to_string()),
                });
            let created = self
                .create_character(CreateCharacterInput {
                    aliases: Vec::new(),
                    project_id: target_project_id.clone(),
                    name: dossier.canonical_name.clone(),
                    summary: summarize_character_summary(dossier),
                    role: imported_character_role(dossier),
                    realm: None,
                    voice_profile: dossier.voice_profile.clone(),
                    emotional_profile: dossier.emotional_profile.clone(),
                    initial_state,
                })
                .await?;
            let character_id = created.character_id.clone();
            characters_by_cluster.insert(dossier.cluster_id.clone(), character_id.clone());
            canonical_characters.push(self.repository.get_character(&character_id).await?);
            created_characters += 1;
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "character".to_string(),
            created: created_characters,
            updated: 0,
            skipped: skipped_characters,
        });

        // ----- final character state snapshot -----
        let mut final_character_state_created = 0usize;
        let mut final_character_state_skipped = 0usize;
        if let Some(final_state) = final_state.as_ref() {
            let existing = self
                .repository
                .list_character_states_by_project_and_branch(&target_project.id, &target_branch_id)
                .await?;
            for snapshot in &final_state.characters {
                let Some(character_id) = characters_by_cluster.get(&snapshot.cluster_id) else {
                    continue;
                };
                if existing.iter().any(|state| {
                    &state.character_id == character_id
                        && state.book_number == final_state.book_number
                        && state.chapter_number == final_state.chapter_number
                        && state.scene_order == final_state.scene_order.unwrap_or(0)
                }) {
                    final_character_state_skipped += 1;
                    continue;
                }
                self.repository
                    .append_character_state(AppendCharacterStateParams {
                        project_id: target_project.id.clone(),
                        branch_id: target_branch_id.clone(),
                        character_id: character_id.clone(),
                        scene_id: None,
                        book_number: final_state.book_number,
                        chapter_number: final_state.chapter_number,
                        scene_order: final_state.scene_order.unwrap_or(0),
                        patch: CharacterStatePatch {
                            emotional_state: snapshot.emotional_state.clone(),
                            goals: Some(snapshot.goals.clone()),
                            status: Some(snapshot.status.clone()),
                            notes: Some(snapshot.notes.clone()),
                            source_summary: Some("import final state".to_string()),
                        },
                    })
                    .await?;
                final_character_state_created += 1;
            }
        } else {
            skipped_sections.push("character_final_state".to_string());
            notes.push(
                "Skipped final character state hydration because no resume snapshot was available."
                    .to_string(),
            );
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "character_state".to_string(),
            created: final_character_state_created,
            updated: 0,
            skipped: final_character_state_skipped,
        });

        // ----- locations -----
        let mut locations_by_name: BTreeMap<String, String> = BTreeMap::new();
        let mut canonical_locations = self
            .repository
            .list_locations_by_project_and_branch(&target_project.id, &main_branch_id)
            .await?;
        let mut created_locations = 0usize;
        let mut skipped_locations = 0usize;
        if let Some(world) = world.as_ref() {
            for location in &world.locations {
                if let Some(existing) = canonical_locations
                    .iter()
                    .find(|candidate| candidate.normalized_name == normalize_name(&location.name))
                {
                    locations_by_name.insert(normalize_name(&location.name), existing.id.clone());
                    skipped_locations += 1;
                    continue;
                }
                let snapshot = final_state.as_ref().and_then(|state| {
                    state.locations.iter().find(|candidate| {
                        normalize_name(&candidate.location_name) == normalize_name(&location.name)
                    })
                });
                let created = self
                    .create_location(CreateLocationInput {
                        project_id: target_project_id.clone(),
                        name: location.name.clone(),
                        kind: location.kind.clone(),
                        realm: location.realm.clone(),
                        summary: location.summary.clone(),
                        initial_state: WorldStateInput {
                            controlling_faction: snapshot
                                .and_then(|item| item.controlling_faction.clone()),
                            status: snapshot.and_then(|item| item.status.clone()),
                            prosperity: snapshot.and_then(|item| item.prosperity.clone()),
                            stability: snapshot.and_then(|item| item.stability.clone()),
                            threat_level: snapshot.and_then(|item| item.threat_level.clone()),
                            sensory_details: snapshot
                                .map(|item| item.sensory_details.clone())
                                .unwrap_or_default(),
                        },
                    })
                    .await?;
                locations_by_name
                    .insert(normalize_name(&location.name), created.location_id.clone());
                canonical_locations.push(self.repository.get_location(&created.location_id).await?);
                created_locations += 1;
            }
        } else {
            skipped_sections.push("world".to_string());
            notes.push(
                "Skipped location hydration because no world dossier was available.".to_string(),
            );
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "location".to_string(),
            created: created_locations,
            updated: 0,
            skipped: skipped_locations,
        });

        // ----- world rules -----
        let mut created_world_rules = 0usize;
        let mut skipped_world_rules = 0usize;
        if let Some(world) = world.as_ref() {
            let existing = self
                .repository
                .list_world_rules_by_project(&target_project.id)
                .await?;
            for rule in &world.world_rules {
                if existing.iter().any(|candidate| {
                    normalize_name(&candidate.rule_name) == normalize_name(&rule.rule_name)
                }) {
                    skipped_world_rules += 1;
                    continue;
                }
                self.create_world_rule(CreateWorldRuleInput {
                    project_id: target_project_id.clone(),
                    rule_name: rule.rule_name.clone(),
                    rule_type: rule.rule_type.clone(),
                    description: rule.description.clone(),
                    scan_pattern: None,
                    relevance_tags: vec![],
                    established_in: rule
                        .source_segment_ids
                        .first()
                        .and_then(|_| structure.chapters.first())
                        .map(|chapter| EstablishedIn {
                            book_number: chapter.book_number,
                            chapter_number: chapter.chapter_number,
                            note: Some("imported from manuscript world analysis".to_string()),
                        }),
                })
                .await?;
                created_world_rules += 1;
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "world_rule".to_string(),
            created: created_world_rules,
            updated: 0,
            skipped: skipped_world_rules,
        });

        // ----- factions/religions/economies/terms -----
        let mut created_factions = 0usize;
        let mut skipped_factions = 0usize;
        let mut created_religions = 0usize;
        let mut skipped_religions = 0usize;
        let mut created_economies = 0usize;
        let mut skipped_economies = 0usize;
        let mut created_terms = 0usize;
        let mut skipped_terms = 0usize;
        if let Some(world) = world.as_ref() {
            let existing_factions = self
                .repository
                .list_factions_by_project(&target_project.id)
                .await?;
            let existing_religions = self
                .repository
                .list_religions_by_project(&target_project.id)
                .await?;
            let existing_economies = self
                .repository
                .list_economies_by_project(&target_project.id)
                .await?;
            let existing_terms = self
                .repository
                .list_terms_by_project(&target_project.id)
                .await?;
            for entity in &world.entities {
                match entity.entity_kind {
                    ImportEntityKind::Faction => {
                        if existing_factions.iter().any(|candidate| {
                            candidate.normalized_name == normalize_name(&entity.canonical_name)
                        }) {
                            skipped_factions += 1;
                        } else {
                            self.create_faction(CreateFactionInput {
                                project_id: target_project_id.clone(),
                                name: entity.canonical_name.clone(),
                                faction_type: entity
                                    .tags
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| "imported".to_string()),
                                realm: entity.realm.clone(),
                                summary: entity.summary.clone(),
                                tags: entity.tags.clone(),
                            })
                            .await?;
                            created_factions += 1;
                        }
                    }
                    ImportEntityKind::Religion => {
                        if existing_religions.iter().any(|candidate| {
                            candidate.normalized_name == normalize_name(&entity.canonical_name)
                        }) {
                            skipped_religions += 1;
                        } else {
                            self.create_religion(CreateReligionInput {
                                project_id: target_project_id.clone(),
                                name: entity.canonical_name.clone(),
                                deity_or_principle: entity
                                    .tags
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| "imported doctrine".to_string()),
                                summary: entity.summary.clone(),
                                tags: entity.tags.clone(),
                            })
                            .await?;
                            created_religions += 1;
                        }
                    }
                    ImportEntityKind::Economy => {
                        if existing_economies.iter().any(|candidate| {
                            candidate.normalized_name == normalize_name(&entity.canonical_name)
                        }) {
                            skipped_economies += 1;
                        } else {
                            self.create_economy(CreateEconomyInput {
                                project_id: target_project_id.clone(),
                                name: entity.canonical_name.clone(),
                                realm: entity.realm.clone(),
                                summary: entity.summary.clone(),
                                scarce_resources: Vec::new(),
                                trade_goods: entity.tags.clone(),
                                currency: None,
                                notes: vec!["imported from manuscript world analysis".to_string()],
                            })
                            .await?;
                            created_economies += 1;
                        }
                    }
                    ImportEntityKind::Term => {
                        if existing_terms.iter().any(|candidate| {
                            candidate.normalized_term == normalize_name(&entity.canonical_name)
                        }) {
                            skipped_terms += 1;
                        } else {
                            self.create_term(CreateTermInput {
                                project_id: target_project_id.clone(),
                                term_text: entity.canonical_name.clone(),
                                pronunciation: None,
                                definition: entity.summary.clone(),
                                usage_context: entity.realm.clone(),
                                origin: Some("imported manuscript glossary".to_string()),
                            })
                            .await?;
                            created_terms += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "faction".to_string(),
            created: created_factions,
            updated: 0,
            skipped: skipped_factions,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "religion".to_string(),
            created: created_religions,
            updated: 0,
            skipped: skipped_religions,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "economy".to_string(),
            created: created_economies,
            updated: 0,
            skipped: skipped_economies,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "term".to_string(),
            created: created_terms,
            updated: 0,
            skipped: skipped_terms,
        });

        // ----- themes / plot lines / conflicts / promises / motifs / arcs -----
        let mut theme_ids_by_statement: BTreeMap<String, String> = BTreeMap::new();
        let mut created_themes = 0usize;
        let mut skipped_themes = 0usize;
        let mut created_plot_lines = 0usize;
        let mut skipped_plot_lines = 0usize;
        let mut created_conflicts = 0usize;
        let mut skipped_conflicts = 0usize;
        let mut created_promises = 0usize;
        let mut skipped_promises = 0usize;
        let mut created_motifs = 0usize;
        let mut skipped_motifs = 0usize;
        let mut created_arcs = 0usize;
        let mut skipped_arcs = 0usize;
        if let Some(narrative) = narrative.as_ref() {
            if narrative
                .plot_lines
                .iter()
                .any(|plot_line| !plot_line.convergence_points.is_empty())
            {
                notes.push(
                    "Skipped imported plot-line convergence points during hydration because canonical plot_line storage does not yet accept the imported placement shape."
                        .to_string(),
                );
            }
            let existing_themes = self
                .repository
                .list_themes_by_project(&target_project.id)
                .await?;
            for theme in &narrative.themes {
                if let Some(existing) = existing_themes
                    .iter()
                    .find(|candidate| candidate.theme_statement == theme.theme_statement)
                {
                    theme_ids_by_statement
                        .insert(theme.theme_statement.clone(), existing.id.clone());
                    skipped_themes += 1;
                    continue;
                }
                let created = self
                    .create_theme(CreateThemeInput {
                        project_id: target_project_id.clone(),
                        theme_statement: theme.theme_statement.clone(),
                        thesis_antithesis: theme.thesis_antithesis.clone(),
                        introduction_point: structure.chapters.first().map(|chapter| {
                            StoryPlacement {
                                book_number: chapter.book_number,
                                chapter_number: chapter.chapter_number,
                                scene_order: chapter
                                    .scenes
                                    .first()
                                    .map(|scene| scene.scene_index as i32),
                                note: None,
                            }
                        }),
                        resolution_point: final_state.as_ref().map(|state| StoryPlacement {
                            book_number: state.book_number,
                            chapter_number: state.chapter_number,
                            scene_order: state.scene_order,
                            note: None,
                        }),
                    })
                    .await?;
                theme_ids_by_statement.insert(theme.theme_statement.clone(), created.theme_id);
                created_themes += 1;
            }

            let existing_plot_lines = self
                .repository
                .list_plot_lines_by_project(&target_project.id)
                .await?;
            for plot_line in &narrative.plot_lines {
                if existing_plot_lines
                    .iter()
                    .any(|candidate| candidate.normalized_name == normalize_name(&plot_line.name))
                {
                    skipped_plot_lines += 1;
                    continue;
                }
                let _ = self
                    .create_plot_line(CreatePlotLineInput {
                        project_id: target_project_id.clone(),
                        name: plot_line.name.clone(),
                        plot_type: plot_line.plot_type.clone(),
                        summary: plot_line.summary.clone(),
                        status: plot_line.status.clone(),
                        convergence_points: Vec::new(),
                        connected_conflict_ids: Vec::new(),
                        connected_theme_ids: Vec::new(),
                    })
                    .await?;
                created_plot_lines += 1;
            }

            let existing_conflicts = self
                .repository
                .list_conflicts_by_project(&target_project.id)
                .await?;
            for conflict in &narrative.conflicts {
                if existing_conflicts
                    .iter()
                    .any(|candidate| candidate.normalized_name == normalize_name(&conflict.name))
                {
                    skipped_conflicts += 1;
                    continue;
                }
                let _ = self
                    .create_conflict(CreateConflictInput {
                        project_id: target_project_id.clone(),
                        name: conflict.name.clone(),
                        conflict_type: conflict.conflict_type.clone(),
                        stakes: conflict.stakes.clone(),
                        escalation_stages: conflict.escalation_stages.clone(),
                        expected_total_cycles: Some(conflict.try_fail_cycles.len() as i32),
                        try_fail_cycles: conflict.try_fail_cycles.clone(),
                        stated_consequences: conflict.stated_consequences.clone(),
                    })
                    .await?;
                created_conflicts += 1;
            }

            let existing_promises = self
                .repository
                .list_narrative_promises_by_project(&target_project.id)
                .await?;
            for promise in &narrative.narrative_promises {
                if existing_promises
                    .iter()
                    .any(|candidate| candidate.description == promise.description)
                {
                    skipped_promises += 1;
                    continue;
                }
                self.create_narrative_promise(CreateNarrativePromiseInput {
                    project_id: target_project_id.clone(),
                    promise_type: promise.promise_type.clone(),
                    description: promise.description.clone(),
                    planted_at: promise.planted_at.clone(),
                    planned_payoff: promise.planned_payoff.clone(),
                    notes: promise.notes.clone(),
                })
                .await?;
                created_promises += 1;
            }

            let existing_motifs = self
                .repository
                .list_motifs_by_project(&target_project.id)
                .await?;
            for motif in &narrative.motifs {
                if existing_motifs
                    .iter()
                    .any(|candidate| candidate.normalized_name == normalize_name(&motif.name))
                {
                    skipped_motifs += 1;
                    continue;
                }
                let connected_theme_ids = motif
                    .connected_theme_statements
                    .iter()
                    .filter_map(|statement| theme_ids_by_statement.get(statement).cloned())
                    .collect::<Vec<_>>();
                self.create_motif(CreateMotifInput {
                    project_id: target_project_id.clone(),
                    name: motif.name.clone(),
                    description: motif.description.clone(),
                    max_uses_per_chapter: None,
                    connected_theme_ids,
                })
                .await?;
                created_motifs += 1;
            }

            let existing_arcs = self
                .repository
                .list_character_arcs_by_project(&target_project.id)
                .await?;
            for arc in &narrative.arcs {
                let Some(character_id) = characters_by_cluster.get(&arc.character_cluster_id)
                else {
                    skipped_arcs += 1;
                    continue;
                };
                if existing_arcs
                    .iter()
                    .any(|candidate| &candidate.character_id == character_id)
                {
                    skipped_arcs += 1;
                    continue;
                }
                let connected_theme_ids =
                    theme_ids_by_statement.values().cloned().collect::<Vec<_>>();
                self.create_character_arc(CreateCharacterArcInput {
                    project_id: target_project_id.clone(),
                    character_id: character_id.clone(),
                    arc_type: arc.arc_type.clone(),
                    starting_state: arc.starting_state.clone(),
                    ending_state: arc.ending_state.clone(),
                    milestones: arc.milestones.clone(),
                    thematic_purpose: arc.thematic_purpose.clone(),
                    connected_theme_ids,
                })
                .await?;
                created_arcs += 1;
            }
        } else {
            skipped_sections.push("narrative".to_string());
            notes.push(
                "Skipped narrative hydration because no narrative dossier was available."
                    .to_string(),
            );
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "theme".to_string(),
            created: created_themes,
            updated: 0,
            skipped: skipped_themes,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "plot_line".to_string(),
            created: created_plot_lines,
            updated: 0,
            skipped: skipped_plot_lines,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "conflict".to_string(),
            created: created_conflicts,
            updated: 0,
            skipped: skipped_conflicts,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "narrative_promise".to_string(),
            created: created_promises,
            updated: 0,
            skipped: skipped_promises,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "motif".to_string(),
            created: created_motifs,
            updated: 0,
            skipped: skipped_motifs,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "character_arc".to_string(),
            created: created_arcs,
            updated: 0,
            skipped: skipped_arcs,
        });

        // ----- pacing -----
        let mut created_pacing_configs = 0usize;
        let mut skipped_pacing_configs = 0usize;
        let mut created_pacing_curves = 0usize;
        let mut skipped_pacing_curves = 0usize;
        let mut created_chapter_summaries = 0usize;
        let mut skipped_chapter_summaries = 0usize;
        if let Some(narrative) = narrative.as_ref() {
            let high_confidence_pacing = narrative
                .pacing_hints
                .iter()
                .filter(|hint| matches!(hint.confidence_level, ImportConfidenceLevel::High))
                .collect::<Vec<_>>();
            if high_confidence_pacing.is_empty() {
                skipped_sections.push("pacing".to_string());
                notes.push(
                    "Skipped pacing hydration because imported pacing evidence was not high-confidence."
                        .to_string(),
                );
            } else {
                let existing_configs = self
                    .repository
                    .list_pacing_configs_by_project(&target_project.id)
                    .await?;
                if existing_configs.is_empty() {
                    let avg_chapters = if structure.chapters.is_empty() {
                        1
                    } else {
                        structure.chapters.len() as i32
                    };
                    let avg_scenes = if structure.chapters.is_empty() {
                        1
                    } else {
                        (structure
                            .chapters
                            .iter()
                            .map(|chapter| chapter.scenes.len())
                            .sum::<usize>()
                            / structure.chapters.len().max(1)) as i32
                    };
                    self.create_pacing_config(CreatePacingConfigInput {
                        project_id: target_project_id.clone(),
                        total_planned_books: structure
                            .chapters
                            .iter()
                            .map(|chapter| chapter.book_number)
                            .max()
                            .unwrap_or(1),
                        avg_chapters_per_book: avg_chapters.max(1),
                        avg_scenes_per_chapter: avg_scenes.max(1),
                        tension_model: narrative.reader_contract.reader_contract.promise.clone(),
                    })
                    .await?;
                    created_pacing_configs += 1;
                } else {
                    skipped_pacing_configs += 1;
                }

                let existing_curves = self
                    .repository
                    .list_pacing_curves_by_project(&target_project.id)
                    .await?;
                for hint in high_confidence_pacing {
                    let Some(book_number) = hint.book_number else {
                        skipped_pacing_curves += 1;
                        continue;
                    };
                    if existing_curves
                        .iter()
                        .any(|curve| curve.book_number == book_number)
                    {
                        skipped_pacing_curves += 1;
                        continue;
                    }
                    self.create_pacing_curve(CreatePacingCurveInput {
                        project_id: target_project_id.clone(),
                        book_number,
                        act_breakpoints: hint.act_breakpoints.clone(),
                        scene_type_density: hint.scene_type_density.clone(),
                        intensity_points: Vec::new(),
                    })
                    .await?;
                    created_pacing_curves += 1;
                }

                let existing_summaries = self
                    .repository
                    .list_chapter_summaries_by_project(&target_project.id)
                    .await?;
                for chapter in &structure.chapters {
                    if existing_summaries.iter().any(|summary| {
                        summary.book_number == chapter.book_number
                            && summary.chapter_number == chapter.chapter_number
                    }) {
                        skipped_chapter_summaries += 1;
                        continue;
                    }
                    let scene_summaries = chapter
                        .scenes
                        .iter()
                        .map(|scene| find_scene_text_summary(&source_documents, &segments, scene))
                        .collect::<Result<Vec<_>>>()?;
                    self.save_summary(SaveSummaryInput {
                        project_id: target_project_id.clone(),
                        book_number: chapter.book_number,
                        chapter_number: chapter.chapter_number,
                        entity_type: None,
                        entity_id: None,
                        summary: scene_summaries.join(" "),
                        key_events: scene_summaries.clone(),
                        character_changes: Vec::new(),
                        relationship_shifts: Vec::new(),
                        arc_advances: Vec::new(),
                        promise_events: Vec::new(),
                    })
                    .await?;
                    created_chapter_summaries += 1;
                }
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "pacing_config".to_string(),
            created: created_pacing_configs,
            updated: 0,
            skipped: skipped_pacing_configs,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "pacing_curve".to_string(),
            created: created_pacing_curves,
            updated: 0,
            skipped: skipped_pacing_curves,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "chapter_summary".to_string(),
            created: created_chapter_summaries,
            updated: 0,
            skipped: skipped_chapter_summaries,
        });

        // ----- scenes -----
        let mut created_scenes = 0usize;
        let mut updated_scenes = 0usize;
        let mut skipped_scenes = 0usize;
        if input.include_scenes {
            for chapter in &structure.chapters {
                let reliable_scene_segmentation = !chapter.scenes.is_empty()
                    && chapter.scenes.iter().all(|scene| {
                        matches!(
                            scene.confidence_level,
                            ImportConfidenceLevel::High | ImportConfidenceLevel::Medium
                        )
                    });
                if reliable_scene_segmentation {
                    for scene in &chapter.scenes {
                        let text = crate::sqlite::import_service::scene_text_from_slice(
                            &source_documents,
                            &segments,
                            scene,
                        )?;
                        let summary = summarize_text(&text.replace('\n', " "), 180);
                        let (detected_rating, rating_confidence) = detect_content_rating(&text);
                        let existing_scene = self
                            .repository
                            .find_scene_by_natural_key(
                                &target_project.id,
                                &target_branch_id,
                                chapter.book_number,
                                chapter.chapter_number,
                                scene.scene_index as i32,
                            )
                            .await?;
                        self.save_scene_draft(SaveSceneDraftInput {
                            project_id: target_project_id.clone(),
                            book_number: chapter.book_number,
                            chapter_number: chapter.chapter_number,
                            chapter_id: None,
                            scene_order: scene.scene_index as i32,
                            full_text: text,
                            summary,
                            content_rating: detected_rating.clone(),
                            tone: None,
                            source_path: None,
                            generation_id: None,
                            ..Default::default()
                        })
                        .await?;
                        if rating_confidence < 0.70 {
                            self.repository
                                .create_import_review_item(CreateImportReviewItemParams {
                                    session_id: input.session_id.clone(),
                                    pass_name: import_pass_name(&ImportPassName::Hydration),
                                    item_kind: import_review_item_kind_name(
                                        &ImportReviewItemKind::ContentRating,
                                    ),
                                    severity: import_review_severity_name(
                                        &ImportReviewSeverity::RequiresReview,
                                    ),
                                    status: import_review_status_name(&ImportReviewStatus::Open),
                                    title: format!(
                                        "Review content rating for ch{}.{} scene {}",
                                        chapter.book_number,
                                        chapter.chapter_number,
                                        scene.scene_index
                                    ),
                                    description: format!(
                                        "Detected '{}' with confidence {:.2}. The rating may need manual adjustment.",
                                        detected_rating.as_str(),
                                        rating_confidence
                                    ),
                                    related_segment_ids: vec![scene.segment_id.clone()],
                                    related_entity_ids: Vec::new(),
                                    confidence: Some(rating_confidence),
                                    proposed_correction: Some(serde_json::to_value(
                                        ImportCorrectionPayload::ContentRating {
                                            detected_rating: detected_rating.as_str().to_string(),
                                            confidence: rating_confidence,
                                        },
                                    )?),
                                    resolver_notes: None,
                                })
                                .await?;
                        }
                        if existing_scene.is_some() {
                            updated_scenes += 1;
                        } else {
                            created_scenes += 1;
                        }
                    }
                } else {
                    let existing_scene = self
                        .repository
                        .find_scene_by_natural_key(
                            &target_project.id,
                            &target_branch_id,
                            chapter.book_number,
                            chapter.chapter_number,
                            1,
                        )
                        .await?;
                    let text = crate::sqlite::import_service::chapter_text_from_slice(
                        &source_documents,
                        &segments,
                        chapter,
                    )?;
                    let (detected_rating, rating_confidence) = detect_content_rating(&text);
                    self.save_scene_draft(SaveSceneDraftInput {
                        project_id: target_project_id.clone(),
                        book_number: chapter.book_number,
                        chapter_number: chapter.chapter_number,
                        chapter_id: None,
                        scene_order: 1,
                        full_text: text.clone(),
                        summary: summarize_text(&text.replace('\n', " "), 180),
                        content_rating: detected_rating.clone(),
                        tone: None,
                        source_path: None,
                        generation_id: None,
                        ..Default::default()
                    })
                    .await?;
                    if rating_confidence < 0.70 {
                        self.repository
                            .create_import_review_item(CreateImportReviewItemParams {
                                session_id: input.session_id.clone(),
                                pass_name: import_pass_name(&ImportPassName::Hydration),
                                item_kind: import_review_item_kind_name(
                                    &ImportReviewItemKind::ContentRating,
                                ),
                                severity: import_review_severity_name(
                                    &ImportReviewSeverity::RequiresReview,
                                ),
                                status: import_review_status_name(&ImportReviewStatus::Open),
                                title: format!(
                                    "Review content rating for ch{}.{}",
                                    chapter.book_number, chapter.chapter_number
                                ),
                                description: format!(
                                    "Detected '{}' with confidence {:.2}. The rating may need manual adjustment.",
                                    detected_rating.as_str(),
                                    rating_confidence
                                ),
                                related_segment_ids: chapter
                                    .scenes
                                    .first()
                                    .map(|s| s.segment_id.clone())
                                    .into_iter()
                                    .collect(),
                                related_entity_ids: Vec::new(),
                                confidence: Some(rating_confidence),
                                proposed_correction: Some(serde_json::to_value(
                                    ImportCorrectionPayload::ContentRating {
                                        detected_rating: detected_rating.as_str().to_string(),
                                        confidence: rating_confidence,
                                    },
                                )?),
                                resolver_notes: None,
                            })
                            .await?;
                    }
                    notes.push(format!(
                        "Chapter {}.{} used chapter-level scene hydration because scene segmentation confidence was weak.",
                        chapter.book_number, chapter.chapter_number
                    ));
                    if existing_scene.is_some() {
                        updated_scenes += 1;
                    } else {
                        created_scenes += 1;
                    }
                }
            }
        } else {
            skipped_sections.push("scenes".to_string());
            skipped_scenes = structure.chapters.len();
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "scene".to_string(),
            created: created_scenes,
            updated: updated_scenes,
            skipped: skipped_scenes,
        });

        // ----- relationships -----
        //
        // Import snapshots carry canonical absolute trust/tension. Fresh
        // relationships are created via `create_relationship`; existing
        // ones are reset via `set_relationship_absolute` (which writes
        // absolutes, not deltas). `skipped_relationships` only counts
        // entries whose source/target cluster id never resolved to a
        // hydrated character row.
        let mut created_relationships = 0usize;
        let mut updated_relationships = 0usize;
        let mut skipped_relationships = 0usize;
        if let Some(final_state) = final_state.as_ref() {
            for relationship in &final_state.relationships {
                let Some(source_id) =
                    characters_by_cluster.get(&relationship.source_character_cluster_id)
                else {
                    skipped_relationships += 1;
                    continue;
                };
                let Some(target_id) =
                    characters_by_cluster.get(&relationship.target_character_cluster_id)
                else {
                    skipped_relationships += 1;
                    continue;
                };
                match self
                    .repository
                    .get_relationship(&target_branch_id, source_id, target_id)
                    .await
                {
                    Ok(_existing) => {
                        self.repository
                            .set_relationship_absolute(
                                &target_branch_id,
                                source_id,
                                target_id,
                                relationship.trust,
                                relationship.tension,
                                Some(relationship.summary.clone()),
                                None,
                            )
                            .await?;
                        updated_relationships += 1;
                    }
                    Err(_) => {
                        self.repository
                            .create_relationship(
                                &target_branch_id,
                                &CreateRelationshipInput {
                                    character_a_id: source_id.clone(),
                                    character_b_id: target_id.clone(),
                                    relationship_type: relationship.relationship_type.clone(),
                                    initial_trust: relationship.trust,
                                    initial_tension: relationship.tension,
                                    dynamics: vec![relationship.summary.clone()],
                                },
                            )
                            .await?;
                        created_relationships += 1;
                    }
                }
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "relationship".to_string(),
            created: created_relationships,
            updated: updated_relationships,
            skipped: skipped_relationships,
        });

        // ----- knowledge facts -----
        let mut created_knowledge = 0usize;
        let mut skipped_knowledge = 0usize;
        if let Some(final_state) = final_state.as_ref() {
            let existing_knowledge = self
                .repository
                .list_knowledge_facts_by_project(&target_project.id)
                .await?;
            for snapshot in &final_state.characters {
                let Some(character_id) = characters_by_cluster.get(&snapshot.cluster_id) else {
                    skipped_knowledge += 1;
                    continue;
                };
                for fact in snapshot
                    .notes
                    .iter()
                    .filter(|note| !note.trim().is_empty())
                    .chain(snapshot.status.iter())
                    .take(3)
                {
                    let normalized = normalize_name(fact);
                    if existing_knowledge.iter().any(|candidate| {
                        candidate.character_id == *character_id
                            && candidate.normalized_fact == normalized
                    }) {
                        skipped_knowledge += 1;
                        continue;
                    }
                    self.repository
                        .upsert_knowledge_fact(UpsertKnowledgeFactParams {
                            project_id: target_project.id.clone(),
                            branch_id: target_branch_id.clone(),
                            character_id: character_id.clone(),
                            fact: fact.clone(),
                            normalized_fact: normalized,
                            source_summary: "Imported final-state hydration".to_string(),
                            learned_at: Some(StoryPlacement {
                                book_number: final_state.book_number,
                                chapter_number: final_state.chapter_number,
                                scene_order: final_state.scene_order,
                                note: None,
                            }),
                            confidence: Some(snapshot.confidence),
                            tags: vec!["imported".to_string(), "hydrated".to_string()],
                            reader_visible: true,
                            source_import_session_id: Some(input.session_id.clone()),
                            secret_of_fact_id: None,
                        })
                        .await?;
                    created_knowledge += 1;
                }
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "knowledge_fact".to_string(),
            created: created_knowledge,
            updated: 0,
            skipped: skipped_knowledge,
        });

        // ----- timeline / future knowledge / system overlays -----
        let mut created_future_knowledge = 0usize;
        let mut skipped_future_knowledge = 0usize;
        let mut created_timeline_events = 0usize;
        let mut skipped_timeline_events = 0usize;
        let created_temporal_interventions = 0usize;
        let skipped_temporal_interventions = 0usize;
        let mut created_system_overlays = 0usize;
        let mut skipped_system_overlays = 0usize;
        if let Some(world) = world.as_ref() {
            let high_confidence_systems = world
                .system_signals
                .iter()
                .filter(|signal| matches!(signal.confidence_level, ImportConfidenceLevel::High))
                .collect::<Vec<_>>();
            if high_confidence_systems.is_empty() {
                skipped_sections.push("advanced_records".to_string());
                notes.push(
                    "Skipped advanced temporal/system hydration because evidence was not high-confidence."
                        .to_string(),
                );
            } else {
                let existing_overlays = self
                    .repository
                    .list_system_overlays_by_project(&target_project.id)
                    .await?;
                for signal in &high_confidence_systems {
                    if existing_overlays
                        .iter()
                        .any(|overlay| overlay.normalized_name == normalize_name(&signal.summary))
                    {
                        skipped_system_overlays += 1;
                        continue;
                    }
                    self.create_system_overlay(CreateSystemOverlayInput {
                        project_id: target_project_id.clone(),
                        system_name: summarize_text(&signal.summary, 48),
                        system_type: signal.signal_type.clone(),
                        rules: signal.summary.clone(),
                        visibility: "mixed".to_string(),
                        progression_currency: None,
                        stats: Vec::new(),
                        advancement_tiers: Vec::new(),
                    })
                    .await?;
                    created_system_overlays += 1;
                }
            }
        }
        if let Some(final_state) = final_state.as_ref() {
            let existing_events = self
                .repository
                .list_timeline_events_by_project(&target_project.id)
                .await?;
            if existing_events.is_empty() {
                self.create_timeline_event(CreateTimelineEventInput {
                    project_id: target_project_id.clone(),
                    title: format!(
                        "Import continuation point {}.{}",
                        final_state.book_number, final_state.chapter_number
                    ),
                    event_type: "continuation_point".to_string(),
                    placement: StoryPlacement {
                        book_number: final_state.book_number,
                        chapter_number: final_state.chapter_number,
                        scene_order: final_state.scene_order,
                        note: Some("imported manuscript continuation".to_string()),
                    },
                    summary: final_state.summary.clone(),
                    related_entity_ids: Vec::new(),
                })
                .await?;
                created_timeline_events += 1;
            } else {
                skipped_timeline_events += 1;
            }
            for snapshot in &final_state.characters {
                let displaced = snapshot.notes.iter().any(|note| {
                    normalize_name(note).contains("future")
                        || normalize_name(note).contains("prophecy")
                });
                if !displaced {
                    continue;
                }
                let Some(character_id) = characters_by_cluster.get(&snapshot.cluster_id) else {
                    skipped_future_knowledge += 1;
                    continue;
                };
                self.create_future_knowledge(CreateFutureKnowledgeInput {
                    project_id: target_project_id.clone(),
                    character_id: character_id.clone(),
                    knowledge_summary: snapshot.notes.join(" "),
                    source: "imported manuscript inference".to_string(),
                    learned_at: StoryPlacement {
                        book_number: final_state.book_number,
                        chapter_number: final_state.chapter_number,
                        scene_order: final_state.scene_order,
                        note: None,
                    },
                    expires_at: None,
                    notes: vec!["hydrated from import final-state signals".to_string()],
                })
                .await?;
                created_future_knowledge += 1;
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "future_knowledge".to_string(),
            created: created_future_knowledge,
            updated: 0,
            skipped: skipped_future_knowledge,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "timeline_event".to_string(),
            created: created_timeline_events,
            updated: 0,
            skipped: skipped_timeline_events,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "temporal_intervention".to_string(),
            created: created_temporal_interventions,
            updated: 0,
            skipped: skipped_temporal_interventions,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "system_overlay".to_string(),
            created: created_system_overlays,
            updated: 0,
            skipped: skipped_system_overlays,
        });

        notes.push(format!(
            "Hydration provenance anchored to import session {}.",
            input.session_id
        ));

        let report = ImportHydrationReport {
            session_id: input.session_id.clone(),
            project_id: target_project_id.clone(),
            branch_id: target_branch_id.clone(),
            status: if skipped_sections.is_empty() {
                ImportHydrationStatus::Completed
            } else {
                ImportHydrationStatus::Partial
            },
            created_project,
            target_branch_id: target_branch_id.clone(),
            record_counts,
            skipped_sections,
            review_item_ids: review_items.iter().map(|item| item.id.clone()).collect(),
            notes,
        };

        let _ = self
            .repository
            .update_import_session_hydration_report(
                &input.session_id,
                serde_json::to_value(&report)?,
            )
            .await?;
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::Hydration),
                serde_json::to_value(import_progress(
                    structure.source_documents.len(),
                    structure.source_documents.len(),
                    structure
                        .chapters
                        .iter()
                        .map(|chapter| chapter.scenes.len())
                        .sum(),
                    structure
                        .chapters
                        .iter()
                        .map(|chapter| chapter.scenes.len())
                        .sum(),
                    review_items.len(),
                    review_items
                        .iter()
                        .filter(|item| {
                            item.status == import_review_status_name(&ImportReviewStatus::Open)
                        })
                        .count(),
                ))?,
                &import_session_status_name(&ImportSessionStatus::Hydrated),
            )
            .await?;

        Ok(spindle_core::models::ImportHydrateBibleOutput {
            session_id: input.session_id,
            report,
        })
    }
}
