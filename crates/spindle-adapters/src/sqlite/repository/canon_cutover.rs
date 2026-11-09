//! Per-project canon cutover state and readiness gating (P2 PR E).

use super::canon_events::enum_to_sql_str;
use super::*;
use spindle_core::canon::*;

pub(crate) fn canon_cutover_mode_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
) -> Result<CanonCutoverModeV1> {
    let mode: Option<String> = conn
        .query_row(
            "SELECT mode FROM canon_cutover_state WHERE project_id = ?1",
            [project_id],
            |row| row.get(0),
        )
        .optional_inner()?;
    Ok(match mode.as_deref() {
        Some("dual_write") => CanonCutoverModeV1::DualWrite,
        Some("shadow_read") => CanonCutoverModeV1::ShadowRead,
        Some("canon_read") => CanonCutoverModeV1::CanonRead,
        _ => CanonCutoverModeV1::Legacy,
    })
}

fn state_from_row(
    project_id: String,
    mode: String,
    activated_decision_watermark: i64,
    reason: String,
    updated_at: i64,
) -> CanonCutoverStateV1 {
    CanonCutoverStateV1 {
        project_id,
        mode: match mode.as_str() {
            "dual_write" => CanonCutoverModeV1::DualWrite,
            "shadow_read" => CanonCutoverModeV1::ShadowRead,
            "canon_read" => CanonCutoverModeV1::CanonRead,
            _ => CanonCutoverModeV1::Legacy,
        },
        activated_decision_watermark,
        reason,
        updated_at_micros: updated_at,
    }
}

impl Repository {
    /// Record the accepted scene revision as a source-map row so evidence-bound
    /// claims can be verified against the exact bytes. This is metadata only:
    /// it creates no claim, event or decision. Returns `false` when cutover has
    /// not started or the scene is empty.
    pub async fn record_canon_scene_revision(
        &self,
        project_id: &str,
        branch_id: &str,
        scene_id: &str,
        revision_id: &str,
        text: &str,
    ) -> Result<bool> {
        if text.trim().is_empty() || !self.canon_cutover_mode(project_id).await?.mirrors_writes() {
            return Ok(false);
        }
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let scene_id = scene_id.to_string();
        let revision_id = revision_id.to_string();
        let raw_snapshot = serde_json::json!({
            "scene_id": scene_id,
            "revision_id": revision_id,
            "accepted_text_sha256": exact_text_sha256(text),
            "byte_length": text.len(),
        })
        .to_string();
        self.with_transaction(move |tx| {
            let now_micros = timestamp_to_micros(chrono::Utc::now());
            tx.tx.execute(
                "INSERT INTO canon_source_map (
                    project_id, branch_id, source_table, legacy_primary_key,
                    migration_version, status, provenance, claim_ids, event_ids,
                    decision_id, quarantine_reason, raw_snapshot, created_at, updated_at
                 ) VALUES (?1, ?2, 'scene_revision', ?3, 1, 'mapped', 'legacy_unverified',
                    '[]', '[]', NULL, NULL, ?4, ?5, ?5)
                 ON CONFLICT(project_id, source_table, legacy_primary_key, migration_version)
                 DO UPDATE SET raw_snapshot = excluded.raw_snapshot, updated_at = excluded.updated_at",
                rusqlite::params![
                    &project_id,
                    &branch_id,
                    &revision_id,
                    &raw_snapshot,
                    now_micros,
                ],
            )?;
            Ok(true)
        })
        .await
    }

    pub async fn canon_cutover_mode(&self, project_id: &str) -> Result<CanonCutoverModeV1> {
        Ok(self
            .get_canon_cutover_state(project_id)
            .await?
            .map(|state| state.mode)
            .unwrap_or(CanonCutoverModeV1::Legacy))
    }

    pub async fn get_canon_cutover_state(
        &self,
        project_id: &str,
    ) -> Result<Option<CanonCutoverStateV1>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                conn.query_row(
                    "SELECT project_id, mode, activated_decision_watermark, reason, updated_at \
                     FROM canon_cutover_state WHERE project_id = ?1",
                    [&project_id],
                    |row| {
                        Ok(state_from_row(
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional_inner()
            })
            .await
    }

    /// Transition a project's cutover mode. Forward transitions require the
    /// compatibility parity/quarantine readiness gate; rollback never does.
    pub async fn set_canon_cutover_mode(
        &self,
        project_id: &str,
        requested: CanonCutoverModeV1,
        reason: &str,
    ) -> Result<CanonCutoverStateV1> {
        let readiness = self
            .evaluate_project_canon_cutover_readiness(project_id)
            .await?;
        let project_id = project_id.to_string();
        let reason = reason.to_string();
        self.with_transaction(move |tx| {
            let now_micros = timestamp_to_micros(chrono::Utc::now());
            let current = tx
                .tx
                .query_row(
                    "SELECT project_id, mode, activated_decision_watermark, reason, updated_at \
                     FROM canon_cutover_state WHERE project_id = ?1",
                    [&project_id],
                    |row| {
                        Ok(state_from_row(
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional_inner()?;
            let decision_watermark: i64 = tx.tx.query_row(
                "SELECT COALESCE(MAX(decision_seq), 0) FROM canon_decision WHERE project_id = ?1",
                [&project_id],
                |row| row.get(0),
            )?;
            let state = evaluate_canon_cutover_transition(
                current.as_ref(),
                requested,
                &readiness,
                decision_watermark,
                now_micros,
            )
            .map_err(|error| anyhow!(error.to_string()))?;
            let mode_str = enum_to_sql_str(state.mode)?;
            tx.tx.execute(
                "INSERT INTO canon_cutover_state (
                    project_id, mode, activated_decision_watermark, reason, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(project_id) DO UPDATE SET
                    mode = excluded.mode,
                    activated_decision_watermark = excluded.activated_decision_watermark,
                    reason = excluded.reason,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    &state.project_id,
                    &mode_str,
                    state.activated_decision_watermark,
                    &reason,
                    state.updated_at_micros,
                ],
            )?;
            Ok(state)
        })
        .await
    }
}
