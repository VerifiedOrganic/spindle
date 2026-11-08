use super::*;

impl Repository {
    pub async fn create_research_log(
        &self,
        project_id: &str,
        query: &str,
        context_hint: Option<&str>,
        model: &str,
        response: &str,
        context_summary: &str,
    ) -> Result<ResearchLog> {
        let id = mint_id("research_log");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let query_owned = query.to_string();
        let context_hint = context_hint.map(|s| s.to_string());
        let model_owned = model.to_string();
        let response_owned = response.to_string();
        let summary = context_summary.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO research_log (id, project_id, query, context_hint, model, \
                     response, context_summary, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &query_owned,
                        &context_hint,
                        &model_owned,
                        &response_owned,
                        &summary,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        let id_lookup = id_out;
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {RESEARCH_LOG_COLUMNS} FROM research_log WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| ResearchLog::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("research_log vanished after insert"))
    }

    pub async fn list_research_logs_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ResearchLog>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RESEARCH_LOG_COLUMNS} FROM research_log \
                     WHERE project_id = ?1 ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ResearchLog::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn get_research_source(&self, id: &str) -> Result<Option<ResearchSource>> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {RESEARCH_SOURCE_COLUMNS} FROM research_source WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ResearchSource::try_from(r))
                    .optional_inner()
            })
            .await
    }

    pub async fn get_research_note(&self, id: &str) -> Result<Option<ResearchNote>> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {RESEARCH_NOTE_COLUMNS} FROM research_note WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ResearchNote::try_from(r))
                    .optional_inner()
            })
            .await
    }

    pub async fn get_research_claim(&self, id: &str) -> Result<Option<ResearchClaim>> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {RESEARCH_CLAIM_COLUMNS} FROM research_claim WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ResearchClaim::try_from(r))
                    .optional_inner()
            })
            .await
    }

    pub async fn get_research_usage(&self, id: &str) -> Result<Option<ResearchUsage>> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {RESEARCH_USAGE_COLUMNS} FROM research_usage WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ResearchUsage::try_from(r))
                    .optional_inner()
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_research_source(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        title: &str,
        source_type: &str,
        url: Option<&str>,
        file_path: Option<&str>,
        author: Option<&str>,
        publisher: Option<&str>,
        published_date: Option<&str>,
        accessed_at: chrono::DateTime<chrono::Utc>,
        reliability: &str,
        tags: &[String],
        summary: Option<&str>,
    ) -> Result<ResearchSource> {
        let id = mint_id("research_source");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let branch_id = branch_id.map(|s| s.to_string());
        let title = title.to_string();
        let source_type = source_type.to_string();
        let url = url.map(|s| s.to_string());
        let file_path = file_path.map(|s| s.to_string());
        let author = author.map(|s| s.to_string());
        let publisher = publisher.map(|s| s.to_string());
        let published_date = published_date.map(|s| s.to_string());
        let accessed_at_micros = timestamp_to_micros(accessed_at);
        let reliability = reliability.to_string();
        let tags_json = serde_json::to_string(tags)?;
        let summary = summary.map(|s| s.to_string());
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO research_source (id, project_id, branch_id, title, source_type, \
                     url, file_path, author, publisher, published_date, accessed_at, reliability, tags, summary, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &title,
                        &source_type,
                        &url,
                        &file_path,
                        &author,
                        &publisher,
                        &published_date,
                        accessed_at_micros,
                        &reliability,
                        &tags_json,
                        &summary,
                        now,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;

        let id_lookup = id_out;
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {RESEARCH_SOURCE_COLUMNS} FROM research_source WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| ResearchSource::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("research_source vanished after insert"))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_research_note(
        &self,
        project_id: &str,
        source_id: Option<&str>,
        branch_id: Option<&str>,
        note: &str,
        quote: Option<&str>,
        locator: Option<&str>,
        tags: &[String],
    ) -> Result<ResearchNote> {
        let id = mint_id("research_note");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let source_id = source_id.map(|s| s.to_string());
        let branch_id = branch_id.map(|s| s.to_string());
        let note = note.to_string();
        let quote = quote.map(|s| s.to_string());
        let locator = locator.map(|s| s.to_string());
        let tags_json = serde_json::to_string(tags)?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO research_note (id, project_id, source_id, branch_id, note, \
                     quote, locator, tags, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &source_id,
                        &branch_id,
                        &note,
                        &quote,
                        &locator,
                        &tags_json,
                        now,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;

        let id_lookup = id_out;
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {RESEARCH_NOTE_COLUMNS} FROM research_note WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| ResearchNote::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("research_note vanished after insert"))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_research_claim(
        &self,
        project_id: &str,
        source_id: Option<&str>,
        note_id: Option<&str>,
        branch_id: Option<&str>,
        claim: &str,
        topic: Option<&str>,
        time_period: Option<&str>,
        location: Option<&str>,
        confidence: &str,
        tags: &[String],
    ) -> Result<ResearchClaim> {
        let id = mint_id("research_claim");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let source_id = source_id.map(|s| s.to_string());
        let note_id = note_id.map(|s| s.to_string());
        let branch_id = branch_id.map(|s| s.to_string());
        let claim = claim.to_string();
        let topic = topic.map(|s| s.to_string());
        let time_period = time_period.map(|s| s.to_string());
        let location = location.map(|s| s.to_string());
        let confidence = confidence.to_string();
        let tags_json = serde_json::to_string(tags)?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO research_claim (id, project_id, source_id, note_id, branch_id, claim, \
                     topic, time_period, location, confidence, tags, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &source_id,
                        &note_id,
                        &branch_id,
                        &claim,
                        &topic,
                        &time_period,
                        &location,
                        &confidence,
                        &tags_json,
                        now,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;

        let id_lookup = id_out;
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {RESEARCH_CLAIM_COLUMNS} FROM research_claim WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| ResearchClaim::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("research_claim vanished after insert"))
    }

    pub async fn list_research_sources_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ResearchSource>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RESEARCH_SOURCE_COLUMNS} FROM research_source \
                     WHERE project_id = ?1 AND archived_at IS NULL ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ResearchSource::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_research_notes_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ResearchNote>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RESEARCH_NOTE_COLUMNS} FROM research_note \
                     WHERE project_id = ?1 AND archived_at IS NULL ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ResearchNote::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_research_claims_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ResearchClaim>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RESEARCH_CLAIM_COLUMNS} FROM research_claim \
                     WHERE project_id = ?1 AND archived_at IS NULL ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ResearchClaim::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_research_usage(
        &self,
        project_id: &str,
        branch_id: &str,
        run_id: &str,
        step_checkpoint_id: Option<&str>,
        scene_id: &str,
        source_ids: &[String],
        note_ids: &[String],
        claim_ids: &[String],
        query_pack_input: &str,
        context_hash: &str,
    ) -> Result<ResearchUsage> {
        let id = mint_id("research_usage");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let run_id = run_id.to_string();
        let step_checkpoint_id = step_checkpoint_id.map(|s| s.to_string());
        let scene_id = scene_id.to_string();
        let source_ids_json = serde_json::to_string(source_ids)?;
        let note_ids_json = serde_json::to_string(note_ids)?;
        let claim_ids_json = serde_json::to_string(claim_ids)?;
        let query_pack_input = query_pack_input.to_string();
        let context_hash = context_hash.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO research_usage (id, project_id, branch_id, run_id, step_checkpoint_id, scene_id, \
                     source_ids, note_ids, claim_ids, query_pack_input, context_hash, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &run_id,
                        &step_checkpoint_id,
                        &scene_id,
                        &source_ids_json,
                        &note_ids_json,
                        &claim_ids_json,
                        &query_pack_input,
                        &context_hash,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;

        let id_lookup = id_out;
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {RESEARCH_USAGE_COLUMNS} FROM research_usage WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| ResearchUsage::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("research_usage vanished after insert"))
    }

    pub async fn list_research_usages_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ResearchUsage>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RESEARCH_USAGE_COLUMNS} FROM research_usage \
                     WHERE project_id = ?1 ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ResearchUsage::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_research_usages_for_scene(
        &self,
        project_id: &str,
        scene_id: &str,
    ) -> Result<Vec<ResearchUsage>> {
        let project_id = project_id.to_string();
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RESEARCH_USAGE_COLUMNS} FROM research_usage \
                     WHERE project_id = ?1 AND scene_id = ?2 ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id, &scene_id], |r| ResearchUsage::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn research_tags_by_project(&self, project_id: &str) -> Result<Vec<String>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let mut tags_set = std::collections::BTreeSet::new();

                let mut stmt1 = conn.prepare_cached(
                    "SELECT tags FROM research_source WHERE project_id = ?1 AND archived_at IS NULL",
                )?;
                let mut rows1 = stmt1.query([&project_id])?;
                while let Some(row) = rows1.next()? {
                    let tags_json: String = row.get(0)?;
                    if let Ok(tags) = serde_json::from_str::<Vec<String>>(&tags_json) {
                        for t in tags {
                            tags_set.insert(t);
                        }
                    }
                }

                let mut stmt2 = conn.prepare_cached(
                    "SELECT tags FROM research_note WHERE project_id = ?1 AND archived_at IS NULL",
                )?;
                let mut rows2 = stmt2.query([&project_id])?;
                while let Some(row) = rows2.next()? {
                    let tags_json: String = row.get(0)?;
                    if let Ok(tags) = serde_json::from_str::<Vec<String>>(&tags_json) {
                        for t in tags {
                            tags_set.insert(t);
                        }
                    }
                }

                let mut stmt3 = conn.prepare_cached(
                    "SELECT tags FROM research_claim WHERE project_id = ?1 AND archived_at IS NULL",
                )?;
                let mut rows3 = stmt3.query([&project_id])?;
                while let Some(row) = rows3.next()? {
                    let tags_json: String = row.get(0)?;
                    if let Ok(tags) = serde_json::from_str::<Vec<String>>(&tags_json) {
                        for t in tags {
                            tags_set.insert(t);
                        }
                    }
                }

                Ok(tags_set.into_iter().collect())
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn search_research(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query: &str,
        tags: &[String],
        time_period: Option<&str>,
        location: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<spindle_core::models::ResearchSearchResultItem>> {
        let project_id = project_id.to_string();
        let query = fts_safe_query(query);
        let branch_id = branch_id.map(|s| s.to_string());
        let tags = tags.to_vec();
        let time_period = time_period.map(|s| s.trim().to_string());
        let location = location.map(|s| s.trim().to_string());
        let limit = limit.unwrap_or(20);

        self.inner
            .pool
            .read(move |conn| {
                let mut sources_map = std::collections::HashMap::new();

                struct MinimalSource {
                    id: String,
                    title: String,
                    url: Option<String>,
                    author: Option<String>,
                    reliability: String,
                    tags: Vec<String>,
                    summary: Option<String>,
                }

                let load_source = |row: &rusqlite::Row<'_>| -> rusqlite::Result<MinimalSource> {
                    let id: String = row.get(0)?;
                    let title: String = row.get(1)?;
                    let url: Option<String> = row.get(2)?;
                    let author: Option<String> = row.get(3)?;
                    let reliability: String = row.get(4)?;
                    let tags_json: String = row.get(5)?;
                    let tags_parsed: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                    let summary: Option<String> = row.get(6)?;
                    Ok(MinimalSource {
                        id,
                        title,
                        url,
                        author,
                        reliability,
                        tags: tags_parsed,
                        summary,
                    })
                };

                if let Some(branch_id) = &branch_id {
                    let mut sources_stmt = conn.prepare_cached(
                        "SELECT id, title, url, author, reliability, tags, summary \
                         FROM research_source WHERE project_id = ?1 AND (branch_id = ?2 OR branch_id IS NULL) AND archived_at IS NULL"
                    )?;
                    let mut sources_rows = sources_stmt.query(rusqlite::params![&project_id, branch_id])?;
                    while let Some(row) = sources_rows.next()? {
                        let source = load_source(row)?;
                        sources_map.insert(source.id.clone(), source);
                    }
                } else {
                    let mut sources_stmt = conn.prepare_cached(
                        "SELECT id, title, url, author, reliability, tags, summary FROM research_source WHERE project_id = ?1 AND archived_at IS NULL"
                    )?;
                    let mut sources_rows = sources_stmt.query([&project_id])?;
                    while let Some(row) = sources_rows.next()? {
                        let source = load_source(row)?;
                        sources_map.insert(source.id.clone(), source);
                    }
                }

                let mut matched_sources = Vec::new();
                if let Some(query) = &query {
                    if let Some(branch_id) = &branch_id {
                        let mut stmt = conn.prepare_cached(
                            "SELECT s.id FROM research_source s JOIN fts_research_source f ON s.id = f.source_id \
                             WHERE fts_research_source MATCH ?1 AND s.project_id = ?2 AND (s.branch_id = ?3 OR s.branch_id IS NULL)"
                        )?;
                        let mut rows = stmt.query(rusqlite::params![query, &project_id, branch_id])?;
                        while let Some(row) = rows.next()? {
                            let id: String = row.get(0)?;
                            if let Some(src) = sources_map.get(&id) {
                                matched_sources.push(src);
                            }
                        }
                    } else {
                        let mut stmt = conn.prepare_cached(
                            "SELECT s.id FROM research_source s JOIN fts_research_source f ON s.id = f.source_id \
                             WHERE fts_research_source MATCH ?1 AND s.project_id = ?2"
                        )?;
                        let mut rows = stmt.query(rusqlite::params![query, &project_id])?;
                        while let Some(row) = rows.next()? {
                            let id: String = row.get(0)?;
                            if let Some(src) = sources_map.get(&id) {
                                matched_sources.push(src);
                            }
                        }
                    }
                } else {
                    for src in sources_map.values() {
                        matched_sources.push(src);
                    }
                }

                let mut matched_notes = Vec::new();
                struct MinimalNote {
                    id: String,
                    source_id: Option<String>,
                    note: String,
                    quote: Option<String>,
                    locator: Option<String>,
                    tags: Vec<String>,
                }
                if let Some(query) = &query {
                    if let Some(branch_id) = &branch_id {
                        let mut stmt = conn.prepare_cached(
                            "SELECT n.id, n.source_id, n.note, n.quote, n.locator, n.tags \
                             FROM research_note n JOIN fts_research_note f ON n.id = f.note_id \
                             WHERE fts_research_note MATCH ?1 AND n.project_id = ?2 AND (n.branch_id = ?3 OR n.branch_id IS NULL) AND n.archived_at IS NULL"
                        )?;
                        let mut rows = stmt.query(rusqlite::params![query, &project_id, branch_id])?;
                        while let Some(row) = rows.next()? {
                            let tags_json: String = row.get(5)?;
                            let tags_parsed: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                            matched_notes.push(MinimalNote {
                                id: row.get(0)?,
                                source_id: row.get(1)?,
                                note: row.get(2)?,
                                quote: row.get(3)?,
                                locator: row.get(4)?,
                                tags: tags_parsed,
                            });
                        }
                    } else {
                        let mut stmt = conn.prepare_cached(
                            "SELECT n.id, n.source_id, n.note, n.quote, n.locator, n.tags \
                             FROM research_note n JOIN fts_research_note f ON n.id = f.note_id \
                             WHERE fts_research_note MATCH ?1 AND n.project_id = ?2 AND n.archived_at IS NULL"
                        )?;
                        let mut rows = stmt.query(rusqlite::params![query, &project_id])?;
                        while let Some(row) = rows.next()? {
                            let tags_json: String = row.get(5)?;
                            let tags_parsed: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                            matched_notes.push(MinimalNote {
                                id: row.get(0)?,
                                source_id: row.get(1)?,
                                note: row.get(2)?,
                                quote: row.get(3)?,
                                locator: row.get(4)?,
                                tags: tags_parsed,
                            });
                        }
                    }
                } else {
                    let (sql, has_branch) = if branch_id.is_some() {
                        (
                            "SELECT id, source_id, note, quote, locator, tags FROM research_note \
                             WHERE project_id = ?1 AND (branch_id = ?2 OR branch_id IS NULL) AND archived_at IS NULL",
                            true,
                        )
                    } else {
                        (
                            "SELECT id, source_id, note, quote, locator, tags FROM research_note WHERE project_id = ?1 AND archived_at IS NULL",
                            false,
                        )
                    };
                    let mut stmt = conn.prepare_cached(sql)?;
                    let mut rows = if has_branch {
                        stmt.query(rusqlite::params![&project_id, branch_id.as_ref().unwrap()])?
                    } else {
                        stmt.query(rusqlite::params![&project_id])?
                    };
                    while let Some(row) = rows.next()? {
                        let tags_json: String = row.get(5)?;
                        let tags_parsed: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                        matched_notes.push(MinimalNote {
                            id: row.get(0)?,
                            source_id: row.get(1)?,
                            note: row.get(2)?,
                            quote: row.get(3)?,
                            locator: row.get(4)?,
                            tags: tags_parsed,
                        });
                    }
                }

                let mut matched_claims = Vec::new();
                struct MinimalClaim {
                    id: String,
                    source_id: Option<String>,
                    claim: String,
                    topic: Option<String>,
                    time_period: Option<String>,
                    location: Option<String>,
                    confidence: String,
                    tags: Vec<String>,
                }
                if let Some(query) = &query {
                    if let Some(branch_id) = &branch_id {
                        let mut stmt = conn.prepare_cached(
                            "SELECT c.id, c.source_id, c.claim, c.topic, c.time_period, c.location, c.confidence, c.tags \
                             FROM research_claim c JOIN fts_research_claim f ON c.id = f.claim_id \
                             WHERE fts_research_claim MATCH ?1 AND c.project_id = ?2 AND (c.branch_id = ?3 OR c.branch_id IS NULL) AND c.archived_at IS NULL"
                        )?;
                        let mut rows = stmt.query(rusqlite::params![query, &project_id, branch_id])?;
                        while let Some(row) = rows.next()? {
                            let tags_json: String = row.get(7)?;
                            let tags_parsed: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                            matched_claims.push(MinimalClaim {
                                id: row.get(0)?,
                                source_id: row.get(1)?,
                                claim: row.get(2)?,
                                topic: row.get(3)?,
                                time_period: row.get(4)?,
                                location: row.get(5)?,
                                confidence: row.get(6)?,
                                tags: tags_parsed,
                            });
                        }
                    } else {
                        let mut stmt = conn.prepare_cached(
                            "SELECT c.id, c.source_id, c.claim, c.topic, c.time_period, c.location, c.confidence, c.tags \
                             FROM research_claim c JOIN fts_research_claim f ON c.id = f.claim_id \
                             WHERE fts_research_claim MATCH ?1 AND c.project_id = ?2 AND c.archived_at IS NULL"
                        )?;
                        let mut rows = stmt.query(rusqlite::params![query, &project_id])?;
                        while let Some(row) = rows.next()? {
                            let tags_json: String = row.get(7)?;
                            let tags_parsed: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                            matched_claims.push(MinimalClaim {
                                id: row.get(0)?,
                                source_id: row.get(1)?,
                                claim: row.get(2)?,
                                topic: row.get(3)?,
                                time_period: row.get(4)?,
                                location: row.get(5)?,
                                confidence: row.get(6)?,
                                tags: tags_parsed,
                            });
                        }
                    }
                } else {
                    let (sql, has_branch) = if branch_id.is_some() {
                        (
                            "SELECT id, source_id, claim, topic, time_period, location, confidence, tags \
                             FROM research_claim WHERE project_id = ?1 AND (branch_id = ?2 OR branch_id IS NULL) AND archived_at IS NULL",
                            true,
                        )
                    } else {
                        (
                            "SELECT id, source_id, claim, topic, time_period, location, confidence, tags \
                             FROM research_claim WHERE project_id = ?1 AND archived_at IS NULL",
                            false,
                        )
                    };
                    let mut stmt = conn.prepare_cached(sql)?;
                    let mut rows = if has_branch {
                        stmt.query(rusqlite::params![&project_id, branch_id.as_ref().unwrap()])?
                    } else {
                        stmt.query(rusqlite::params![&project_id])?
                    };
                    while let Some(row) = rows.next()? {
                        let tags_json: String = row.get(7)?;
                        let tags_parsed: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                        matched_claims.push(MinimalClaim {
                            id: row.get(0)?,
                            source_id: row.get(1)?,
                            claim: row.get(2)?,
                            topic: row.get(3)?,
                            time_period: row.get(4)?,
                            location: row.get(5)?,
                            confidence: row.get(6)?,
                            tags: tags_parsed,
                        });
                    }
                }

                let mut results = Vec::new();

                let matches_tags = |item_tags: &[String]| -> bool {
                    if tags.is_empty() {
                        return true;
                    }
                    tags.iter().all(|t| item_tags.iter().any(|it| it.eq_ignore_ascii_case(t)))
                };

                for src in matched_sources {
                    if !matches_tags(&src.tags) {
                        continue;
                    }
                    results.push(spindle_core::models::ResearchSearchResultItem {
                        item_type: "source".to_string(),
                        id: src.id.clone(),
                        title_or_summary: src.title.clone(),
                        preview: src.summary.clone().unwrap_or_default(),
                        tags: src.tags.clone(),
                        source_title: Some(src.title.clone()),
                        source_url: src.url.clone(),
                        source_author: src.author.clone(),
                        locator: None,
                        confidence_or_reliability: Some(src.reliability.clone()),
                    });
                }

                for note in matched_notes {
                    if !matches_tags(&note.tags) {
                        continue;
                    }
                    let (src_title, src_url, src_author) = if let Some(sid) = &note.source_id {
                        if let Some(src) = sources_map.get(sid) {
                            (Some(src.title.clone()), src.url.clone(), src.author.clone())
                        } else {
                            (None, None, None)
                        }
                    } else {
                        (None, None, None)
                    };

                    let note_text = note.note.clone();
                    let preview = if let Some(q) = &note.quote {
                        format!("\"{}\"\n— {}", q, note_text)
                    } else {
                        note_text.clone()
                    };

                    results.push(spindle_core::models::ResearchSearchResultItem {
                        item_type: "note".to_string(),
                        id: note.id,
                        title_or_summary: if note.note.len() > 60 { format!("{}...", &note.note[..57]) } else { note.note.clone() },
                        preview,
                        tags: note.tags,
                        source_title: src_title,
                        source_url: src_url,
                        source_author: src_author,
                        locator: note.locator,
                        confidence_or_reliability: None,
                    });
                }

                for claim in matched_claims {
                    if !matches_tags(&claim.tags) {
                        continue;
                    }
                    if let Some(tp) = &time_period {
                        if let Some(ctp) = &claim.time_period {
                            if !ctp.to_lowercase().contains(&tp.to_lowercase()) {
                                continue;
                            }
                        } else {
                            continue;
                        }
                    }
                    if let Some(loc) = &location {
                        if let Some(cloc) = &claim.location {
                            if !cloc.to_lowercase().contains(&loc.to_lowercase()) {
                                continue;
                            }
                        } else {
                            continue;
                        }
                    }

                    let (src_title, src_url, src_author) = if let Some(sid) = &claim.source_id {
                        if let Some(src) = sources_map.get(sid) {
                            (Some(src.title.clone()), src.url.clone(), src.author.clone())
                        } else {
                            (None, None, None)
                        }
                    } else {
                        (None, None, None)
                    };

                    results.push(spindle_core::models::ResearchSearchResultItem {
                        item_type: "claim".to_string(),
                        id: claim.id,
                        title_or_summary: claim.claim.clone(),
                        preview: format!("Factual Claim: {}\nTopic: {}\nConfidence: {}",
                            claim.claim,
                            claim.topic.clone().unwrap_or_else(|| "N/A".to_string()),
                            claim.confidence
                        ),
                        tags: claim.tags,
                        source_title: src_title,
                        source_url: src_url,
                        source_author: src_author,
                        locator: None,
                        confidence_or_reliability: Some(claim.confidence),
                    });
                }

                results.truncate(limit);
                Ok(results)
            })
            .await
    }
}
