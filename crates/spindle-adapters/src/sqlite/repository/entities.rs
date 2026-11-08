use super::*;

impl Repository {
    /// Re-home rows of a branch-scoped canon table onto `target_branch_id` by
    /// id (used by branch merges). `table` is a fixed internal constant, never
    /// user input. Re-homing keeps each row's other FKs (e.g. a fact's scene_id)
    /// valid because the referenced rows still exist; if the target branch has
    /// an ancestor row with the same natural key, it is replaced by the child's
    /// overriding row.
    pub async fn rehome_rows_to_branch(
        &self,
        table: &'static str,
        target_branch_id: &str,
        ids: &[String],
    ) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let sql = format!("UPDATE {table} SET branch_id = ?1 WHERE id = ?2");
        let target_branch_id = target_branch_id.to_string();
        let ids = ids.to_vec();
        let count = ids.len();
        let natural_key_col: Option<&'static str> = match table {
            "faction" | "religion" | "economy" | "plot_line" | "conflict" | "motif"
            | "system_overlay" => Some("normalized_name"),
            "term" => Some("normalized_term"),
            _ => None,
        };
        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                for id in &ids {
                    if let Some(col) = natural_key_col {
                        let select_sql =
                            format!("SELECT project_id, {col} FROM {table} WHERE id = ?1");
                        if let Some((project_id, key_val)) = tx
                            .query_row(&select_sql, [id], |r| {
                                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                            })
                            .optional_inner()?
                        {
                            let delete_sql = format!(
                                "DELETE FROM {table} \
                                 WHERE project_id = ?1 AND branch_id = ?2 AND {col} = ?3 AND id != ?4"
                            );
                            tx.execute(
                                &delete_sql,
                                rusqlite::params![&project_id, &target_branch_id, &key_val, id],
                            )?;
                        }
                    } else if table == "knowledge_fact"
                        && let Some((project_id, character_id, normalized_fact)) = tx
                            .query_row(
                                "SELECT project_id, character_id, normalized_fact \
                                 FROM knowledge_fact WHERE id = ?1",
                                [id],
                                |r| {
                                    Ok((
                                        r.get::<_, String>(0)?,
                                        r.get::<_, String>(1)?,
                                        r.get::<_, String>(2)?,
                                    ))
                                },
                            )
                            .optional_inner()?
                    {
                        tx.execute(
                            "DELETE FROM knowledge_fact \
                             WHERE project_id = ?1 AND branch_id = ?2 \
                               AND character_id = ?3 AND normalized_fact = ?4 AND id != ?5",
                            rusqlite::params![
                                &project_id,
                                &target_branch_id,
                                &character_id,
                                &normalized_fact,
                                id
                            ],
                        )?;
                    }
                    tx.execute(&sql, rusqlite::params![&target_branch_id, id])?;
                    if table == "canonical_fact" {
                        // Follow replacements made on intermediate branches too.
                        // Ancestors outside the merge target remain untouched.
                        tx.execute(
                            "WITH RECURSIVE predecessors(id) AS ( \
                               SELECT supersedes_fact_id FROM canonical_fact WHERE id = ?1 \
                               UNION \
                               SELECT f.supersedes_fact_id FROM canonical_fact f \
                               JOIN predecessors p ON f.id = p.id \
                             ) UPDATE canonical_fact SET superseded_by = ?1, updated_at = ?2 \
                               WHERE branch_id = ?3 AND id IN (SELECT id FROM predecessors) \
                                 AND id != ?1",
                            rusqlite::params![id, timestamp_to_micros(chrono::Utc::now()), &target_branch_id],
                        )?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        Ok(count)
    }

    // =========================================================================
    // Generic helpers: archive_entity, update_entity, get_entity
    // =========================================================================

    /// Set `archived_at` on a single row identified by `table:id`. The set of
    /// tables that have an `archived_at` column is fixed by the schema; this
    /// helper validates the table is in that allowlist before issuing the
    /// UPDATE so we get a clear error instead of a confusing "no such column"
    /// from SQLite.
    pub async fn archive_entity(&self, table: &str, entity_id: &str) -> Result<()> {
        if !table_has_archived_at(table) {
            anyhow::bail!("entity table '{table}' has no archived_at column");
        }
        let entity_id = entity_id.to_string();
        let table = table.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let sql = format!(
                    "UPDATE \"{table}\" SET archived_at = ?1, updated_at = ?1 WHERE id = ?2"
                );
                let n = conn.execute(&sql, rusqlite::params![now, &entity_id])?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Apply a map of field updates to a row identified by `table:id`. Each
    /// (column, value) pair is gated by the same `column_is_updatable`
    /// allowlist as `update_entity_field`; unknown columns return an error
    /// rather than silently being ignored. `updated_at` is set automatically
    /// and must not appear in the changes map (mirrors the SurrealDB version's
    /// `strip_updated_at` behavior).
    ///
    /// This is the SQLite counterpart to SurrealDB's MERGE-style `update_entity`.
    pub async fn update_entity_fields(
        &self,
        table: &str,
        entity_id: &str,
        changes: std::collections::BTreeMap<String, Value>,
    ) -> Result<()> {
        if changes.is_empty() {
            // No-op but still bump updated_at to match the SurrealDB semantics.
            return self.touch_entity_updated_at(table, entity_id).await;
        }
        for col in changes.keys() {
            if col == "updated_at" {
                anyhow::bail!("update_entity_fields refuses to set updated_at directly");
            }
            if !column_is_updatable(table, col) {
                anyhow::bail!("column '{col}' on '{table}' is not in the update allowlist");
            }
        }
        // Issue one statement per column. The allowlist lookup is small and
        // each `update_entity_field` already uses prepare_cached, so the cost
        // is dominated by the round-trips, which are negligible for the small
        // change maps callers actually pass.
        for (col, value) in changes {
            self.update_entity_field(table, entity_id, &col, value)
                .await?;
        }
        Ok(())
    }

    /// Touch a row's `updated_at` column to the current time, no other
    /// changes. Used as the empty-changes path of `update_entity_fields`.
    pub async fn touch_entity_updated_at(&self, table: &str, entity_id: &str) -> Result<()> {
        // Defensive: only run this against tables that have an updated_at
        // column (every project+branch-scoped table does in our schema).
        let table = table.to_string();
        let entity_id = entity_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let sql = format!("UPDATE \"{table}\" SET updated_at = ?1 WHERE id = ?2");
                conn.execute(&sql, rusqlite::params![now, &entity_id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Set a single column on a row identified by `table:id` to a JSON-encoded
    /// value. The schema column names allowed for update are checked against a
    /// per-table allowlist; this gives the same safety guarantee the
    /// SurrealDB version had via SCHEMAFULL while keeping the signature simple.
    pub async fn update_entity_field(
        &self,
        table: &str,
        entity_id: &str,
        field: &str,
        value: Value,
    ) -> Result<()> {
        if !column_is_updatable(table, field) {
            anyhow::bail!("column '{field}' on '{table}' is not in the update allowlist");
        }
        if table == "narrative_promise" && field == "status" {
            return self
                .update_promise_status(&spindle_core::models::UpdatePromiseStatusInput {
                    narrative_promise_id: entity_id.to_owned(),
                    status: value
                        .as_str()
                        .context("promise status must be a string")?
                        .to_owned(),
                    ..Default::default()
                })
                .await;
        }
        let table = table.to_string();
        let entity_id = entity_id.to_string();
        let field = field.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        let value_repr = value;
        self.inner
            .pool
            .write(move |conn| {
                let sql = format!(
                    "UPDATE \"{table}\" SET \"{field}\" = ?1, updated_at = ?2 WHERE id = ?3"
                );
                let n = match &value_repr {
                    Value::String(s) => {
                        conn.execute(&sql, rusqlite::params![s, now, &entity_id])?
                    }
                    Value::Null => conn.execute(
                        &sql,
                        rusqlite::params![rusqlite::types::Null, now, &entity_id],
                    )?,
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            conn.execute(&sql, rusqlite::params![i, now, &entity_id])?
                        } else if let Some(f) = n.as_f64() {
                            conn.execute(&sql, rusqlite::params![f, now, &entity_id])?
                        } else {
                            return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(
                                std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "unsupported numeric width for field update",
                                ),
                            )));
                        }
                    }
                    Value::Bool(b) => conn.execute(
                        &sql,
                        rusqlite::params![if *b { 1 } else { 0 }, now, &entity_id],
                    )?,
                    other => {
                        // Arrays/objects: persist as JSON TEXT.
                        let s = serde_json::to_string(other)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                        conn.execute(&sql, rusqlite::params![s, now, &entity_id])?
                    }
                };
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        Ok(())
    }
}
