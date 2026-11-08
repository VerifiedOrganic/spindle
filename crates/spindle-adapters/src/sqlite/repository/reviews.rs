use super::*;

impl Repository {
    // =========================================================================
    // Reviews + validator findings + progression events + scene beat annotations
    // =========================================================================

    pub async fn upsert_dual_persona_review(
        &self,
        params: UpsertDualPersonaReviewParams,
    ) -> Result<DualPersonaReview> {
        let stored_rounds: Vec<StoredDualPersonaReviewRound> = params
            .review_rounds
            .iter()
            .cloned()
            .map(Into::into)
            .collect();
        let rounds_json = serde_json::to_string(&serde_json::json!({ "rounds": stored_rounds }))?;
        let id = mint_id("dual_persona_review");
        let now = timestamp_to_micros(chrono::Utc::now());
        let project_id = params.project_id.clone();
        let branch_id = params.branch_id.clone();
        let scene_id = params.scene_id.clone();
        let fingerprint = params.scene_revision_fingerprint;
        let rounds_completed = params.rounds_completed as i64;
        let status = params.status;

        self.inner
            .pool
            .write({
                let branch_id = branch_id.clone();
                let scene_id = scene_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    let sql = format!("SELECT {SCENE_COLUMNS} FROM scene WHERE id = ?1");
                    let scene = tx.query_row(&sql, rusqlite::params![&scene_id], |row| {
                        crate::sqlite::records::Scene::try_from(row)
                    })?;
                    // Persist only while this job still matches live prose.
                    // A different fingerprint on the row is not enough to
                    // decide: a new job MUST replace an old one after an
                    // edit, but a late finish from the old job must not
                    // replace the new one. The live scene is the tie-break.
                    if Repository::scene_revision_fingerprint(&scene) != fingerprint {
                        tx.commit()?;
                        return Ok(());
                    }
                    tx.execute(
                        "DELETE FROM dual_persona_review WHERE branch_id = ?1 AND scene_id = ?2",
                        rusqlite::params![&branch_id, &scene_id],
                    )?;
                    tx.execute(
                        "INSERT INTO dual_persona_review (id, project_id, branch_id, scene_id, \
                         scene_revision_fingerprint, rounds_completed, status, review_rounds, \
                         created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                        rusqlite::params![
                            &id,
                            &project_id,
                            &branch_id,
                            &scene_id,
                            &fingerprint,
                            rounds_completed,
                            &status,
                            &rounds_json,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_dual_persona_review(&branch_id, &scene_id)
            .await?
            .ok_or_else(|| anyhow!("dual_persona_review vanished after upsert"))
    }

    /// Stable fingerprint over the scene fields that drive review staleness.
    /// Must stay in lockstep with the service-layer copy used to key jobs.
    pub(crate) fn scene_revision_fingerprint(scene: &Scene) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(scene.summary.as_bytes());
        hasher.update(b"\n");
        hasher.update(scene.full_text.as_bytes());
        hasher.update(b"\n");
        hasher.update(scene.content_rating.as_bytes());
        hasher.update(b"\n");
        hasher.update(scene.tone.clone().unwrap_or_default().as_bytes());
        format!("{:x}", hasher.finalize())
    }

    pub async fn get_dual_persona_review(
        &self,
        branch_id: &str,
        scene_id: &str,
    ) -> Result<Option<DualPersonaReview>> {
        let branch_id = branch_id.to_string();
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {DUAL_PERSONA_REVIEW_COLUMNS} FROM dual_persona_review \
                     WHERE branch_id = ?1 AND scene_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&branch_id, &scene_id], |r| {
                    DualPersonaReview::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn list_dual_persona_reviews_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<DualPersonaReview>> {
        let branch_id = self.active_branch_id(project_id).await?;
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {DUAL_PERSONA_REVIEW_COLUMNS} FROM dual_persona_review \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY updated_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        DualPersonaReview::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_dual_persona_reviews(
        &self,
        project_id: &str,
    ) -> Result<Vec<DualPersonaReview>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {DUAL_PERSONA_REVIEW_COLUMNS} FROM dual_persona_review \
                     WHERE project_id = ?1 ORDER BY updated_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| DualPersonaReview::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
