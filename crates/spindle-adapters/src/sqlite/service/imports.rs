use super::*;

impl SqliteSpindleService {
    // Import pipeline.
    //
    // The 10 stubs that follow implement the import passes against the
    // SQLite backend. They lift the SurrealDB-era service methods with the
    // mechanical `RecordId → String` swap and route through the helpers
    // in `crate::sqlite::import_service` + the pure-logic modules under
    // `crate::sqlite::import`. The `_resolve_*` and `_persist_*` helpers
    // they depend on live further down on this impl.

    pub async fn import_manuscript(
        &self,
        input: spindle_core::models::ImportManuscriptInput,
    ) -> Result<spindle_core::models::ImportManuscriptOutput> {
        use crate::sqlite::import::slicer::IngestSourcesOptions;
        use crate::sqlite::import::{analyze_structure, ingest_sources, sha256_hex};
        use crate::sqlite::import_service::{
            default_import_data_dir, import_hydration_mode_name, import_pass_name, import_progress,
            import_session_status_name, import_session_summary, import_source_format_name,
        };
        use crate::sqlite::repository::CreateImportSessionParams;
        use spindle_core::models::{
            ImportDuplicateStrategy, ImportManuscriptOutput, ImportPassName, ImportSessionStatus,
        };
        use std::collections::BTreeSet;
        use std::path::PathBuf;

        if input.source_paths.is_empty() {
            anyhow::bail!("import requires at least one source path");
        }

        let data_dir = default_import_data_dir();
        let duplicate_strategy = input
            .duplicate_strategy
            .clone()
            .unwrap_or(ImportDuplicateStrategy::Reject);

        let (project, target_branch_id, hydrate_mode) = self.resolve_import_target(&input).await?;
        let source_paths = input
            .source_paths
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();

        let mut existing_hashes = BTreeSet::new();
        if matches!(duplicate_strategy, ImportDuplicateStrategy::Reject) {
            for source_path in &source_paths {
                let bytes = std::fs::read(source_path).with_context(|| {
                    format!("failed to read source file {}", source_path.display())
                })?;
                let original_sha256 = sha256_hex(&bytes);
                if self
                    .repository
                    .import_session_exists_for_source_hash(&original_sha256)
                    .await?
                {
                    existing_hashes.insert(original_sha256);
                }
            }
        }

        let ingested = ingest_sources(
            &source_paths,
            IngestSourcesOptions {
                data_dir: &data_dir,
                existing_source_hashes: &existing_hashes,
                duplicate_strategy: duplicate_strategy.clone(),
                source_format_hint: input.source_format_hint.clone(),
            },
        )?;
        let analysis = analyze_structure(ingested);
        let initial_progress = import_progress(
            analysis.source_documents.len(),
            0,
            analysis.total_segments(),
            0,
            0,
            0,
        );

        let session = self
            .repository
            .create_import_session(CreateImportSessionParams {
                project_id: Some(project.id.clone()),
                target_branch_id: Some(target_branch_id.clone()),
                source_format: input
                    .source_format_hint
                    .as_ref()
                    .map(import_source_format_name),
                active_pass: import_pass_name(&ImportPassName::StructuralAnalysis),
                progress: serde_json::to_value(&initial_progress)?,
                session_status: import_session_status_name(&ImportSessionStatus::Running),
                hydrate_mode: import_hydration_mode_name(&hydrate_mode),
                source_count: analysis.source_documents.len(),
            })
            .await?;

        let (summary, review_count) = self
            .persist_structural_analysis(&session.id, &project.id, &analysis)
            .await?;
        let final_status = if review_count > 0 {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::ReadyToHydrate
        };
        let final_progress = import_progress(
            summary.source_documents.len(),
            summary.source_documents.len(),
            analysis.total_segments(),
            analysis.total_segments(),
            review_count,
            review_count,
        );
        let persisted_session = self
            .repository
            .update_import_session_state(
                &session.id,
                &import_pass_name(&ImportPassName::StructuralAnalysis),
                serde_json::to_value(&final_progress)?,
                &import_session_status_name(&final_status),
            )
            .await?;

        Ok(ImportManuscriptOutput {
            session: import_session_summary(&persisted_session)?,
            structure: summary,
        })
    }

    pub async fn import_status(
        &self,
        input: spindle_core::models::ImportStatusInput,
    ) -> Result<spindle_core::models::ImportStatusOutput> {
        use crate::sqlite::import_service::{
            character_dossier_summaries_from_records, entity_consolidation_report_from_records,
            entity_extraction_report_from_records, import_review_item_summary,
            import_session_summary, narrative_dossier_summary_from_record,
            resume_snapshot_summary_from_record, structural_summary_from_records,
            world_dossier_summary_from_record,
        };
        use spindle_core::models::ImportStatusOutput;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let clusters = self
            .repository
            .list_import_entity_clusters(&input.session_id)
            .await?;
        let character_dossiers = self
            .repository
            .list_import_character_dossiers(&input.session_id)
            .await?;
        let review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let structure = (!source_documents.is_empty() || !segments.is_empty())
            .then(|| {
                structural_summary_from_records(&source_documents, &segments, review_items.len())
            })
            .transpose()?;
        let entity_extraction = (!mentions.is_empty())
            .then(|| entity_extraction_report_from_records(&mentions, &review_items));
        let entity_consolidation = (!clusters.is_empty())
            .then(|| entity_consolidation_report_from_records(&clusters, &review_items));
        let characters = character_dossier_summaries_from_records(&character_dossiers)?;
        let world = self
            .repository
            .find_import_world_dossier(&input.session_id)
            .await?
            .map(world_dossier_summary_from_record)
            .transpose()?;
        let narrative = self
            .repository
            .find_import_narrative_dossier(&input.session_id)
            .await?
            .map(narrative_dossier_summary_from_record)
            .transpose()?;
        let final_state = self
            .repository
            .find_import_resume_snapshot(&input.session_id)
            .await?
            .map(resume_snapshot_summary_from_record)
            .transpose()?;

        Ok(ImportStatusOutput {
            session: import_session_summary(&session)?,
            structure,
            entity_extraction,
            entity_consolidation,
            characters,
            world,
            narrative,
            final_state,
            review_items: review_items
                .into_iter()
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
            hydration_report: session
                .hydration_report
                .clone()
                .map(serde_json::from_value)
                .transpose()?,
        })
    }

    pub async fn import_extract_entities(
        &self,
        input: spindle_core::models::ImportExtractEntitiesInput,
    ) -> Result<spindle_core::models::ImportExtractEntitiesOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::extract::extract_entity_candidates;
        use crate::sqlite::import::prompts::{ImportExtractPrompt, build_entity_extraction_prompt};
        use crate::sqlite::import_service::{
            entity_extraction_report_from_records, import_entity_kind_name, import_pass_name,
            import_progress, import_review_item_kind_name, import_review_item_summary,
            import_review_severity_name, import_review_status_name, import_session_status_name,
            load_segment_text, structural_summary_from_records,
        };
        use crate::sqlite::repository::{
            CreateImportEntityMentionParams, CreateImportReviewItemParams,
        };
        use spindle_core::models::{
            ImportCorrectionPayload, ImportExtractEntitiesOutput, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };
        use std::collections::BTreeSet;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let project = self.repository.get_project(&input.project_id).await?;
        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let structure = structural_summary_from_records(&source_documents, &segments, 0)?;
        let segment_filter = if input.segment_ids.is_empty() {
            None
        } else {
            Some(input.segment_ids.iter().cloned().collect::<BTreeSet<_>>())
        };

        let existing_mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let mut completed_segment_ids = existing_mentions
            .iter()
            .map(|mention| mention.segment_id.clone())
            .collect::<BTreeSet<_>>();
        let mut review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let mut mentions = existing_mentions;
        let mut processed_segments = 0usize;
        let mut extraction_round = mentions.len() + 1;

        for chapter in &structure.chapters {
            for scene in &chapter.scenes {
                let scene_segment_id = scene.segment_id.clone();
                if completed_segment_ids.contains(&scene_segment_id) {
                    continue;
                }
                if let Some(segment_filter) = segment_filter.as_ref()
                    && !segment_filter.contains(&scene_segment_id)
                {
                    continue;
                }
                if input.limit.is_some_and(|limit| processed_segments >= limit) {
                    break;
                }

                let segment_record = segments
                    .iter()
                    .find(|segment| segment.id == scene_segment_id)
                    .context("scene segment not found for extraction")?;
                let source_document = source_documents
                    .iter()
                    .find(|document| document.id == segment_record.source_document_id)
                    .context("source document missing for scene extraction")?;
                let segment_text = load_segment_text(source_document, segment_record)?;

                let prompt = build_entity_extraction_prompt(&ImportExtractPrompt {
                    project_name: Some(&project.name),
                    chapter,
                    scene: Some(scene),
                    text: &segment_text,
                });
                match self
                    .repository
                    .model_router()
                    .complete(&ModelRequest {
                        route: "import_extract".to_string(),
                        prompt,
                        rating: None,
                        context: None,
                    })
                    .await
                {
                    Ok(_model_response) => {}
                    // A provider safety refusal is deterministic for this
                    // segment+provider; retrying won't help and must not abort the
                    // whole import. Surface it as a Warning review item and fall
                    // through to the local (regex) extraction so the segment still
                    // yields what it can. Any other error still propagates.
                    Err(err)
                        if err
                            .downcast_ref::<crate::ai::ContentPolicyRefusal>()
                            .is_some() =>
                    {
                        let refusal = err
                            .downcast_ref::<crate::ai::ContentPolicyRefusal>()
                            .expect("matched above");
                        let review = self
                            .repository
                            .create_import_review_item(CreateImportReviewItemParams {
                                session_id: input.session_id.clone(),
                                pass_name: import_pass_name(&ImportPassName::EntityExtraction),
                                item_kind: import_review_item_kind_name(
                                    &ImportReviewItemKind::Entity,
                                ),
                                severity: import_review_severity_name(
                                    &ImportReviewSeverity::Warning,
                                ),
                                status: import_review_status_name(&ImportReviewStatus::Open),
                                title: "Model refused entity extraction (content policy)"
                                    .to_string(),
                                description: format!(
                                    "The import model ({}) refused this segment on content-policy grounds; local extraction still ran. Route import to an explicit-cleared agent (see `route_import_to_explicit`) to avoid this. Detail: {}",
                                    refusal.endpoint, refusal.detail
                                ),
                                related_segment_ids: vec![segment_record.id.clone()],
                                related_entity_ids: Vec::new(),
                                confidence: None,
                                proposed_correction: None,
                                resolver_notes: None,
                            })
                            .await?;
                        review_items.push(review);
                    }
                    Err(err) => return Err(err),
                }

                let candidates = extract_entity_candidates(&segment_text);
                for candidate in candidates {
                    let mention = self
                        .repository
                        .create_import_entity_mention(CreateImportEntityMentionParams {
                            session_id: input.session_id.clone(),
                            segment_id: segment_record.id.clone(),
                            entity_kind: import_entity_kind_name(&candidate.entity_kind),
                            surface_form: candidate.surface_form.clone(),
                            normalized_name: candidate.normalized_name.clone(),
                            alias_hint: candidate.alias_hint.clone(),
                            surrounding_text: candidate.surrounding_text.clone(),
                            confidence: candidate.confidence,
                            extraction_pass: format!("extract-{}", extraction_round),
                        })
                        .await?;
                    if let Some(reason) = candidate.review_reason.as_ref() {
                        let review = self
                            .repository
                            .create_import_review_item(CreateImportReviewItemParams {
                                session_id: input.session_id.clone(),
                                pass_name: import_pass_name(&ImportPassName::EntityExtraction),
                                item_kind: import_review_item_kind_name(
                                    &ImportReviewItemKind::Entity,
                                ),
                                severity: import_review_severity_name(
                                    &ImportReviewSeverity::RequiresReview,
                                ),
                                status: import_review_status_name(&ImportReviewStatus::Open),
                                title: format!(
                                    "Review extracted entity '{}'",
                                    candidate.surface_form
                                ),
                                description: reason.clone(),
                                related_segment_ids: vec![segment_record.id.clone()],
                                related_entity_ids: vec![mention.id.clone()],
                                confidence: Some(candidate.confidence),
                                proposed_correction: Some(serde_json::to_value(
                                    ImportCorrectionPayload::Entity {
                                        entity_kind: candidate.entity_kind.clone(),
                                        canonical_name: Some(candidate.surface_form.clone()),
                                        merge_cluster_ids: Vec::new(),
                                        split_aliases: candidate
                                            .alias_hint
                                            .clone()
                                            .into_iter()
                                            .collect(),
                                    },
                                )?),
                                resolver_notes: None,
                            })
                            .await?;
                        review_items.push(review);
                    }
                    mentions.push(mention);
                }

                completed_segment_ids.insert(scene_segment_id);
                processed_segments += 1;
                extraction_round += 1;
            }
        }

        let completed_segment_ids_vec = completed_segment_ids.into_iter().collect::<Vec<_>>();
        let total_segments = segments
            .iter()
            .filter(|segment| segment.segment_type == "scene")
            .count();
        let progress = import_progress(
            source_documents.len(),
            source_documents.len(),
            total_segments,
            completed_segment_ids_vec.len(),
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        );
        let session_status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::EntityExtraction),
                serde_json::to_value(&progress)?,
                &import_session_status_name(&session_status),
            )
            .await?;

        let report = entity_extraction_report_from_records(&mentions, &review_items);
        Ok(ImportExtractEntitiesOutput {
            session_id: input.session_id,
            report,
            review_items: review_items
                .into_iter()
                .filter(|item| {
                    item.pass_name == import_pass_name(&ImportPassName::EntityExtraction)
                })
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_consolidate_entities(
        &self,
        input: spindle_core::models::ImportConsolidateEntitiesInput,
    ) -> Result<spindle_core::models::ImportConsolidateEntitiesOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::consolidate::consolidate_mentions;
        use crate::sqlite::import::prompts::build_entity_consolidation_prompt;
        use crate::sqlite::import_service::{
            entity_consolidation_report_from_records, import_entity_kind_name, import_pass_name,
            import_progress, import_review_item_kind_name, import_review_item_summary,
            import_review_severity_name, import_review_status_name, import_session_status_name,
            parse_import_entity_kind,
        };
        use crate::sqlite::repository::{
            CreateImportReviewItemParams, UpsertImportEntityClusterParams,
        };
        use spindle_core::models::{
            ImportConsolidateEntitiesOutput, ImportCorrectionPayload, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };
        use std::collections::BTreeSet;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let mut review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let entity_kind_filter = (!input.entity_kinds.is_empty()).then(|| {
            input
                .entity_kinds
                .iter()
                .map(import_entity_kind_name)
                .collect::<BTreeSet<_>>()
        });
        let clusters = consolidate_mentions(&mentions, entity_kind_filter.as_ref());

        for cluster in &clusters {
            let candidates = mentions
                .iter()
                .filter(|mention| {
                    mention.entity_kind == cluster.entity_kind
                        && cluster.mention_ids.contains(&mention.id)
                })
                .map(|mention| mention.surface_form.clone())
                .collect::<Vec<_>>();
            let _model_response = self
                .repository
                .model_router()
                .complete(&ModelRequest {
                    route: "import_synthesize".to_string(),
                    prompt: build_entity_consolidation_prompt(
                        parse_import_entity_kind(&cluster.entity_kind),
                        &candidates,
                    ),
                    rating: None,
                    context: None,
                })
                .await?;

            let persisted_cluster = self
                .repository
                .upsert_import_entity_cluster(UpsertImportEntityClusterParams {
                    session_id: input.session_id.clone(),
                    entity_kind: cluster.entity_kind.clone(),
                    canonical_name: cluster.canonical_name.clone(),
                    normalized_name: cluster.normalized_name.clone(),
                    aliases: cluster.aliases.clone(),
                    mention_ids: cluster.mention_ids.clone(),
                    first_segment_id: cluster.first_segment_id.clone(),
                    last_segment_id: cluster.last_segment_id.clone(),
                    importance_rank: cluster.importance_rank,
                    merge_confidence: cluster.merge_confidence,
                    review_required: cluster.review_required,
                    notes: cluster.notes.clone(),
                })
                .await?;

            if cluster.review_required {
                let review = self
                    .repository
                    .create_import_review_item(CreateImportReviewItemParams {
                        session_id: input.session_id.clone(),
                        pass_name: import_pass_name(&ImportPassName::EntityConsolidation),
                        item_kind: import_review_item_kind_name(&ImportReviewItemKind::Entity),
                        severity: import_review_severity_name(
                            &ImportReviewSeverity::RequiresReview,
                        ),
                        status: import_review_status_name(&ImportReviewStatus::Open),
                        title: format!("Review entity cluster '{}'", cluster.canonical_name),
                        description: if cluster.notes.is_empty() {
                            "cluster merge confidence was low".to_string()
                        } else {
                            cluster.notes.join(" ")
                        },
                        related_segment_ids: persisted_cluster
                            .first_segment_id
                            .clone()
                            .into_iter()
                            .collect(),
                        related_entity_ids: vec![persisted_cluster.id.clone()],
                        confidence: Some(cluster.merge_confidence),
                        proposed_correction: Some(serde_json::to_value(
                            ImportCorrectionPayload::Entity {
                                entity_kind: parse_import_entity_kind(&cluster.entity_kind),
                                canonical_name: Some(cluster.canonical_name.clone()),
                                merge_cluster_ids: vec![persisted_cluster.id.clone()],
                                split_aliases: cluster.aliases.clone(),
                            },
                        )?),
                        resolver_notes: None,
                    })
                    .await?;
                review_items.push(review);
            }
        }

        let persisted_clusters = self
            .repository
            .list_import_entity_clusters(&input.session_id)
            .await?;
        let progress = serde_json::to_value(import_progress(
            0,
            0,
            0,
            0,
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        ))?;
        let status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::EntityConsolidation),
                progress,
                &import_session_status_name(&status),
            )
            .await?;

        let report = entity_consolidation_report_from_records(&persisted_clusters, &review_items);
        Ok(ImportConsolidateEntitiesOutput {
            session_id: input.session_id,
            report,
            review_items: review_items
                .into_iter()
                .filter(|item| {
                    item.pass_name == import_pass_name(&ImportPassName::EntityConsolidation)
                })
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_analyze_character(
        &self,
        input: spindle_core::models::ImportAnalyzeCharacterInput,
    ) -> Result<spindle_core::models::ImportAnalyzeCharacterOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::character::{
            build_character_dossiers, build_character_dossiers_for_clusters,
        };
        use crate::sqlite::import::prompts::build_character_analysis_prompt;
        use crate::sqlite::import_service::{
            character_dossier_summaries_from_records, import_pass_name, import_progress,
            import_review_item_kind_name, import_review_item_summary, import_review_severity_name,
            import_review_status_name, import_session_status_name,
        };
        use crate::sqlite::repository::{
            CreateImportReviewItemParams, UpsertImportCharacterDossierParams,
        };
        use spindle_core::models::{
            ImportAnalyzeCharacterOutput, ImportCorrectionPayload, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };
        use std::collections::BTreeSet;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let all_clusters = self
            .repository
            .list_import_entity_clusters(&input.session_id)
            .await?;
        let selected_clusters = if input.cluster_ids.is_empty() {
            build_character_dossiers(&all_clusters, &mentions, &segments)
        } else {
            let cluster_filter = input.cluster_ids.iter().cloned().collect::<BTreeSet<_>>();
            let filtered_clusters = all_clusters
                .iter()
                .filter(|cluster| cluster_filter.contains(&cluster.id))
                .cloned()
                .collect::<Vec<_>>();
            build_character_dossiers_for_clusters(&filtered_clusters, &mentions, &segments)
        };

        let mut review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let character_names = selected_clusters
            .iter()
            .map(|dossier| dossier.canonical_name.clone())
            .collect::<Vec<_>>();
        let character_notes = selected_clusters
            .iter()
            .flat_map(|dossier| dossier.review_notes.clone())
            .collect::<Vec<_>>();
        let _model_response = self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "import_synthesize".to_string(),
                prompt: build_character_analysis_prompt(&character_names, &character_notes),
                rating: None,
                context: None,
            })
            .await?;

        for dossier in &selected_clusters {
            let persisted = self
                .repository
                .upsert_import_character_dossier(UpsertImportCharacterDossierParams {
                    session_id: input.session_id.clone(),
                    cluster_id: dossier.cluster_id.clone(),
                    canonical_name: dossier.canonical_name.clone(),
                    aliases: dossier.aliases.clone(),
                    importance_rank: dossier.importance_rank,
                    voice_profile: serde_json::to_value(&dossier.voice_profile)?,
                    emotional_profile: serde_json::to_value(&dossier.emotional_profile)?,
                    state_trajectory: serde_json::to_value(&dossier.state_trajectory)?,
                    relationship_inferences: serde_json::to_value(
                        &dossier.relationship_inferences,
                    )?,
                    decision_patterns: dossier.decision_patterns.clone(),
                    dialogue_samples: dossier.dialogue_samples.clone(),
                    confidence: dossier.confidence,
                    review_required: dossier.review_required,
                })
                .await?;

            if dossier.review_required {
                let related_segment_ids = dossier
                    .state_trajectory
                    .iter()
                    .map(|point| point.segment_id.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let review = self
                    .repository
                    .create_import_review_item(CreateImportReviewItemParams {
                        session_id: input.session_id.clone(),
                        pass_name: import_pass_name(&ImportPassName::CharacterAnalysis),
                        item_kind: import_review_item_kind_name(&ImportReviewItemKind::Character),
                        severity: import_review_severity_name(
                            &ImportReviewSeverity::RequiresReview,
                        ),
                        status: import_review_status_name(&ImportReviewStatus::Open),
                        title: format!("Review character dossier '{}'", dossier.canonical_name),
                        description: dossier.review_notes.join(" "),
                        related_segment_ids,
                        related_entity_ids: vec![persisted.id.clone(), dossier.cluster_id.clone()],
                        confidence: Some(dossier.confidence),
                        proposed_correction: Some(serde_json::to_value(
                            ImportCorrectionPayload::Character {
                                cluster_id: dossier.cluster_id.clone(),
                                preferred_name: Some(dossier.canonical_name.clone()),
                                relationship_notes: dossier.review_notes.clone(),
                            },
                        )?),
                        resolver_notes: None,
                    })
                    .await?;
                review_items.push(review);
            }
        }

        let persisted_dossiers = self
            .repository
            .list_import_character_dossiers(&input.session_id)
            .await?;
        let progress = serde_json::to_value(import_progress(
            0,
            0,
            0,
            selected_clusters.len(),
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        ))?;
        let status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::CharacterAnalysis),
                progress,
                &import_session_status_name(&status),
            )
            .await?;

        Ok(ImportAnalyzeCharacterOutput {
            session_id: input.session_id,
            characters: character_dossier_summaries_from_records(&persisted_dossiers)?,
            review_items: review_items
                .into_iter()
                .filter(|item| {
                    item.pass_name == import_pass_name(&ImportPassName::CharacterAnalysis)
                })
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_extract_world(
        &self,
        input: spindle_core::models::ImportExtractWorldInput,
    ) -> Result<spindle_core::models::ImportExtractWorldOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::prompts::{
            ImportSynthesizePrompt, build_world_extraction_prompt,
        };
        use crate::sqlite::import::world::{extract_world_dossier, to_world_summary};
        use crate::sqlite::import_service::{
            import_pass_name, import_progress, import_review_item_kind_name,
            import_review_item_summary, import_review_severity_name, import_review_status_name,
            import_session_status_name, load_segment_text, structural_summary_from_records,
        };
        use crate::sqlite::repository::{
            CreateImportReviewItemParams, UpsertImportWorldDossierParams,
        };
        use spindle_core::models::{
            ImportCorrectionPayload, ImportExtractWorldOutput, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };
        use std::collections::BTreeMap;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let project = self.repository.get_project(&input.project_id).await?;
        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let clusters = self
            .repository
            .list_import_entity_clusters(&input.session_id)
            .await?;
        let structure = structural_summary_from_records(&source_documents, &segments, 0)?;
        let mut segment_texts = BTreeMap::new();
        for segment in segments
            .iter()
            .filter(|segment| segment.segment_type == "scene")
        {
            let Some(source_document) = source_documents
                .iter()
                .find(|document| document.id == segment.source_document_id)
            else {
                continue;
            };
            let text = load_segment_text(source_document, segment)?;
            segment_texts.insert(segment.id.clone(), text);
        }

        let focus_notes = vec![
            "extract world rules, institutions, terms, and system signals".to_string(),
            "preserve low-confidence inferences as reviewable outputs".to_string(),
        ];
        let _model_response = self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "import_synthesize".to_string(),
                prompt: build_world_extraction_prompt(&ImportSynthesizePrompt {
                    structure: &structure,
                    focus: &project.name,
                    notes: &focus_notes,
                }),
                rating: None,
                context: None,
            })
            .await?;

        let draft = extract_world_dossier(&segments, &segment_texts, &mentions, &clusters);
        let persisted = self
            .repository
            .upsert_import_world_dossier(UpsertImportWorldDossierParams {
                session_id: input.session_id.clone(),
                world_rules: serde_json::to_value(&draft.world_rules)?,
                locations: serde_json::to_value(&draft.locations)?,
                entities: serde_json::to_value(&draft.entities)?,
                system_signals: serde_json::to_value(&draft.system_signals)?,
            })
            .await?;

        let mut review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        for review in &draft.review_items {
            let created = self
                .repository
                .create_import_review_item(CreateImportReviewItemParams {
                    session_id: input.session_id.clone(),
                    pass_name: import_pass_name(&ImportPassName::WorldExtraction),
                    item_kind: import_review_item_kind_name(&ImportReviewItemKind::World),
                    severity: import_review_severity_name(&ImportReviewSeverity::RequiresReview),
                    status: import_review_status_name(&ImportReviewStatus::Open),
                    title: review.title.clone(),
                    description: review.description.clone(),
                    related_segment_ids: review.source_segment_ids.clone(),
                    related_entity_ids: vec![persisted.id.clone()],
                    confidence: Some(review.confidence),
                    proposed_correction: Some(serde_json::to_value(
                        ImportCorrectionPayload::World {
                            entity_kind: review.entity_kind.clone(),
                            canonical_name: review.canonical_name.clone(),
                            replacement_summary: review.replacement_summary.clone(),
                        },
                    )?),
                    resolver_notes: None,
                })
                .await?;
            review_items.push(created);
        }

        let progress = serde_json::to_value(import_progress(
            structure.source_documents.len(),
            structure.source_documents.len(),
            structure
                .chapters
                .iter()
                .map(|chapter| chapter.scenes.len())
                .sum(),
            draft.world_rules.len()
                + draft.locations.len()
                + draft.entities.len()
                + draft.system_signals.len(),
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        ))?;
        let status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::WorldExtraction),
                progress,
                &import_session_status_name(&status),
            )
            .await?;

        Ok(ImportExtractWorldOutput {
            session_id: input.session_id,
            world: to_world_summary(&draft),
            review_items: review_items
                .into_iter()
                .filter(|item| item.pass_name == import_pass_name(&ImportPassName::WorldExtraction))
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_analyze_narrative(
        &self,
        input: spindle_core::models::ImportAnalyzeNarrativeInput,
    ) -> Result<spindle_core::models::ImportAnalyzeNarrativeOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::narrative::{analyze_narrative_dossier, to_narrative_summary};
        use crate::sqlite::import::prompts::{
            ImportSynthesizePrompt, build_narrative_analysis_prompt,
        };
        use crate::sqlite::import_service::{
            character_dossier_summaries_from_records, import_pass_name, import_progress,
            import_review_item_kind_name, import_review_item_summary, import_review_severity_name,
            import_review_status_name, import_session_status_name, load_segment_text,
            structural_summary_from_records,
        };
        use crate::sqlite::repository::{
            CreateImportReviewItemParams, UpsertImportNarrativeDossierParams,
        };
        use spindle_core::models::{
            ImportAnalyzeNarrativeOutput, ImportCorrectionPayload, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };
        use std::collections::BTreeMap;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let character_dossiers = self
            .repository
            .list_import_character_dossiers(&input.session_id)
            .await?;
        let structure = structural_summary_from_records(&source_documents, &segments, 0)?;
        let mut segment_texts = BTreeMap::new();
        for segment in segments
            .iter()
            .filter(|segment| segment.segment_type == "scene")
        {
            let Some(source_document) = source_documents
                .iter()
                .find(|document| document.id == segment.source_document_id)
            else {
                continue;
            };
            let text = load_segment_text(source_document, segment)?;
            segment_texts.insert(segment.id.clone(), text);
        }

        let review_items_before = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let prompt_notes = vec![
            "extract plot lines, conflicts, promises, arcs, themes, motifs, and pacing hints"
                .to_string(),
            "preserve low-confidence narrative reads as review items".to_string(),
        ];
        let _model_response = self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "import_synthesize".to_string(),
                prompt: build_narrative_analysis_prompt(&ImportSynthesizePrompt {
                    structure: &structure,
                    focus: "narrative architecture",
                    notes: &prompt_notes,
                }),
                rating: None,
                context: None,
            })
            .await?;

        let draft = analyze_narrative_dossier(
            &segments,
            &segment_texts,
            &mentions,
            &character_dossier_summaries_from_records(&character_dossiers)?,
        );
        let persisted = self
            .repository
            .upsert_import_narrative_dossier(UpsertImportNarrativeDossierParams {
                session_id: input.session_id.clone(),
                plot_lines: serde_json::to_value(&draft.plot_lines)?,
                conflicts: serde_json::to_value(&draft.conflicts)?,
                narrative_promises: serde_json::to_value(&draft.narrative_promises)?,
                arcs: serde_json::to_value(&draft.arcs)?,
                themes: serde_json::to_value(&draft.themes)?,
                motifs: serde_json::to_value(&draft.motifs)?,
                reader_contract: serde_json::to_value(&draft.reader_contract)?,
                pacing_hints: serde_json::to_value(&draft.pacing_hints)?,
            })
            .await?;

        let mut review_items = review_items_before;
        for review in &draft.review_items {
            let created = self
                .repository
                .create_import_review_item(CreateImportReviewItemParams {
                    session_id: input.session_id.clone(),
                    pass_name: import_pass_name(&ImportPassName::NarrativeAnalysis),
                    item_kind: import_review_item_kind_name(&ImportReviewItemKind::Narrative),
                    severity: import_review_severity_name(&ImportReviewSeverity::RequiresReview),
                    status: import_review_status_name(&ImportReviewStatus::Open),
                    title: review.title.clone(),
                    description: review.description.clone(),
                    related_segment_ids: review.related_segment_ids.clone(),
                    related_entity_ids: vec![persisted.id.clone()],
                    confidence: Some(review.confidence),
                    proposed_correction: Some(serde_json::to_value(
                        ImportCorrectionPayload::Narrative {
                            target_id: review.target_id.clone(),
                            status: review.status.clone(),
                            thematic_purpose: review.thematic_purpose.clone(),
                            note: review.note.clone(),
                        },
                    )?),
                    resolver_notes: None,
                })
                .await?;
            review_items.push(created);
        }

        let progress = serde_json::to_value(import_progress(
            structure.source_documents.len(),
            structure.source_documents.len(),
            structure
                .chapters
                .iter()
                .map(|chapter| chapter.scenes.len())
                .sum(),
            draft.plot_lines.len()
                + draft.conflicts.len()
                + draft.narrative_promises.len()
                + draft.arcs.len()
                + draft.themes.len()
                + draft.motifs.len(),
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        ))?;
        let status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::NarrativeAnalysis),
                progress,
                &import_session_status_name(&status),
            )
            .await?;

        Ok(ImportAnalyzeNarrativeOutput {
            session_id: input.session_id,
            narrative: to_narrative_summary(&draft),
            review_items: review_items
                .into_iter()
                .filter(|item| {
                    item.pass_name == import_pass_name(&ImportPassName::NarrativeAnalysis)
                })
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_compute_final_state(
        &self,
        input: spindle_core::models::ImportComputeFinalStateInput,
    ) -> Result<spindle_core::models::ImportComputeFinalStateOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::final_state::compute_final_state;
        use crate::sqlite::import::prompts::{ImportSynthesizePrompt, build_final_state_prompt};
        use crate::sqlite::import_service::{
            character_dossier_summaries_from_records, import_pass_name, import_progress,
            import_review_item_kind_name, import_review_item_summary, import_review_severity_name,
            import_review_status_name, import_session_status_name,
            narrative_dossier_summary_from_record, resume_snapshot_summary_from_record,
            structural_summary_from_records, world_dossier_summary_from_record,
        };
        use crate::sqlite::repository::{
            CreateImportReviewItemParams, UpsertImportResumeSnapshotParams,
        };
        use spindle_core::models::{
            ImportComputeFinalStateOutput, ImportCorrectionPayload, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let character_dossiers = self
            .repository
            .list_import_character_dossiers(&input.session_id)
            .await?;
        let world = self
            .repository
            .find_import_world_dossier(&input.session_id)
            .await?
            .map(world_dossier_summary_from_record)
            .transpose()?;
        let narrative = self
            .repository
            .find_import_narrative_dossier(&input.session_id)
            .await?
            .map(narrative_dossier_summary_from_record)
            .transpose()?;
        let structure = structural_summary_from_records(&source_documents, &segments, 0)?;
        let prompt_notes = vec![
            "compute the explicit manuscript resume point".to_string(),
            "defer canonical knowledge facts until hydration".to_string(),
        ];
        let _model_response = self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "import_synthesize".to_string(),
                prompt: build_final_state_prompt(&ImportSynthesizePrompt {
                    structure: &structure,
                    focus: "resume state and continuation handoff",
                    notes: &prompt_notes,
                }),
                rating: None,
                context: None,
            })
            .await?;

        let draft = compute_final_state(
            &segments,
            &character_dossier_summaries_from_records(&character_dossiers)?,
            world.as_ref(),
            narrative.as_ref(),
        );
        let persisted = self
            .repository
            .upsert_import_resume_snapshot(UpsertImportResumeSnapshotParams {
                session_id: input.session_id.clone(),
                book_number: draft.resume_snapshot.book_number,
                chapter_number: draft.resume_snapshot.chapter_number,
                scene_order: draft.resume_snapshot.scene_order,
                summary: draft.resume_snapshot.summary.clone(),
                characters: serde_json::to_value(&draft.resume_snapshot.characters)?,
                relationships: serde_json::to_value(&draft.resume_snapshot.relationships)?,
                locations: serde_json::to_value(&draft.resume_snapshot.locations)?,
                plot_threads: serde_json::to_value(&draft.resume_snapshot.plot_threads)?,
            })
            .await?;
        let _ = resume_snapshot_summary_from_record(persisted.clone())?;

        let mut review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        for review in &draft.review_items {
            let created = self
                .repository
                .create_import_review_item(CreateImportReviewItemParams {
                    session_id: input.session_id.clone(),
                    pass_name: import_pass_name(&ImportPassName::FinalState),
                    item_kind: import_review_item_kind_name(&ImportReviewItemKind::FinalState),
                    severity: import_review_severity_name(&ImportReviewSeverity::RequiresReview),
                    status: import_review_status_name(&ImportReviewStatus::Open),
                    title: review.title.clone(),
                    description: review.description.clone(),
                    related_segment_ids: review.related_segment_ids.clone(),
                    related_entity_ids: vec![persisted.id.clone()],
                    confidence: Some(review.confidence),
                    proposed_correction: Some(serde_json::to_value(
                        ImportCorrectionPayload::FinalState {
                            target_record_id: review.target_record_id.clone(),
                            corrected_summary: review.corrected_summary.clone(),
                        },
                    )?),
                    resolver_notes: None,
                })
                .await?;
            review_items.push(created);
        }

        let progress = serde_json::to_value(import_progress(
            structure.source_documents.len(),
            structure.source_documents.len(),
            structure
                .chapters
                .iter()
                .map(|chapter| chapter.scenes.len())
                .sum(),
            draft.resume_snapshot.characters.len()
                + draft.resume_snapshot.relationships.len()
                + draft.resume_snapshot.locations.len()
                + draft.resume_snapshot.plot_threads.len(),
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        ))?;
        let status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::FinalState),
                progress,
                &import_session_status_name(&status),
            )
            .await?;

        Ok(ImportComputeFinalStateOutput {
            session_id: input.session_id,
            resume_snapshot: draft.resume_snapshot,
            review_items: review_items
                .into_iter()
                .filter(|item| item.pass_name == import_pass_name(&ImportPassName::FinalState))
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_hydrate_bible(
        &self,
        input: spindle_core::models::ImportHydrateBibleInput,
    ) -> Result<spindle_core::models::ImportHydrateBibleOutput> {
        self.import_hydrate_bible_inner(input).await
    }

    pub async fn import_apply_review_decisions(
        &self,
        input: spindle_core::models::ImportApplyReviewDecisionsInput,
    ) -> Result<spindle_core::models::ImportApplyReviewDecisionsOutput> {
        use crate::sqlite::import_service::{
            import_progress, import_review_item_summary, import_review_status_name,
            import_session_status_name,
        };
        use crate::sqlite::repository::ResolveImportReviewItemParams;
        use spindle_core::models::{
            ImportApplyReviewDecisionsOutput, ImportReviewStatus, ImportSessionProgress,
            ImportSessionStatus,
        };
        use std::collections::BTreeMap;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let review_items_by_id = review_items
            .iter()
            .cloned()
            .map(|item| (item.id.clone(), item))
            .collect::<BTreeMap<_, _>>();

        let mut updated_review_items = Vec::new();
        for decision in input.decisions {
            let review_item = review_items_by_id
                .get(&decision.review_item_id)
                .with_context(|| format!("review item not found: {}", decision.review_item_id))?;
            if review_item.session_id != input.session_id {
                anyhow::bail!("review item does not belong to the requested import session");
            }
            let updated = self
                .repository
                .resolve_import_review_item(
                    &review_item.id,
                    ResolveImportReviewItemParams {
                        status: import_review_status_name(&decision.resolution),
                        proposed_correction: decision
                            .correction
                            .as_ref()
                            .map(serde_json::to_value)
                            .transpose()?
                            .or_else(|| review_item.proposed_correction.clone()),
                        resolver_notes: decision.resolver_notes.clone(),
                    },
                )
                .await?;
            updated_review_items.push(import_review_item_summary(updated)?);
        }

        let remaining_open_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?
            .into_iter()
            .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
            .count();
        let next_status = if remaining_open_items == 0 {
            ImportSessionStatus::ReadyToHydrate
        } else {
            ImportSessionStatus::ReviewNeeded
        };
        let existing_progress =
            serde_json::from_value::<ImportSessionProgress>(session.progress.clone())
                .unwrap_or_else(|_| import_progress(0, 0, 0, 0, 0, 0));
        let updated_progress = import_progress(
            existing_progress.total_documents,
            existing_progress.processed_documents,
            existing_progress.total_segments,
            existing_progress.processed_segments,
            existing_progress.total_review_items,
            remaining_open_items,
        );
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &session.active_pass,
                serde_json::to_value(updated_progress)?,
                &import_session_status_name(&next_status),
            )
            .await?;

        Ok(ImportApplyReviewDecisionsOutput {
            session_id: input.session_id,
            updated_review_items,
            remaining_open_items,
        })
    }
}
