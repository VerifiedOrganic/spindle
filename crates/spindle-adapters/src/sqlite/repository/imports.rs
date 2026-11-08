use super::*;

impl Repository {
    // =========================================================================
    // Import pipeline
    // =========================================================================

    pub async fn create_import_session(
        &self,
        params: CreateImportSessionParams,
    ) -> Result<ImportSession> {
        let id = mint_id("import_session");
        let id_out = id.clone();
        let progress_json = serde_json::to_string(&params.progress)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        let CreateImportSessionParams {
            project_id,
            target_branch_id,
            source_format,
            active_pass,
            session_status,
            hydrate_mode,
            source_count,
            ..
        } = params;

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO import_session (id, project_id, target_branch_id, source_format, \
                     active_pass, progress, session_status, hydrate_mode, source_count, \
                     hydration_report, imported_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?10)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &target_branch_id,
                        &source_format,
                        &active_pass,
                        &progress_json,
                        &session_status,
                        &hydrate_mode,
                        source_count as i64,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_session(&id_out).await
    }

    pub async fn get_import_session(&self, id: &str) -> Result<ImportSession> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {IMPORT_SESSION_COLUMNS} FROM import_session WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportSession::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_session not found"))
    }

    pub async fn list_import_sessions_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ImportSession>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_SESSION_COLUMNS} FROM import_session \
                     WHERE project_id = ?1 ORDER BY imported_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ImportSession::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn update_import_session_state(
        &self,
        session_id: &str,
        active_pass: &str,
        progress: Value,
        session_status: &str,
    ) -> Result<ImportSession> {
        let session_id_owned = session_id.to_string();
        let active_pass = active_pass.to_string();
        let progress_json = serde_json::to_string(&progress)?;
        let session_status = session_status.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE import_session SET active_pass = ?1, progress = ?2, \
                     session_status = ?3, updated_at = ?4 WHERE id = ?5",
                    rusqlite::params![
                        &active_pass,
                        &progress_json,
                        &session_status,
                        now,
                        &session_id_owned,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_session(session_id).await
    }

    pub async fn update_import_session_hydration_report(
        &self,
        session_id: &str,
        report: Value,
    ) -> Result<ImportSession> {
        let session_id_owned = session_id.to_string();
        let report_json = serde_json::to_string(&report)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE import_session SET hydration_report = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&report_json, now, &session_id_owned],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_session(session_id).await
    }

    pub async fn upsert_import_source_document(
        &self,
        params: UpsertImportSourceDocumentParams,
    ) -> Result<ImportSourceDocument> {
        let id = mint_id("import_source_document");
        let id_out = id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        let UpsertImportSourceDocumentParams {
            session_id,
            project_id,
            display_name,
            source_path,
            copied_path,
            source_format,
            original_sha256,
            normalized_sha256,
            normalized_text_ref,
            word_count,
            chapter_hint,
            source_order,
        } = params;

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_source_document \
                         WHERE session_id = ?1 AND source_order = ?2",
                        rusqlite::params![&session_id, source_order as i64],
                    )?;
                    tx.execute(
                        "INSERT INTO import_source_document (id, session_id, project_id, \
                         display_name, source_path, copied_path, source_format, original_sha256, \
                         normalized_sha256, normalized_text_ref, word_count, chapter_hint, \
                         source_order, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
                        rusqlite::params![
                            &id,
                            &session_id,
                            &project_id,
                            &display_name,
                            &source_path,
                            &copied_path,
                            &source_format,
                            &original_sha256,
                            &normalized_sha256,
                            &normalized_text_ref,
                            word_count as i64,
                            &chapter_hint,
                            source_order as i64,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_source_document(&id_out).await
    }

    pub async fn get_import_source_document(&self, id: &str) -> Result<ImportSourceDocument> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_SOURCE_DOCUMENT_COLUMNS} FROM import_source_document WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportSourceDocument::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_source_document not found"))
    }

    pub async fn list_import_source_documents(
        &self,
        session_id: &str,
    ) -> Result<Vec<ImportSourceDocument>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_SOURCE_DOCUMENT_COLUMNS} FROM import_source_document \
                     WHERE session_id = ?1 ORDER BY source_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportSourceDocument::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn upsert_import_segment(
        &self,
        params: UpsertImportSegmentParams,
    ) -> Result<ImportSegment> {
        let id = mint_id("import_segment");
        let id_out = id.clone();
        let pov_guess_json = match params.pov_guess.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let UpsertImportSegmentParams {
            session_id,
            source_document_id,
            parent_segment_id,
            segment_type,
            source_order,
            book_number,
            chapter_number,
            scene_order,
            label,
            start_offset,
            end_offset,
            word_count,
            character_count,
            confidence,
            segment_status,
            ..
        } = params;

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                let source_document_id = source_document_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_segment \
                         WHERE session_id = ?1 AND source_document_id = ?2 AND source_order = ?3",
                        rusqlite::params![&session_id, &source_document_id, source_order as i64],
                    )?;
                    tx.execute(
                        "INSERT INTO import_segment (id, session_id, source_document_id, \
                         parent_segment_id, segment_type, source_order, book_number, chapter_number, \
                         scene_order, label, start_offset, end_offset, word_count, character_count, \
                         pov_guess, confidence, segment_status, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, \
                                 ?16, ?17, ?18, ?18)",
                        rusqlite::params![
                            &id, &session_id, &source_document_id, &parent_segment_id,
                            &segment_type, source_order as i64,
                            book_number.map(i64::from), chapter_number.map(i64::from),
                            scene_order.map(i64::from), &label, start_offset as i64,
                            end_offset as i64, word_count as i64, character_count as i64,
                            &pov_guess_json, confidence, &segment_status, now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_segment(&id_out).await
    }

    pub async fn get_import_segment(&self, id: &str) -> Result<ImportSegment> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {IMPORT_SEGMENT_COLUMNS} FROM import_segment WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportSegment::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_segment not found"))
    }

    pub async fn list_import_segments(&self, session_id: &str) -> Result<Vec<ImportSegment>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_SEGMENT_COLUMNS} FROM import_segment \
                     WHERE session_id = ?1 ORDER BY source_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportSegment::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn upsert_import_entity_cluster(
        &self,
        params: UpsertImportEntityClusterParams,
    ) -> Result<ImportEntityCluster> {
        let id = mint_id("import_entity_cluster");
        let id_out = id.clone();
        let aliases_json = serde_json::to_string(&params.aliases)?;
        let mention_ids_json = serde_json::to_string(&params.mention_ids)?;
        let notes_json = serde_json::to_string(&params.notes)?;
        let review_required = if params.review_required { 1 } else { 0 };
        let now = timestamp_to_micros(chrono::Utc::now());
        let UpsertImportEntityClusterParams {
            session_id,
            entity_kind,
            canonical_name,
            normalized_name,
            first_segment_id,
            last_segment_id,
            importance_rank,
            merge_confidence,
            ..
        } = params;

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                let entity_kind = entity_kind.clone();
                let normalized_name = normalized_name.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_entity_cluster \
                         WHERE session_id = ?1 AND entity_kind = ?2 AND normalized_name = ?3",
                        rusqlite::params![&session_id, &entity_kind, &normalized_name],
                    )?;
                    tx.execute(
                        "INSERT INTO import_entity_cluster (id, session_id, entity_kind, \
                         canonical_name, normalized_name, aliases, mention_ids, first_segment_id, \
                         last_segment_id, importance_rank, merge_confidence, review_required, \
                         notes, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
                        rusqlite::params![
                            &id,
                            &session_id,
                            &entity_kind,
                            &canonical_name,
                            &normalized_name,
                            &aliases_json,
                            &mention_ids_json,
                            &first_segment_id,
                            &last_segment_id,
                            importance_rank,
                            merge_confidence,
                            review_required,
                            &notes_json,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_entity_cluster(&id_out).await
    }

    pub async fn get_import_entity_cluster(&self, id: &str) -> Result<ImportEntityCluster> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_CLUSTER_COLUMNS} FROM import_entity_cluster WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportEntityCluster::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_entity_cluster not found"))
    }

    pub async fn upsert_import_character_dossier(
        &self,
        params: UpsertImportCharacterDossierParams,
    ) -> Result<ImportCharacterDossier> {
        let id = mint_id("import_character_dossier");
        let id_out = id.clone();
        let aliases_json = serde_json::to_string(&params.aliases)?;
        let voice_json = serde_json::to_string(&params.voice_profile)?;
        let emo_json = serde_json::to_string(&params.emotional_profile)?;
        let state_json = serde_json::to_string(&params.state_trajectory)?;
        let relationship_json = serde_json::to_string(&params.relationship_inferences)?;
        let decision_json = serde_json::to_string(&params.decision_patterns)?;
        let dialogue_json = serde_json::to_string(&params.dialogue_samples)?;
        let review_required = if params.review_required { 1 } else { 0 };
        let now = timestamp_to_micros(chrono::Utc::now());
        let UpsertImportCharacterDossierParams {
            session_id,
            cluster_id,
            canonical_name,
            importance_rank,
            confidence,
            ..
        } = params;

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                let cluster_id = cluster_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_character_dossier WHERE session_id = ?1 AND cluster_id = ?2",
                        rusqlite::params![&session_id, &cluster_id],
                    )?;
                    tx.execute(
                        "INSERT INTO import_character_dossier (id, session_id, cluster_id, \
                         canonical_name, aliases, importance_rank, voice_profile, emotional_profile, \
                         state_trajectory, relationship_inferences, decision_patterns, dialogue_samples, \
                         confidence, review_required, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
                        rusqlite::params![
                            &id, &session_id, &cluster_id, &canonical_name, &aliases_json,
                            importance_rank, &voice_json, &emo_json, &state_json, &relationship_json,
                            &decision_json, &dialogue_json, confidence, review_required, now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_character_dossier(&id_out).await
    }

    pub async fn get_import_character_dossier(&self, id: &str) -> Result<ImportCharacterDossier> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_CHARACTER_DOSSIER_COLUMNS} FROM import_character_dossier WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportCharacterDossier::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_character_dossier not found"))
    }

    pub async fn upsert_import_world_dossier(
        &self,
        params: UpsertImportWorldDossierParams,
    ) -> Result<ImportWorldDossier> {
        let id = mint_id("import_world_dossier");
        let id_out = id.clone();
        let rules_json = serde_json::to_string(&params.world_rules)?;
        let locations_json = serde_json::to_string(&params.locations)?;
        let entities_json = serde_json::to_string(&params.entities)?;
        let signals_json = serde_json::to_string(&params.system_signals)?;
        let session_id = params.session_id;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_world_dossier WHERE session_id = ?1",
                        [&session_id],
                    )?;
                    tx.execute(
                        "INSERT INTO import_world_dossier (id, session_id, world_rules, locations, \
                         entities, system_signals, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        rusqlite::params![
                            &id,
                            &session_id,
                            &rules_json,
                            &locations_json,
                            &entities_json,
                            &signals_json,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_world_dossier(&id_out).await
    }

    pub async fn get_import_world_dossier(&self, id: &str) -> Result<ImportWorldDossier> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_WORLD_DOSSIER_COLUMNS} FROM import_world_dossier WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportWorldDossier::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_world_dossier not found"))
    }

    pub async fn upsert_import_narrative_dossier(
        &self,
        params: UpsertImportNarrativeDossierParams,
    ) -> Result<ImportNarrativeDossier> {
        let id = mint_id("import_narrative_dossier");
        let id_out = id.clone();
        let plot_lines = serde_json::to_string(&params.plot_lines)?;
        let conflicts = serde_json::to_string(&params.conflicts)?;
        let promises = serde_json::to_string(&params.narrative_promises)?;
        let arcs = serde_json::to_string(&params.arcs)?;
        let themes = serde_json::to_string(&params.themes)?;
        let motifs = serde_json::to_string(&params.motifs)?;
        let reader_contract = serde_json::to_string(&params.reader_contract)?;
        let pacing_hints = serde_json::to_string(&params.pacing_hints)?;
        let session_id = params.session_id;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_narrative_dossier WHERE session_id = ?1",
                        [&session_id],
                    )?;
                    tx.execute(
                        "INSERT INTO import_narrative_dossier (id, session_id, plot_lines, \
                         conflicts, narrative_promises, arcs, themes, motifs, reader_contract, \
                         pacing_hints, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                        rusqlite::params![
                            &id,
                            &session_id,
                            &plot_lines,
                            &conflicts,
                            &promises,
                            &arcs,
                            &themes,
                            &motifs,
                            &reader_contract,
                            &pacing_hints,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_narrative_dossier(&id_out).await
    }

    pub async fn get_import_narrative_dossier(&self, id: &str) -> Result<ImportNarrativeDossier> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_NARRATIVE_DOSSIER_COLUMNS} FROM import_narrative_dossier WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportNarrativeDossier::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_narrative_dossier not found"))
    }

    pub async fn upsert_import_resume_snapshot(
        &self,
        params: UpsertImportResumeSnapshotParams,
    ) -> Result<ImportResumeSnapshot> {
        let id = mint_id("import_resume_snapshot");
        let id_out = id.clone();
        let characters = serde_json::to_string(&params.characters)?;
        let relationships = serde_json::to_string(&params.relationships)?;
        let locations = serde_json::to_string(&params.locations)?;
        let plot_threads = serde_json::to_string(&params.plot_threads)?;
        let session_id = params.session_id;
        let book_number = params.book_number;
        let chapter_number = params.chapter_number;
        let scene_order = params.scene_order;
        let summary = params.summary;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_resume_snapshot WHERE session_id = ?1",
                        [&session_id],
                    )?;
                    tx.execute(
                        "INSERT INTO import_resume_snapshot (id, session_id, book_number, \
                         chapter_number, scene_order, summary, characters, relationships, \
                         locations, plot_threads, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                        rusqlite::params![
                            &id,
                            &session_id,
                            book_number,
                            chapter_number,
                            scene_order.map(i64::from),
                            &summary,
                            &characters,
                            &relationships,
                            &locations,
                            &plot_threads,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_resume_snapshot(&id_out).await
    }

    pub async fn get_import_resume_snapshot(&self, id: &str) -> Result<ImportResumeSnapshot> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_RESUME_SNAPSHOT_COLUMNS} FROM import_resume_snapshot WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportResumeSnapshot::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_resume_snapshot not found"))
    }

    pub async fn create_import_review_item(
        &self,
        params: CreateImportReviewItemParams,
    ) -> Result<ImportReviewItem> {
        let id = mint_id("import_review_item");
        let id_out = id.clone();
        let related_segments = serde_json::to_string(&params.related_segment_ids)?;
        let related_entities = serde_json::to_string(&params.related_entity_ids)?;
        let proposed_correction = match params.proposed_correction.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let CreateImportReviewItemParams {
            session_id,
            pass_name,
            item_kind,
            severity,
            status,
            title,
            description,
            confidence,
            resolver_notes,
            ..
        } = params;

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO import_review_item (id, session_id, pass_name, item_kind, \
                     severity, status, title, description, related_segment_ids, related_entity_ids, \
                     confidence, proposed_correction, resolver_notes, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
                    rusqlite::params![
                        &id, &session_id, &pass_name, &item_kind, &severity, &status, &title,
                        &description, &related_segments, &related_entities, confidence,
                        &proposed_correction, &resolver_notes, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_review_item(&id_out).await
    }

    pub async fn get_import_review_item(&self, id: &str) -> Result<ImportReviewItem> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_REVIEW_ITEM_COLUMNS} FROM import_review_item WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportReviewItem::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_review_item not found"))
    }

    pub async fn list_import_review_items_by_status(
        &self,
        session_id: &str,
        status: &str,
    ) -> Result<Vec<ImportReviewItem>> {
        let session_id = session_id.to_string();
        let status = status.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_REVIEW_ITEM_COLUMNS} FROM import_review_item \
                     WHERE session_id = ?1 AND status = ?2 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&session_id, &status], |r| {
                        ImportReviewItem::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every review item for a session, ordered by `created_at`. The
    /// `_by_status` variant filters; this returns the full set the import
    /// service replays into its rendered summaries.
    pub async fn list_import_review_items(
        &self,
        session_id: &str,
    ) -> Result<Vec<ImportReviewItem>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_REVIEW_ITEM_COLUMNS} FROM import_review_item \
                     WHERE session_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportReviewItem::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Mark a review item resolved by status, optionally updating the
    /// proposed correction and resolver notes. Mirrors the SurrealDB-era
    /// `resolve_import_review_item` helper.
    pub async fn resolve_import_review_item(
        &self,
        id: &str,
        params: ResolveImportReviewItemParams,
    ) -> Result<ImportReviewItem> {
        let id_owned = id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        let proposed_correction = match params.proposed_correction.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let ResolveImportReviewItemParams {
            status,
            resolver_notes,
            ..
        } = params;
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE import_review_item \
                     SET status = ?1, proposed_correction = ?2, resolver_notes = ?3, \
                         updated_at = ?4 \
                     WHERE id = ?5",
                    rusqlite::params![
                        &status,
                        &proposed_correction,
                        &resolver_notes,
                        now,
                        &id_owned,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_review_item(id).await
    }

    /// List every entity mention attached to a session. Used by the
    /// extract/consolidate passes to thread mentions through the cluster
    /// builder.
    pub async fn list_import_entity_mentions(
        &self,
        session_id: &str,
    ) -> Result<Vec<ImportEntityMention>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_MENTION_COLUMNS} FROM import_entity_mention \
                     WHERE session_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportEntityMention::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every entity cluster persisted for a session.
    pub async fn list_import_entity_clusters(
        &self,
        session_id: &str,
    ) -> Result<Vec<ImportEntityCluster>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_CLUSTER_COLUMNS} FROM import_entity_cluster \
                     WHERE session_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportEntityCluster::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every persisted character dossier for a session.
    pub async fn list_import_character_dossiers(
        &self,
        session_id: &str,
    ) -> Result<Vec<ImportCharacterDossier>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_CHARACTER_DOSSIER_COLUMNS} FROM import_character_dossier \
                     WHERE session_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportCharacterDossier::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Return the single world dossier for a session, if any.
    pub async fn find_import_world_dossier(
        &self,
        session_id: &str,
    ) -> Result<Option<ImportWorldDossier>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_WORLD_DOSSIER_COLUMNS} FROM import_world_dossier \
                     WHERE session_id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&session_id], |r| ImportWorldDossier::try_from(r))
                    .optional_inner()
            })
            .await
    }

    /// Return the single narrative dossier for a session, if any.
    pub async fn find_import_narrative_dossier(
        &self,
        session_id: &str,
    ) -> Result<Option<ImportNarrativeDossier>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_NARRATIVE_DOSSIER_COLUMNS} FROM import_narrative_dossier \
                     WHERE session_id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&session_id], |r| ImportNarrativeDossier::try_from(r))
                    .optional_inner()
            })
            .await
    }

    /// Return the single resume snapshot for a session, if any.
    pub async fn find_import_resume_snapshot(
        &self,
        session_id: &str,
    ) -> Result<Option<ImportResumeSnapshot>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_RESUME_SNAPSHOT_COLUMNS} FROM import_resume_snapshot \
                     WHERE session_id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&session_id], |r| ImportResumeSnapshot::try_from(r))
                    .optional_inner()
            })
            .await
    }

    /// Return true when any existing import session has already ingested a
    /// source document with the given `original_sha256`. Used to honor the
    /// `Reject` duplicate-strategy on `import_manuscript`.
    pub async fn import_session_exists_for_source_hash(&self, hash: &str) -> Result<bool> {
        let hash = hash.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT 1 FROM import_source_document WHERE original_sha256 = ?1 LIMIT 1",
                )?;
                stmt.query_row([&hash], |_| Ok(())).optional_inner()
            })
            .await
            .map(|opt| opt.is_some())
    }

    /// Cheap idempotency check: was hydration for this session already
    /// completed against the requested project/branch? Returns `true` once
    /// `update_import_session_hydration_report` has stored a report that
    /// matches both ids.
    pub async fn hydration_target_exists(
        &self,
        session_id: &str,
        project_id: &str,
        branch_id: &str,
    ) -> Result<bool> {
        let session = self.get_import_session(session_id).await?;
        let Some(report) = session.hydration_report.as_ref() else {
            return Ok(false);
        };
        let report_project = report
            .get("project_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let report_branch = report
            .get("branch_id")
            .and_then(|v| v.as_str())
            .or_else(|| report.get("target_branch_id").and_then(|v| v.as_str()))
            .unwrap_or_default();
        Ok(report_project == project_id && report_branch == branch_id)
    }

    pub async fn create_import_entity_mention(
        &self,
        params: CreateImportEntityMentionParams,
    ) -> Result<ImportEntityMention> {
        let id = mint_id("import_entity_mention");
        let id_out = id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        let CreateImportEntityMentionParams {
            session_id,
            segment_id,
            entity_kind,
            surface_form,
            normalized_name,
            alias_hint,
            surrounding_text,
            confidence,
            extraction_pass,
        } = params;

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO import_entity_mention (id, session_id, segment_id, entity_kind, \
                     surface_form, normalized_name, alias_hint, surrounding_text, confidence, \
                     extraction_pass, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        &id,
                        &session_id,
                        &segment_id,
                        &entity_kind,
                        &surface_form,
                        &normalized_name,
                        &alias_hint,
                        &surrounding_text,
                        confidence,
                        &extraction_pass,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_entity_mention(&id_out).await
    }

    pub async fn get_import_entity_mention(&self, id: &str) -> Result<ImportEntityMention> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_MENTION_COLUMNS} FROM import_entity_mention WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportEntityMention::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_entity_mention not found"))
    }

    pub async fn list_import_sessions_by_target_branch_ids(
        &self,
        branch_ids: &[String],
    ) -> Result<Vec<ImportSession>> {
        if branch_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_ids: Vec<String> = branch_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..branch_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_SESSION_COLUMNS} FROM import_session \
                     WHERE target_branch_id IN ({placeholders}) \
                     ORDER BY imported_at DESC"
                );
                let params: Vec<&dyn rusqlite::ToSql> = branch_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportSession::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_source_documents_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportSourceDocument>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_SOURCE_DOCUMENT_COLUMNS} FROM import_source_document \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, source_order"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportSourceDocument::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_segments_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportSegment>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_SEGMENT_COLUMNS} FROM import_segment \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, source_order"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportSegment::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_entity_mentions_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportEntityMention>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_MENTION_COLUMNS} FROM import_entity_mention \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportEntityMention::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_entity_clusters_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportEntityCluster>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_CLUSTER_COLUMNS} FROM import_entity_cluster \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportEntityCluster::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_character_dossiers_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportCharacterDossier>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_CHARACTER_DOSSIER_COLUMNS} FROM import_character_dossier \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportCharacterDossier::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_world_dossiers_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportWorldDossier>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_WORLD_DOSSIER_COLUMNS} FROM import_world_dossier \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportWorldDossier::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_narrative_dossiers_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportNarrativeDossier>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_NARRATIVE_DOSSIER_COLUMNS} FROM import_narrative_dossier \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportNarrativeDossier::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_resume_snapshots_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportResumeSnapshot>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_RESUME_SNAPSHOT_COLUMNS} FROM import_resume_snapshot \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportResumeSnapshot::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_review_items_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportReviewItem>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_REVIEW_ITEM_COLUMNS} FROM import_review_item \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportReviewItem::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
