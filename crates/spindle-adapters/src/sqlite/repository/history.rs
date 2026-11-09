use super::*;
use rusqlite::Row;

#[derive(Debug, Clone)]
pub struct HistoryUndoRecord {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub source_operation_id: String,
    pub compensating_operation_id: Option<String>,
    pub kind: String,
    pub summary: String,
    pub created_at: i64,
}

impl<'a> TryFrom<&Row<'a>> for HistoryUndoRecord {
    type Error = rusqlite::Error;

    fn try_from(row: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(row, 0)?,
            project_id: row::text(row, 1)?,
            branch_id: row::text(row, 2)?,
            source_operation_id: row::text(row, 3)?,
            compensating_operation_id: row::opt_text(row, 4)?,
            kind: row::text(row, 5)?,
            summary: row::text(row, 6)?,
            created_at: row::int(row, 7)?,
        })
    }
}

impl Repository {
    pub async fn append_history_undo(
        &self,
        project_id: &str,
        branch_id: &str,
        source_operation_id: &str,
        compensating_operation_id: Option<&str>,
        kind: &str,
        summary: &str,
    ) -> Result<String> {
        let id = format!("undo:{}", mint_id("undo"));
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let source_operation_id = source_operation_id.to_string();
        let compensating_operation_id = compensating_operation_id.map(str::to_string);
        let kind = kind.to_string();
        let summary = summary.to_string();
        let created_at = timestamp_to_micros(chrono::Utc::now());
        let id_for_return = id.clone();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO history_undo (id, project_id, branch_id, source_operation_id, \
                     compensating_operation_id, kind, summary, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &source_operation_id,
                        &compensating_operation_id,
                        &kind,
                        &summary,
                        created_at,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(id_for_return)
    }

    pub async fn list_history_undo_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<HistoryUndoRecord>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT id, project_id, branch_id, source_operation_id, \
                     compensating_operation_id, kind, summary, created_at \
                     FROM history_undo WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY created_at DESC",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |row| {
                        HistoryUndoRecord::try_from(row)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
