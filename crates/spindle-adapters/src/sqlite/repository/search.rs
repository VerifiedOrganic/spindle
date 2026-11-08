use super::*;

impl Repository {
    /// Trigger an embedding refresh for one entity. The repository's role is
    /// scoped to the storage layer — the actual SearchDocument construction
    /// (which fields to index, how to summarize, etc.) lives at the service
    /// layer. This method is the entry point services call after mutating
    /// an entity; they pass a freshly-built SearchDocument.
    pub async fn refresh_search_embedding_for_entity(
        &self,
        project_id: &str,
        branch_id: &str,
        entity_id: &str,
        document: &SearchDocument,
    ) -> Result<()> {
        self.upsert_search_embedding_document(project_id, branch_id, entity_id, document)
            .await
    }

    // =========================================================================
    // Search embeddings
    // =========================================================================
    //
    // The full search flow (kNN + FTS5) lands in Phase 5 once the vec0
    // virtual table is wired. Phase 4 here covers the write path that Phase 5
    // will consume: upsert_search_embedding_document does the embed-and-store,
    // delete_search_embedding_row removes a row, list returns rows for a
    // (project, branch) pair so the index can be rebuilt.

    pub async fn upsert_search_embedding_document(
        &self,
        project_id: &str,
        branch_id: &str,
        entity_id: &str,
        document: &SearchDocument,
    ) -> Result<()> {
        let embedding_session = self.inner.model_router.embedding_session()?;
        let embedding = embedding_session.embed_text(&document.content).await?;
        let embedding_blob = pack_embedding(&embedding);
        let version = self.inner.model_router.embedding_version()?;
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let entity_id_owned = entity_id.to_string();
        let entity_table = document.entity_table.clone();
        let title = document.title.clone();
        let excerpt = document.excerpt.clone();
        let content = document.content.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        let new_id = mint_id("search_embedding");

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO search_embedding (id, project_id, branch_id, entity_table, \
                     entity_id, title, excerpt, content, embedding_version, embedding, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
                     ON CONFLICT (project_id, branch_id, entity_table, entity_id) DO UPDATE SET \
                        title = excluded.title, \
                        excerpt = excluded.excerpt, \
                        content = excluded.content, \
                        embedding_version = excluded.embedding_version, \
                        embedding = excluded.embedding, \
                        updated_at = excluded.updated_at",
                    rusqlite::params![
                        &new_id,
                        &project_id,
                        &branch_id,
                        &entity_table,
                        &entity_id_owned,
                        &title,
                        &excerpt,
                        &content,
                        &version,
                        &embedding_blob,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_search_embeddings_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<SearchEmbedding>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SEARCH_EMBEDDING_COLUMNS} FROM search_embedding \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY entity_table, entity_id"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        SearchEmbedding::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Phase 5 lexical search across scene prose using FTS5.
    /// Returns `(scene_id, rank, snippet)` tuples ordered by relevance.
    /// `rank` is FTS5's bm25-style score (lower = better match); `snippet` is
    /// a 32-token preview of the matching prose with `<mark>` tags around the
    /// matched terms.
    #[allow(clippy::type_complexity)]
    pub async fn fts_search_scenes(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f64, String)>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.map(|s| s.to_string());
        let query = query.to_string();
        let limit = limit as i64;
        self.inner
            .pool
            .read(move |conn| {
                let (sql, has_branch) = if branch_id.is_some() {
                    (
                        "SELECT scene_id, rank, snippet(fts_scene, 4, '<mark>', '</mark>', '…', 32) \
                         FROM fts_scene \
                         WHERE fts_scene MATCH ?1 AND project_id = ?2 AND branch_id = ?3 \
                         ORDER BY rank LIMIT ?4",
                        true,
                    )
                } else {
                    (
                        "SELECT scene_id, rank, snippet(fts_scene, 4, '<mark>', '</mark>', '…', 32) \
                         FROM fts_scene \
                         WHERE fts_scene MATCH ?1 AND project_id = ?2 \
                         ORDER BY rank LIMIT ?3",
                        false,
                    )
                };
                let mut stmt = conn.prepare_cached(sql)?;
                let rows = if has_branch {
                    stmt.query_map(
                        rusqlite::params![&query, &project_id, &branch_id.unwrap(), limit],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, String>(2)?)),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                } else {
                    stmt.query_map(rusqlite::params![&query, &project_id, limit], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, String>(2)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                Ok(rows)
            })
            .await
    }

    /// Lexical search across characters. Returns `(character_id, rank, snippet)`.
    pub async fn fts_search_characters(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f64, String)>> {
        fts_search_named(
            &self.inner.pool,
            "fts_character",
            "character_id",
            -1, // any matched column
            project_id,
            branch_id,
            query,
            limit,
        )
        .await
    }

    /// Lexical search across locations.
    pub async fn fts_search_locations(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f64, String)>> {
        fts_search_named(
            &self.inner.pool,
            "fts_location",
            "location_id",
            -1,
            project_id,
            branch_id,
            query,
            limit,
        )
        .await
    }

    /// Lexical search across world rules.
    pub async fn fts_search_world_rules(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f64, String)>> {
        fts_search_named(
            &self.inner.pool,
            "fts_world_rule",
            "world_rule_id",
            -1,
            project_id,
            branch_id,
            query,
            limit,
        )
        .await
    }

    /// k-nearest-neighbor search over the vec0 mirror of search_embedding.
    /// Returns `(SearchEmbedding, distance)` pairs ordered by distance.
    pub async fn knn_search_embeddings(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query_embedding: &[f64],
        k: usize,
    ) -> Result<Vec<(SearchEmbedding, f64)>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.map(|s| s.to_string());
        let query_blob = pack_embedding(query_embedding);
        let k = k as i64;
        self.inner
            .pool
            .read(move |conn| {
                // se.* explicitly to avoid `embedding` name collision with the
                // vec0 table's own embedding column.
                let qualified_cols = SEARCH_EMBEDDING_COLUMNS
                    .split(',')
                    .map(|c| format!("se.{}", c.trim()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut sql = format!(
                    "SELECT {qualified_cols}, vec.distance \
                     FROM vec_search_embedding AS vec \
                     JOIN search_embedding AS se ON se.id = vec.se_id \
                     WHERE vec.embedding MATCH ?1 AND vec.k = ?2 \
                       AND se.project_id = ?3"
                );
                if branch_id.is_some() {
                    sql.push_str(" AND se.branch_id = ?4");
                }
                sql.push_str(" ORDER BY vec.distance");
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = if let Some(b) = &branch_id {
                    stmt.query_map(rusqlite::params![&query_blob, k, &project_id, b], |r| {
                        let se = SearchEmbedding::try_from(r)?;
                        let dist: f64 = r.get(11)?;
                        Ok((se, dist))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                } else {
                    stmt.query_map(rusqlite::params![&query_blob, k, &project_id], |r| {
                        let se = SearchEmbedding::try_from(r)?;
                        let dist: f64 = r.get(11)?;
                        Ok((se, dist))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                Ok(rows)
            })
            .await
    }

    pub async fn delete_search_embedding_for_entity(
        &self,
        project_id: &str,
        entity_id: &str,
    ) -> Result<bool> {
        let project_id = project_id.to_string();
        let entity_id = entity_id.to_string();
        let n: usize = self
            .inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM search_embedding WHERE project_id = ?1 AND entity_id = ?2",
                    rusqlite::params![&project_id, &entity_id],
                )
            })
            .await?;
        Ok(n > 0)
    }

    pub async fn delete_search_embeddings_for_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM search_embedding WHERE project_id = ?1 AND branch_id = ?2",
                    rusqlite::params![&project_id, &branch_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Clear the search-embedding index for a project's active branch.
    /// The caller is expected to iterate searchable entities and re-upsert
    /// embeddings via `upsert_search_embedding_document` afterwards. Returns
    /// the number of rows removed.
    pub async fn rebuild_search_index_clear(&self, project_id: &str) -> Result<usize> {
        let branch_id = self.active_branch_id(project_id).await?;
        let project_id = project_id.to_string();
        let n: usize = self
            .inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM search_embedding WHERE project_id = ?1 AND branch_id = ?2",
                    rusqlite::params![&project_id, &branch_id],
                )
            })
            .await?;
        Ok(n)
    }
}
