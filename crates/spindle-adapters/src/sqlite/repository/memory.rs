//! SQLite persistence for layered derived memory, dependency edges, and
//! durable lazy-recompute jobs (P2 PR G).

use super::*;
use spindle_core::memory::*;

fn enum_str<T: serde::Serialize>(value: T) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("expected string-serialized enum variant"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecomputeJobRow {
    pub job_id: String,
    pub summary_id: String,
    pub reason: String,
    pub attempts: i64,
}

fn load_layered_memory_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
) -> Result<Vec<MemorySummaryV1>> {
    let mut stmt = conn.prepare(
        "SELECT data FROM memory_summary \
         WHERE project_id = ?1 AND branch_id = ?2 \
         ORDER BY book_number, chapter_number, scene_order, id",
    )?;
    let rows = stmt.query_map(rusqlite::params![project_id, branch_id], |row| {
        row.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(serde_json::from_str(&row?)?);
    }
    Ok(out)
}

fn load_layered_memory_dependencies_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
) -> Result<Vec<MemoryDependencyEdgeV1>> {
    let mut stmt = conn.prepare(
        "SELECT d.summary_id, d.depends_on_summary_id, d.source_kind \
         FROM memory_summary_dependency d \
         JOIN memory_summary s ON s.id = d.summary_id \
         WHERE s.project_id = ?1 AND s.branch_id = ?2 \
         ORDER BY d.summary_id, d.depends_on_summary_id",
    )?;
    rows_to_dependency_edges(&mut stmt, project_id, branch_id)
}

fn rows_to_dependency_edges(
    stmt: &mut rusqlite::Statement<'_>,
    project_id: &str,
    branch_id: &str,
) -> Result<Vec<MemoryDependencyEdgeV1>> {
    let rows = stmt.query_map(rusqlite::params![project_id, branch_id], |row| {
        Ok(MemoryDependencyEdgeV1 {
            summary_id: row.get(0)?,
            depends_on_summary_id: row.get(1)?,
            source_kind: row.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(crate) fn invalidate_layered_memory_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
    plan: &MemoryInvalidationPlanV1,
    now: i64,
) -> Result<()> {
    if plan.stale_summary_ids.is_empty() {
        return Ok(());
    }
    for summary_id in &plan.stale_summary_ids {
        let data: Option<String> = conn
            .query_row(
                "SELECT data FROM memory_summary WHERE id = ?1",
                [summary_id],
                |row| row.get(0),
            )
            .optional_inner()?;
        if let Some(data) = data {
            let mut summary: MemorySummaryV1 = serde_json::from_str(&data)?;
            summary.status = MemorySummaryStatusV1::Stale {
                reason: plan
                    .jobs
                    .iter()
                    .find(|job| &job.summary_id == summary_id)
                    .map(|job| job.reason.clone())
                    .unwrap_or_else(|| "source invalidation".into()),
            };
            summary.updated_at_micros = now;
            conn.execute(
                "UPDATE memory_summary SET status = 'stale', updated_at = ?2, data = ?3 \
                 WHERE id = ?1",
                rusqlite::params![summary_id, now, serde_json::to_string(&summary)?],
            )?;
        }
    }
    for job in &plan.jobs {
        conn.execute(
            "INSERT INTO memory_recompute_job (
                id, project_id, branch_id, summary_id, reason, dedup_key,
                status, attempts, last_error, next_attempt, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, NULL, ?7, ?7, ?7)
             ON CONFLICT(dedup_key) DO UPDATE SET
                status = 'pending',
                reason = excluded.reason,
                next_attempt = excluded.next_attempt,
                updated_at = excluded.updated_at,
                last_error = NULL",
            rusqlite::params![
                mint_id_local("memory_job"),
                project_id,
                branch_id,
                &job.summary_id,
                &job.reason,
                &job.dedup_key,
                now,
            ],
        )?;
    }
    Ok(())
}

/// Shared scene-write boundary helper: mark the scene summary and every
/// transitive ancestor stale in the caller's transaction.
pub(crate) fn invalidate_layered_memory_for_scene_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
    branch_id: &str,
    scene_id: &str,
    reason: &str,
) -> Result<()> {
    let summaries = load_layered_memory_in_conn(conn, project_id, branch_id)?;
    if summaries.is_empty() {
        return Ok(());
    }
    let edges = load_layered_memory_dependencies_in_conn(conn, project_id, branch_id)?;
    let roots: Vec<String> = summaries
        .iter()
        .filter(|summary| {
            summary.layer == MemoryLayerV1::Scene
                && summary.scope_key == format!("scene:{scene_id}")
        })
        .map(|summary| summary.summary_id.clone())
        .collect();
    let plan = plan_memory_invalidation(&roots, &summaries, &edges, reason);
    invalidate_layered_memory_in_conn(
        conn,
        project_id,
        branch_id,
        &plan,
        timestamp_to_micros(chrono::Utc::now()),
    )
}

impl Repository {
    /// Replace the branch's layered summaries and dependency DAG in one
    /// transaction. Deterministic summary ids keep edges stable across rebuilds.
    pub async fn replace_layered_memory(
        &self,
        project_id: &str,
        branch_id: &str,
        summaries: Vec<MemorySummaryV1>,
        edges: Vec<MemoryDependencyEdgeV1>,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.with_transaction(move |tx| {
            for summary in &summaries {
                let status = match summary.status {
                    MemorySummaryStatusV1::Fresh => "fresh",
                    MemorySummaryStatusV1::Stale { .. } => "stale",
                    MemorySummaryStatusV1::Recomputing => "recomputing",
                };
                let layer = enum_str(summary.layer)?;
                let data = serde_json::to_string(summary)?;
                tx.tx.execute(
                    "INSERT INTO memory_summary (
                        id, project_id, branch_id, layer, scope_key,
                        book_number, chapter_number, scene_order,
                        covered_end_book, covered_end_chapter, covered_end_scene,
                        status, updated_at, data
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                     ON CONFLICT(project_id, branch_id, layer, scope_key) DO UPDATE SET
                        scope_key = excluded.scope_key,
                        book_number = excluded.book_number,
                        chapter_number = excluded.chapter_number,
                        scene_order = excluded.scene_order,
                        covered_end_book = excluded.covered_end_book,
                        covered_end_chapter = excluded.covered_end_chapter,
                        covered_end_scene = excluded.covered_end_scene,
                        status = excluded.status,
                        updated_at = excluded.updated_at,
                        data = excluded.data",
                    rusqlite::params![
                        &summary.summary_id,
                        &project_id,
                        &branch_id,
                        &layer,
                        &summary.scope_key,
                        summary.coverage.start.book_number,
                        summary.coverage.start.chapter_number,
                        summary.coverage.start.scene_order,
                        summary.coverage.end.book_number,
                        summary.coverage.end.chapter_number,
                        summary.coverage.end.scene_order,
                        &status,
                        now,
                        &data,
                    ],
                )?;
            }

            let keep_ids: Vec<String> = summaries
                .iter()
                .map(|summary| summary.summary_id.clone())
                .collect();
            if keep_ids.is_empty() {
                tx.tx.execute(
                    "DELETE FROM memory_summary WHERE project_id = ?1 AND branch_id = ?2",
                    rusqlite::params![&project_id, &branch_id],
                )?;
            } else {
                let placeholders = (1..=keep_ids.len())
                    .map(|index| format!("?{}", index + 2))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "DELETE FROM memory_summary \
                     WHERE project_id = ?1 AND branch_id = ?2 AND id NOT IN ({placeholders})"
                );
                let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(keep_ids.len() + 2);
                params.push(&project_id);
                params.push(&branch_id);
                for id in &keep_ids {
                    params.push(id);
                }
                tx.tx.execute(&sql, params.as_slice())?;
            }

            tx.tx.execute(
                "DELETE FROM memory_summary_dependency \
                 WHERE summary_id IN (
                    SELECT id FROM memory_summary WHERE project_id = ?1 AND branch_id = ?2
                 )",
                rusqlite::params![&project_id, &branch_id],
            )?;
            for edge in &edges {
                tx.tx.execute(
                    "INSERT OR IGNORE INTO memory_summary_dependency (
                        summary_id, depends_on_summary_id, source_kind
                     ) VALUES (?1, ?2, ?3)",
                    rusqlite::params![
                        &edge.summary_id,
                        &edge.depends_on_summary_id,
                        &edge.source_kind,
                    ],
                )?;
            }
            Ok(())
        })
        .await
    }

    pub async fn list_layered_memory(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<MemorySummaryV1>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT data FROM memory_summary \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY book_number, chapter_number, scene_order, id",
                )?;
                let rows = stmt.query_map(rusqlite::params![&project_id, &branch_id], |row| {
                    row.get::<_, String>(0)
                })?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(serde_json::from_str(&row?).map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn list_layered_memory_dependencies(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<MemoryDependencyEdgeV1>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT d.summary_id, d.depends_on_summary_id, d.source_kind \
                     FROM memory_summary_dependency d \
                     JOIN memory_summary s ON s.id = d.summary_id \
                     WHERE s.project_id = ?1 AND s.branch_id = ?2 \
                     ORDER BY d.summary_id, d.depends_on_summary_id",
                )?;
                let rows = stmt.query_map(rusqlite::params![&project_id, &branch_id], |row| {
                    Ok(MemoryDependencyEdgeV1 {
                        summary_id: row.get(0)?,
                        depends_on_summary_id: row.get(1)?,
                        source_kind: row.get(2)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Mark every planned summary stale and enqueue deduplicated jobs. Recompute
    /// is lazy; callers can show stale status until they run the rebuild.
    pub async fn invalidate_layered_memory(
        &self,
        project_id: &str,
        branch_id: &str,
        plan: &MemoryInvalidationPlanV1,
    ) -> Result<()> {
        if plan.stale_summary_ids.is_empty() {
            return Ok(());
        }
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let plan = plan.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.with_transaction(move |tx| {
            invalidate_layered_memory_in_conn(&tx.tx, &project_id, &branch_id, &plan, now)
        })
        .await
    }

    pub async fn claim_memory_recompute_jobs(
        &self,
        project_id: &str,
        branch_id: &str,
        limit: usize,
    ) -> Result<Vec<MemoryRecomputeJobRow>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.with_transaction(move |tx| {
            let mut stmt = tx.tx.prepare(
                "SELECT id, summary_id, reason, attempts FROM memory_recompute_job \
                 WHERE project_id = ?1 AND branch_id = ?2 AND status = 'pending' \
                   AND next_attempt <= ?3 \
                 ORDER BY next_attempt, created_at, id LIMIT ?4",
            )?;
            let rows = stmt
                .query_map(
                    rusqlite::params![&project_id, &branch_id, now, limit as i64],
                    |row| {
                        Ok(MemoryRecomputeJobRow {
                            job_id: row.get(0)?,
                            summary_id: row.get(1)?,
                            reason: row.get(2)?,
                            attempts: row.get(3)?,
                        })
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for row in &rows {
                tx.tx.execute(
                    "UPDATE memory_recompute_job \
                     SET status = 'claimed', attempts = attempts + 1, updated_at = ?2 \
                     WHERE id = ?1",
                    rusqlite::params![&row.job_id, now],
                )?;
            }
            Ok(rows)
        })
        .await
    }

    pub async fn complete_all_memory_recompute_jobs(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<usize> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE memory_recompute_job SET status = 'done', updated_at = ?3 \
                     WHERE project_id = ?1 AND branch_id = ?2 AND status IN ('pending', 'claimed')",
                    rusqlite::params![&project_id, &branch_id, now],
                )
            })
            .await
    }

    pub async fn has_pending_memory_recompute_jobs(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<bool> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let pending: bool = conn.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM memory_recompute_job
                        WHERE project_id = ?1 AND branch_id = ?2
                          AND status IN ('pending', 'claimed')
                     )",
                    rusqlite::params![&project_id, &branch_id],
                    |row| row.get(0),
                )?;
                Ok(pending)
            })
            .await
    }
}
