use super::*;

impl Repository {
    // =========================================================================
    // Generic JSON-row dumps (used by export_bible / save_point snapshots)
    //
    // Walks an arbitrary table via `SELECT *` and converts each row to a JSON
    // object keyed by column name. Values follow rusqlite's `ValueRef`
    // taxonomy:
    //
    //   NULL    -> Value::Null
    //   INTEGER -> Value::Number (i64)
    //   REAL    -> Value::Number (f64; non-finite becomes Value::Null)
    //   TEXT    -> Value::String, with a heuristic to re-parse JSON-shaped
    //              text (objects "{...}" / arrays "[...]") when the column
    //              name suggests a JSON payload (see `looks_like_json_column`).
    //   BLOB    -> base64-encoded Value::String prefixed with "base64:".
    //
    // This deliberately does not piggy-back on the typed record TryFrom impls
    // because (a) ~59 record types would need `#[derive(Serialize)]` to mirror
    // the SurrealDB JSON shape, and (b) the SurrealDB reference reaches for
    // raw `db().select() / db().query()` which is itself shapeless — so the
    // export format is documented as "schema-by-name JSON" either way.
    // =========================================================================

    /// Dump every row of `table` where `project_id = ?` as a list of JSON
    /// objects. Columns are emitted under their schema names. Used by
    /// `export_bible` and save-point snapshots; the table name is validated
    /// against a fixed allowlist by the caller (`build_project_export_payload`),
    /// which is the only safe entry point for this primitive.
    pub async fn dump_project_table(&self, table: &str, project_id: &str) -> Result<Vec<Value>> {
        ensure_safe_export_identifier(table)?;
        let project_id = project_id.to_string();
        let sql = format!("SELECT * FROM {table} WHERE project_id = ?1");
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let column_names: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();
                let rows = stmt
                    .query_map([&project_id], move |row| {
                        Ok(row_to_json_value(row, &column_names))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.into_iter().collect::<rusqlite::Result<Vec<Value>>>()
            })
            .await
    }

    /// Dump every row of `table` filtered by an arbitrary single-column WHERE
    /// `{column} = ?` clause. Used for join-table exports (e.g. `relates_to`
    /// keyed by branch_id) where the row itself has no `project_id` column.
    pub async fn dump_table_by_column(
        &self,
        table: &str,
        column: &str,
        value: &str,
    ) -> Result<Vec<Value>> {
        ensure_safe_export_identifier(table)?;
        ensure_safe_export_identifier(column)?;
        let value = value.to_string();
        let sql = format!("SELECT * FROM {table} WHERE {column} = ?1");
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let column_names: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();
                let rows = stmt
                    .query_map([&value], move |row| {
                        Ok(row_to_json_value(row, &column_names))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.into_iter().collect::<rusqlite::Result<Vec<Value>>>()
            })
            .await
    }

    /// Dump every row of `table` where `{column} IN (...)`. Used to follow
    /// FK chains during export (e.g. `import_source_document` keyed by a set
    /// of `session_id` values gathered from `import_session`).
    pub async fn dump_table_by_column_in(
        &self,
        table: &str,
        column: &str,
        values: &[String],
    ) -> Result<Vec<Value>> {
        ensure_safe_export_identifier(table)?;
        ensure_safe_export_identifier(column)?;
        if values.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (0..values.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT * FROM {table} WHERE {column} IN ({placeholders})");
        let owned: Vec<String> = values.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let column_names: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();
                let params: Vec<&dyn rusqlite::ToSql> =
                    owned.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), move |row| {
                        Ok(row_to_json_value(row, &column_names))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.into_iter().collect::<rusqlite::Result<Vec<Value>>>()
            })
            .await
    }
}
