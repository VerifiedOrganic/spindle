//! Provider/dimension-aware embedding index generations (P2 PR H).

use super::*;
use spindle_core::embeddings::*;
use std::collections::{BTreeMap, BTreeSet};

fn generation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EmbeddingGenerationV1> {
    Ok(EmbeddingGenerationV1 {
        generation_id: row.get(0)?,
        spec: EmbeddingGenerationSpecV1 {
            provider: match row.get::<_, String>(1)?.as_str() {
                "http" => EmbeddingProviderKindV1::Http,
                _ => EmbeddingProviderKindV1::TokenHash,
            },
            model: row.get(2)?,
            dimension: row.get::<_, i64>(3)? as usize,
            metric: match row.get::<_, String>(4)?.as_str() {
                "l2" => EmbeddingMetricV1::L2,
                _ => EmbeddingMetricV1::Cosine,
            },
            normalization: match row.get::<_, String>(5)?.as_str() {
                "none" => EmbeddingNormalizationV1::None,
                _ => EmbeddingNormalizationV1::L2,
            },
            version: row.get(6)?,
        },
        table_name: row.get(7)?,
        status: match row.get::<_, String>(8)?.as_str() {
            "building" => EmbeddingGenerationStatusV1::Building,
            "active" => EmbeddingGenerationStatusV1::Active,
            "retired" => EmbeddingGenerationStatusV1::Retired,
            _ => EmbeddingGenerationStatusV1::Failed,
        },
        created_at_micros: row.get(9)?,
        activated_at_micros: row.get(10)?,
        retired_at_micros: row.get(11)?,
        rollback_until_micros: row.get(12)?,
    })
}

fn safe_identifier(table_name: &str) -> bool {
    !table_name.is_empty()
        && table_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn generation_sql_columns() -> &'static str {
    "generation_id, provider, model, dimension, metric, normalization, version, \
     table_name, status, created_at, activated_at, retired_at, rollback_until"
}

impl Repository {
    pub async fn get_embedding_generation(
        &self,
        generation_id: &str,
    ) -> Result<Option<EmbeddingGenerationV1>> {
        let generation_id = generation_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM embedding_generation WHERE generation_id = ?1",
                        generation_sql_columns()
                    ),
                    [&generation_id],
                    generation_from_row,
                )
                .optional_inner()
            })
            .await
    }

    pub async fn list_embedding_generations(&self) -> Result<Vec<EmbeddingGenerationV1>> {
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM embedding_generation ORDER BY created_at, generation_id",
                    generation_sql_columns()
                ))?;
                let rows = stmt.query_map([], generation_from_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    pub async fn get_embedding_index_state(
        &self,
        project_id: &str,
    ) -> Result<Option<EmbeddingIndexStateV1>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                conn.query_row(
                    "SELECT project_id, active_generation_id, previous_generation_id, \
                            rollback_until, updated_at \
                     FROM embedding_index_state WHERE project_id = ?1",
                    [&project_id],
                    |row| {
                        Ok(EmbeddingIndexStateV1 {
                            project_id: row.get(0)?,
                            active_generation_id: row.get(1)?,
                            previous_generation_id: row.get(2)?,
                            rollback_until_micros: row.get(3)?,
                            updated_at_micros: row.get(4)?,
                        })
                    },
                )
                .optional_inner()
            })
            .await
    }

    /// The project's active generation; projects created after V0056 fall back
    /// to the preserved legacy token-hash generation until they are migrated.
    pub async fn active_embedding_generation(
        &self,
        project_id: &str,
    ) -> Result<EmbeddingGenerationV1> {
        let active_id = self
            .get_embedding_index_state(project_id)
            .await?
            .map(|state| state.active_generation_id)
            .unwrap_or_else(|| LEGACY_TOKEN_HASH_GENERATION_ID.to_string());
        self.get_embedding_generation(&active_id)
            .await?
            .ok_or_else(|| anyhow!(EmbeddingValidationError::NoActiveGeneration.to_string()))
    }

    /// Create the generation row and its per-dimension vec0 table. Reusing an
    /// existing generation id resumes a previously interrupted build.
    pub async fn create_embedding_generation(
        &self,
        spec: EmbeddingGenerationSpecV1,
    ) -> Result<EmbeddingGenerationV1> {
        spec.validate()?;
        let generation_id = spec.generation_id();
        if let Some(existing) = self.get_embedding_generation(&generation_id).await? {
            return Ok(existing);
        }
        let table_name = spec.table_name();
        if !safe_identifier(&table_name) {
            anyhow::bail!("unsafe embedding generation table name");
        }
        let dimension = spec.dimension;
        let provider = match spec.provider {
            EmbeddingProviderKindV1::TokenHash => "token_hash",
            EmbeddingProviderKindV1::Http => "http",
        };
        let metric = match spec.metric {
            EmbeddingMetricV1::Cosine => "cosine",
            EmbeddingMetricV1::L2 => "l2",
        };
        let normalization = match spec.normalization {
            EmbeddingNormalizationV1::L2 => "l2",
            EmbeddingNormalizationV1::None => "none",
        };
        let model = spec.model.clone();
        let version = spec.version.clone();
        let generation_id_for_tx = generation_id.clone();
        let table_name_for_tx = table_name.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.with_transaction(move |tx| {
            tx.tx.execute(
                "INSERT INTO embedding_generation (
                    generation_id, provider, model, dimension, metric, normalization,
                    version, table_name, status, created_at, activated_at, retired_at,
                    rollback_until
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'building', ?9, NULL, NULL, NULL)",
                rusqlite::params![
                    &generation_id_for_tx,
                    provider,
                    &model,
                    dimension as i64,
                    metric,
                    normalization,
                    &version,
                    &table_name_for_tx,
                    now,
                ],
            )?;
            let create = format!(
                "CREATE VIRTUAL TABLE IF NOT EXISTS {table_name_for_tx} USING vec0(\
                 se_id TEXT PRIMARY KEY, embedding FLOAT[{dimension}])"
            );
            tx.tx.execute(&create, [])?;
            Ok(())
        })
        .await?;
        self.get_embedding_generation(&generation_id)
            .await?
            .ok_or_else(|| anyhow!("embedding generation vanished after create"))
    }

    pub async fn project_generation_status(
        &self,
        project_id: &str,
        generation_id: &str,
    ) -> Result<Option<EmbeddingGenerationStatusV1>> {
        let project_id = project_id.to_string();
        let generation_id = generation_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let status: Option<String> = conn
                    .query_row(
                        "SELECT status FROM embedding_generation_project \
                         WHERE project_id = ?1 AND generation_id = ?2",
                        rusqlite::params![&project_id, &generation_id],
                        |row| row.get(0),
                    )
                    .optional_inner()?;
                Ok(status.map(|status| match status.as_str() {
                    "building" => EmbeddingGenerationStatusV1::Building,
                    "active" => EmbeddingGenerationStatusV1::Active,
                    "retired" => EmbeddingGenerationStatusV1::Retired,
                    _ => EmbeddingGenerationStatusV1::Failed,
                }))
            })
            .await
    }

    pub async fn ensure_project_generation(
        &self,
        project_id: &str,
        generation_id: &str,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let generation_id = generation_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO embedding_generation_project (
                        project_id, generation_id, status, created_at
                     ) VALUES (?1, ?2, 'building', ?3)
                     ON CONFLICT(project_id, generation_id) DO NOTHING",
                    rusqlite::params![&project_id, &generation_id, now],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn resume_embedding_generation(
        &self,
        project_id: &str,
        generation_id: &str,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let generation_id = generation_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE embedding_generation_project SET status = 'building', \
                        failed_at = NULL \
                     WHERE project_id = ?1 AND generation_id = ?2 \
                       AND status IN ('failed', 'retired')",
                    rusqlite::params![&project_id, &generation_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn fail_embedding_generation(
        &self,
        project_id: &str,
        generation_id: &str,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let generation_id = generation_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE embedding_generation_project SET status = 'failed', failed_at = ?3 \
                     WHERE project_id = ?1 AND generation_id = ?2 AND status = 'building'",
                    rusqlite::params![&project_id, &generation_id, now],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Activate a completed building generation. The compare-and-swap on the
    /// expected previous active id makes concurrent switches fail instead of
    /// silently overwriting each other.
    pub async fn activate_embedding_generation(
        &self,
        project_id: &str,
        generation_id: &str,
        expected_previous_generation_id: &str,
        rollback_until_micros: i64,
    ) -> Result<EmbeddingIndexStateV1> {
        let project_id = project_id.to_string();
        let generation_id = generation_id.to_string();
        let expected_previous_generation_id = expected_previous_generation_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.with_transaction(move |tx| {
            let current: Option<String> = tx
                .tx
                .query_row(
                    "SELECT active_generation_id FROM embedding_index_state WHERE project_id = ?1",
                    [&project_id],
                    |row| row.get(0),
                )
                .optional_inner()?;
            let current = current.unwrap_or_else(|| LEGACY_TOKEN_HASH_GENERATION_ID.to_string());
            anyhow::ensure!(
                current == expected_previous_generation_id,
                "stale embedding index state: expected active {expected_previous_generation_id}, current {current}"
            );
            let status: String = tx.tx.query_row(
                "SELECT status FROM embedding_generation WHERE generation_id = ?1",
                [&generation_id],
                |row| row.get(0),
            )?;
            anyhow::ensure!(
                status == "building" || status == "active",
                "embedding generation {generation_id} is {status}, not buildable"
            );
            // Lifecycle changes are per project. The shared registry row and
            // its vec0 table must stay usable by other projects activating the
            // same spec.
            tx.tx.execute(
                "INSERT INTO embedding_generation_project (
                    project_id, generation_id, status, created_at, activated_at
                 ) VALUES (?1, ?2, 'active', ?3, ?3)
                 ON CONFLICT(project_id, generation_id) DO UPDATE SET
                    status = 'active', activated_at = excluded.activated_at,
                    retired_at = NULL, failed_at = NULL",
                rusqlite::params![&project_id, &generation_id, now],
            )?;
            if current != generation_id {
                tx.tx.execute(
                    "INSERT INTO embedding_generation_project (
                        project_id, generation_id, status, created_at, retired_at
                     ) VALUES (?1, ?2, 'retired', ?3, ?3)
                     ON CONFLICT(project_id, generation_id) DO UPDATE SET
                        status = 'retired', retired_at = excluded.retired_at",
                    rusqlite::params![&project_id, &current, now],
                )?;
            }
            tx.tx.execute(
                "UPDATE embedding_generation_project SET status = 'retired', retired_at = ?3 \
                 WHERE project_id = ?1 AND generation_id = ?2 AND status = 'active' \
                   AND generation_id <> ?4",
                rusqlite::params![&project_id, &current, now, &generation_id],
            )?;
            tx.tx.execute(
                "UPDATE embedding_generation SET status = 'active', \
                    activated_at = COALESCE(activated_at, ?2) \
                 WHERE generation_id = ?1 AND status = 'building'",
                rusqlite::params![&generation_id, now],
            )?;
            tx.tx.execute(
                "INSERT INTO embedding_index_state (
                    project_id, active_generation_id, previous_generation_id,
                    rollback_until, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(project_id) DO UPDATE SET
                    active_generation_id = excluded.active_generation_id,
                    previous_generation_id = excluded.previous_generation_id,
                    rollback_until = excluded.rollback_until,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    &project_id,
                    &generation_id,
                    &current,
                    rollback_until_micros,
                    now,
                ],
            )?;
            Ok(EmbeddingIndexStateV1 {
                project_id: project_id.clone(),
                active_generation_id: generation_id.clone(),
                previous_generation_id: Some(current),
                rollback_until_micros: Some(rollback_until_micros),
                updated_at_micros: now,
            })
        })
        .await
    }

    /// Roll back to the previous generation while its window is open. The
    /// rolled-back generation is retired; old tables remain until cleanup.
    pub async fn rollback_embedding_generation(
        &self,
        project_id: &str,
    ) -> Result<EmbeddingRollbackOutputV1> {
        let project_id = project_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.with_transaction(move |tx| {
            let state = tx
                .tx
                .query_row(
                    "SELECT project_id, active_generation_id, previous_generation_id, \
                            rollback_until, updated_at \
                     FROM embedding_index_state WHERE project_id = ?1",
                    [&project_id],
                    |row| {
                        Ok(EmbeddingIndexStateV1 {
                            project_id: row.get(0)?,
                            active_generation_id: row.get(1)?,
                            previous_generation_id: row.get(2)?,
                            rollback_until_micros: row.get(3)?,
                            updated_at_micros: row.get(4)?,
                        })
                    },
                )
                .optional_inner()?
                .ok_or_else(|| anyhow!(EmbeddingValidationError::NoActiveGeneration.to_string()))?;
            let Some(previous) = state.previous_generation_id.clone() else {
                anyhow::bail!(EmbeddingValidationError::RollbackWindowClosed.to_string());
            };
            anyhow::ensure!(
                state
                    .rollback_until_micros
                    .is_some_and(|until| now <= until),
                "{}",
                EmbeddingValidationError::RollbackWindowClosed
            );
            tx.tx.execute(
                "UPDATE embedding_generation_project SET status = 'retired', retired_at = ?3 \
                 WHERE project_id = ?1 AND generation_id = ?2",
                rusqlite::params![&project_id, &state.active_generation_id, now],
            )?;
            tx.tx.execute(
                "UPDATE embedding_generation_project SET status = 'active', \
                    activated_at = ?3, retired_at = NULL, failed_at = NULL \
                 WHERE project_id = ?1 AND generation_id = ?2",
                rusqlite::params![&project_id, &previous, now],
            )?;
            tx.tx.execute(
                "UPDATE embedding_index_state SET active_generation_id = ?2, \
                    previous_generation_id = NULL, rollback_until = NULL, updated_at = ?3 \
                 WHERE project_id = ?1",
                rusqlite::params![&project_id, &previous, now],
            )?;
            Ok(EmbeddingRollbackOutputV1 {
                project_id: project_id.clone(),
                active_generation_id: previous,
                retired_generation_id: state.active_generation_id,
            })
        })
        .await
    }

    /// Remove target-generation rows that no longer exist in the source
    /// generation before activation, including their vec0 mirrors.
    pub async fn prune_embedding_generation(
        &self,
        project_id: &str,
        generation: &EmbeddingGenerationV1,
        keep: &BTreeSet<(Option<String>, String, String)>,
    ) -> Result<usize> {
        let table_name = generation.table_name.clone();
        if !safe_identifier(&table_name) {
            anyhow::bail!("unsafe embedding table name");
        }
        let project_id = project_id.to_string();
        let generation_id = generation.generation_id.clone();
        let keep = keep.clone();
        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                let rows: Vec<(String, Option<String>, String, String)> = {
                    let mut stmt = tx.prepare(
                        "SELECT id, branch_id, entity_table, entity_id FROM search_embedding \
                         WHERE project_id = ?1 AND generation_id = ?2",
                    )?;
                    stmt.query_map(rusqlite::params![&project_id, &generation_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                let mut removed = 0usize;
                for (id, branch_id, entity_table, entity_id) in rows {
                    if keep.contains(&(branch_id, entity_table, entity_id)) {
                        continue;
                    }
                    tx.execute(&format!("DELETE FROM {table_name} WHERE se_id = ?1"), [&id])?;
                    removed += tx.execute("DELETE FROM search_embedding WHERE id = ?1", [&id])?;
                }
                tx.commit()?;
                Ok(removed)
            })
            .await
    }

    pub(crate) async fn upsert_embedding_document_for_generation(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        entity_id: &str,
        document: &SearchDocument,
        generation: &EmbeddingGenerationV1,
        embedding: &[f64],
    ) -> Result<()> {
        validate_embedding_vector(embedding, generation.spec.dimension)?;
        let table_name = generation.table_name.clone();
        if !safe_identifier(&table_name) {
            anyhow::bail!("unsafe embedding table name");
        }
        let blob = pack_embedding(embedding);
        let project_id = project_id.to_string();
        let branch_id = branch_id.map(str::to_owned);
        let entity_id = entity_id.to_string();
        let entity_table = document.entity_table.clone();
        let title = document.title.clone();
        let excerpt = document.excerpt.clone();
        let content = document.content.clone();
        let generation_id = generation.generation_id.clone();
        let dimension = generation.spec.dimension;
        let version = generation.spec.version.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                let existing_id: Option<String> = tx
                    .query_row(
                        "SELECT id FROM search_embedding \
                         WHERE project_id = ?1 AND branch_id = ?2 AND entity_table = ?3 \
                           AND entity_id = ?4 AND generation_id = ?5",
                        rusqlite::params![
                            &project_id,
                            &branch_id,
                            &entity_table,
                            &entity_id,
                            &generation_id,
                        ],
                        |row| row.get(0),
                    )
                    .optional_inner()?;
                let id = existing_id.unwrap_or_else(|| mint_id("search_embedding"));
                tx.execute(
                    "INSERT INTO search_embedding (
                        id, project_id, branch_id, entity_table, entity_id, title,
                        excerpt, content, generation_id, embedding_version,
                        dimension, embedding, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                     ON CONFLICT(project_id, branch_id, entity_table, entity_id, generation_id)
                     DO UPDATE SET
                        title = excluded.title,
                        excerpt = excluded.excerpt,
                        content = excluded.content,
                        embedding_version = excluded.embedding_version,
                        dimension = excluded.dimension,
                        embedding = excluded.embedding,
                        updated_at = excluded.updated_at",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &entity_table,
                        &entity_id,
                        &title,
                        &excerpt,
                        &content,
                        &generation_id,
                        &version,
                        dimension as i64,
                        &blob,
                        now,
                    ],
                )?;
                tx.execute(&format!("DELETE FROM {table_name} WHERE se_id = ?1"), [&id])?;
                tx.execute(
                    &format!("INSERT INTO {table_name}(se_id, embedding) VALUES (?1, ?2)"),
                    rusqlite::params![&id, &blob],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    pub async fn list_embeddings_for_generation(
        &self,
        project_id: &str,
        generation_id: &str,
    ) -> Result<Vec<SearchEmbedding>> {
        let project_id = project_id.to_string();
        let generation_id = generation_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SEARCH_EMBEDDING_COLUMNS} FROM search_embedding \
                     WHERE project_id = ?1 AND generation_id = ?2 \
                     ORDER BY entity_table, entity_id"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &generation_id], |row| {
                        SearchEmbedding::try_from(row)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn embedding_row_count(
        &self,
        project_id: &str,
        generation_id: &str,
    ) -> Result<usize> {
        let project_id = project_id.to_string();
        let generation_id = generation_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM search_embedding \
                     WHERE project_id = ?1 AND generation_id = ?2",
                    rusqlite::params![&project_id, &generation_id],
                    |row| row.get(0),
                )?;
                Ok(count as usize)
            })
            .await
    }

    async fn delete_embedding_rows_where(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        entity_id: Option<&str>,
    ) -> Result<usize> {
        let generations = self.list_embedding_generations().await?;
        let table_by_generation: BTreeMap<String, String> = generations
            .into_iter()
            .map(|generation| (generation.generation_id, generation.table_name))
            .collect();
        let project_id = project_id.to_string();
        let branch_id = branch_id.map(str::to_owned);
        let entity_id = entity_id.map(str::to_owned);
        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                let mut sql = String::from(
                    "SELECT id, generation_id FROM search_embedding WHERE project_id = ?1",
                );
                if branch_id.is_some() {
                    sql.push_str(" AND branch_id = ?2");
                    if entity_id.is_some() {
                        sql.push_str(" AND entity_id = ?3");
                    }
                } else if entity_id.is_some() {
                    sql.push_str(" AND entity_id = ?2");
                }
                let mut stmt = tx.prepare(&sql)?;
                let rows = match (&branch_id, &entity_id) {
                    (Some(branch), Some(entity)) => stmt
                        .query_map(rusqlite::params![&project_id, branch, entity], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?,
                    (Some(branch), None) => stmt
                        .query_map(rusqlite::params![&project_id, branch], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?,
                    (None, Some(entity)) => stmt
                        .query_map(rusqlite::params![&project_id, entity], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?,
                    (None, None) => stmt
                        .query_map(rusqlite::params![&project_id], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?,
                };
                drop(stmt);
                for (id, generation_id) in &rows {
                    if let Some(table) = table_by_generation.get(generation_id)
                        && safe_identifier(table)
                    {
                        tx.execute(&format!("DELETE FROM {table} WHERE se_id = ?1"), [id])?;
                    }
                }
                let mut delete_sql =
                    String::from("DELETE FROM search_embedding WHERE project_id = ?1");
                if branch_id.is_some() {
                    delete_sql.push_str(" AND branch_id = ?2");
                    if entity_id.is_some() {
                        delete_sql.push_str(" AND entity_id = ?3");
                    }
                } else if entity_id.is_some() {
                    delete_sql.push_str(" AND entity_id = ?2");
                }
                let deleted = match (&branch_id, &entity_id) {
                    (Some(branch), Some(entity)) => {
                        tx.execute(&delete_sql, rusqlite::params![&project_id, branch, entity])?
                    }
                    (Some(branch), None) => {
                        tx.execute(&delete_sql, rusqlite::params![&project_id, branch])?
                    }
                    (None, Some(entity)) => {
                        tx.execute(&delete_sql, rusqlite::params![&project_id, entity])?
                    }
                    (None, None) => tx.execute(&delete_sql, rusqlite::params![&project_id])?,
                };
                tx.commit()?;
                Ok(deleted)
            })
            .await
    }

    pub async fn delete_embedding_rows_for_entity(
        &self,
        project_id: &str,
        entity_id: &str,
    ) -> Result<usize> {
        self.delete_embedding_rows_where(project_id, None, Some(entity_id))
            .await
    }

    pub async fn delete_embedding_rows_for_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<usize> {
        self.delete_embedding_rows_where(project_id, Some(branch_id), None)
            .await
    }

    pub async fn knn_search_embeddings_for_generation(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        generation: &EmbeddingGenerationV1,
        query: &EmbeddingQueryVectorV1,
        k: usize,
    ) -> Result<Vec<(SearchEmbedding, f64)>> {
        anyhow::ensure!(
            query.generation_id == generation.generation_id,
            "{}",
            EmbeddingValidationError::MixedGeneration {
                query_generation: query.generation_id.clone(),
                index_generation: generation.generation_id.clone(),
            }
        );
        validate_embedding_vector(&query.values, generation.spec.dimension)?;
        let table_name = generation.table_name.clone();
        if !safe_identifier(&table_name) {
            anyhow::bail!("unsafe embedding table name");
        }
        let project_id = project_id.to_string();
        let generation_id = generation.generation_id.clone();
        let branch_id = branch_id.map(str::to_owned);
        let query_blob = pack_embedding(&query.values);
        let k = k as i64;
        self.inner
            .pool
            .read(move |conn| {
                let qualified_cols = SEARCH_EMBEDDING_COLUMNS
                    .split(',')
                    .map(|column| format!("se.{}", column.trim()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut sql = format!(
                    "SELECT {qualified_cols}, vec.distance \
                     FROM {table_name} AS vec \
                     JOIN search_embedding AS se ON se.id = vec.se_id \
                     WHERE vec.embedding MATCH ?1 AND vec.k = ?2 \
                       AND se.project_id = ?3 AND se.generation_id = ?4"
                );
                if branch_id.is_some() {
                    sql.push_str(" AND se.branch_id = ?5");
                }
                sql.push_str(" ORDER BY vec.distance");
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = if let Some(branch_id) = &branch_id {
                    stmt.query_map(
                        rusqlite::params![&query_blob, k, &project_id, &generation_id, branch_id],
                        |row| Ok((SearchEmbedding::try_from(row)?, row.get::<_, f64>(13)?)),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                } else {
                    stmt.query_map(
                        rusqlite::params![&query_blob, k, &project_id, &generation_id],
                        |row| Ok((SearchEmbedding::try_from(row)?, row.get::<_, f64>(13)?)),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                Ok(rows)
            })
            .await
    }
}
