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
        if self.pool().in_application_transaction() {
            return self
                .queue_planning_embedding(project_id, branch_id, entity_id, document)
                .await;
        }
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
        let generation = self.active_embedding_generation(project_id).await?;
        let embedding = self
            .inner
            .model_router
            .embed_for_generation(
                &document.content,
                generation.spec.provider,
                generation.spec.dimension,
            )
            .await?;
        self.upsert_embedding_document_for_generation(
            project_id,
            Some(branch_id),
            entity_id,
            document,
            &generation,
            &embedding,
        )
        .await
    }

    /// Embed a query with the project's active generation identity. A query
    /// from a different provider/model/dimension is rejected, never compared.
    pub async fn embed_query_for_active_generation(
        &self,
        project_id: &str,
        query: &str,
    ) -> Result<spindle_core::embeddings::EmbeddingQueryVectorV1> {
        let generation = self.active_embedding_generation(project_id).await?;
        let vector = self
            .inner
            .model_router
            .embed_for_generation(query, generation.spec.provider, generation.spec.dimension)
            .await?;
        Ok(spindle_core::embeddings::EmbeddingQueryVectorV1::new(
            generation.generation_id,
            generation.spec.dimension,
            vector,
        )?)
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
        query: &spindle_core::embeddings::EmbeddingQueryVectorV1,
        k: usize,
    ) -> Result<Vec<(SearchEmbedding, f64)>> {
        let generation = self.active_embedding_generation(project_id).await?;
        self.knn_search_embeddings_for_generation(project_id, branch_id, &generation, query, k)
            .await
    }

    pub async fn delete_search_embedding_for_entity(
        &self,
        project_id: &str,
        entity_id: &str,
    ) -> Result<bool> {
        let deleted = self
            .delete_embedding_rows_for_entity(project_id, entity_id)
            .await?;
        Ok(deleted > 0)
    }

    pub async fn delete_search_embeddings_for_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<()> {
        self.delete_embedding_rows_for_branch(project_id, branch_id)
            .await?;
        Ok(())
    }

    /// Clear the search-embedding index for a project's active branch.
    /// The caller is expected to iterate searchable entities and re-upsert
    /// embeddings via `upsert_search_embedding_document` afterwards. Returns
    /// the number of rows removed.
    pub async fn rebuild_search_index_clear(&self, project_id: &str) -> Result<usize> {
        let branch_id = self.active_branch_id(project_id).await?;
        let generation = self.active_embedding_generation(project_id).await?;
        let project_id = project_id.to_string();
        let generation_id = generation.generation_id.clone();
        let table_name = generation.table_name.clone();
        anyhow::ensure!(
            table_name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
            "unsafe embedding table name"
        );
        let n: usize = self
            .inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                let ids: Vec<String> = {
                    let mut stmt = tx.prepare(
                        "SELECT id FROM search_embedding \
                         WHERE project_id = ?1 AND branch_id = ?2 AND generation_id = ?3",
                    )?;
                    stmt.query_map(
                        rusqlite::params![&project_id, &branch_id, &generation_id],
                        |row| row.get(0),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                for id in &ids {
                    tx.execute(&format!("DELETE FROM {table_name} WHERE se_id = ?1"), [id])?;
                }
                let deleted = tx.execute(
                    "DELETE FROM search_embedding \
                     WHERE project_id = ?1 AND branch_id = ?2 AND generation_id = ?3",
                    rusqlite::params![&project_id, &branch_id, &generation_id],
                )?;
                tx.commit()?;
                Ok(deleted)
            })
            .await?;
        Ok(n)
    }
}
