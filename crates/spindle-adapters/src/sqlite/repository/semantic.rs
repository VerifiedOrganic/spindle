use super::*;

impl Repository {
    pub async fn get_semantic_continuity_cache(
        &self,
        key: &spindle_core::semantic_continuity::SemanticCacheKeyV1,
    ) -> Result<Option<String>> {
        let key = key.clone();
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT report_json FROM semantic_continuity_cache \
                     WHERE scene_id = ?1 AND scene_sha256 = ?2 AND canon_version = ?3 \
                       AND candidate_set_hash = ?4 AND contract_version = ?5 \
                       AND route = ?6 AND model_identity = ?7",
                )?;
                let mut rows = stmt.query(rusqlite::params![
                    key.scene_id,
                    key.scene_sha256,
                    key.canon_version,
                    key.candidate_set_hash,
                    key.contract_version,
                    key.route,
                    key.model_identity,
                ])?;
                match rows.next()? {
                    Some(row) => Ok(Some(row.get::<_, String>(0)?)),
                    None => Ok(None),
                }
            })
            .await
    }

    pub async fn put_semantic_continuity_cache(
        &self,
        key: &spindle_core::semantic_continuity::SemanticCacheKeyV1,
        report_json: &str,
    ) -> Result<()> {
        let key = key.clone();
        let report_json = report_json.to_string();
        let created = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO semantic_continuity_cache \
                     (scene_id, scene_sha256, canon_version, candidate_set_hash, \
                      contract_version, route, model_identity, report_json, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
                     ON CONFLICT(scene_id, scene_sha256, canon_version, candidate_set_hash, \
                                  contract_version, route, model_identity) \
                     DO UPDATE SET report_json = excluded.report_json, \
                                   created_at = excluded.created_at",
                    rusqlite::params![
                        key.scene_id,
                        key.scene_sha256,
                        key.canon_version,
                        key.candidate_set_hash,
                        key.contract_version,
                        key.route,
                        key.model_identity,
                        report_json,
                        created,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn delete_semantic_continuity_cache_entry(
        &self,
        key: &spindle_core::semantic_continuity::SemanticCacheKeyV1,
    ) -> Result<()> {
        let key = key.clone();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM semantic_continuity_cache \
                     WHERE scene_id = ?1 AND scene_sha256 = ?2 AND canon_version = ?3 \
                       AND candidate_set_hash = ?4 AND contract_version = ?5 \
                       AND route = ?6 AND model_identity = ?7",
                    rusqlite::params![
                        key.scene_id,
                        key.scene_sha256,
                        key.canon_version,
                        key.candidate_set_hash,
                        key.contract_version,
                        key.route,
                        key.model_identity,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Remove superseded reports for one scene, preserving any rows still
    /// matching the current prose hash. Used after a scene edit.
    pub async fn sweep_semantic_continuity_cache(
        &self,
        scene_id: &str,
        current_scene_sha256: &str,
    ) -> Result<usize> {
        let scene_id = scene_id.to_string();
        let current = current_scene_sha256.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let rows = conn.execute(
                    "DELETE FROM semantic_continuity_cache \
                     WHERE scene_id = ?1 AND scene_sha256 <> ?2",
                    rusqlite::params![scene_id, current],
                )?;
                Ok(rows)
            })
            .await
    }
}
