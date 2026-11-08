use super::*;

impl Repository {
    /// Cached raw model output for one deep-check tier over one prose version
    /// (migration V0039). `None` means the tier has not analyzed this scene at
    /// this revision — the caller must call the model.
    pub async fn get_deep_check_cache(
        &self,
        scene_id: &str,
        scene_revision_fingerprint: &str,
        check_type: &str,
    ) -> Result<Option<String>> {
        let scene_id = scene_id.to_string();
        let fingerprint = scene_revision_fingerprint.to_string();
        let check_type = check_type.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT output_text FROM deep_check_cache \
                     WHERE scene_id = ?1 AND scene_revision_fingerprint = ?2 AND check_type = ?3",
                )?;
                let mut rows = stmt.query(rusqlite::params![scene_id, fingerprint, check_type])?;
                match rows.next()? {
                    Some(row) => Ok(Some(row.get::<_, String>(0)?)),
                    None => Ok(None),
                }
            })
            .await
    }

    /// Bank one deep tier's raw model output so a retry does not re-pay for it.
    /// Also drops this scene's entries for OTHER fingerprints: once the prose
    /// has moved on, the superseded analysis is unreachable by key and would
    /// only accumulate.
    pub async fn put_deep_check_cache(
        &self,
        scene_id: &str,
        scene_revision_fingerprint: &str,
        check_type: &str,
        output_text: &str,
    ) -> Result<()> {
        let scene_id = scene_id.to_string();
        let fingerprint = scene_revision_fingerprint.to_string();
        let check_type = check_type.to_string();
        let output_text = output_text.to_string();
        let created = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO deep_check_cache \
                     (scene_id, scene_revision_fingerprint, check_type, output_text, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5) \
                     ON CONFLICT(scene_id, scene_revision_fingerprint, check_type) DO UPDATE SET \
                       output_text = excluded.output_text, \
                       created_at = excluded.created_at",
                    rusqlite::params![scene_id, fingerprint, check_type, output_text, created],
                )?;
                conn.execute(
                    "DELETE FROM deep_check_cache \
                     WHERE scene_id = ?1 AND scene_revision_fingerprint <> ?2",
                    rusqlite::params![scene_id, fingerprint],
                )?;
                Ok(())
            })
            .await
    }

    /// Mark every open validator_finding for a specific validator on a branch
    /// as resolved (sets resolved_at). Returns the number of rows updated.
    pub async fn resolve_validator_findings_for_validator(
        &self,
        project_id: &str,
        branch_id: &str,
        validator_id: &str,
    ) -> Result<usize> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let validator_id = validator_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE validator_finding SET resolved_at = ?1 \
                     WHERE project_id = ?2 AND branch_id = ?3 \
                       AND validator_id = ?4 AND resolved_at IS NULL",
                    rusqlite::params![now, &project_id, &branch_id, &validator_id],
                )
            })
            .await
    }

    /// Mark every open validator_finding tied to the given scenes as resolved.
    pub async fn resolve_validator_findings_for_scenes(
        &self,
        branch_id: &str,
        scene_ids: &[String],
    ) -> Result<usize> {
        if scene_ids.is_empty() {
            return Ok(0);
        }
        let branch_id = branch_id.to_string();
        let scene_ids: Vec<String> = scene_ids.to_vec();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let placeholders = (0..scene_ids.len())
                    .map(|i| format!("?{}", i + 3))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "UPDATE validator_finding SET resolved_at = ?1 \
                     WHERE branch_id = ?2 AND scene_id IN ({placeholders}) \
                       AND resolved_at IS NULL"
                );
                let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(scene_ids.len() + 2);
                params.push(&now);
                params.push(&branch_id);
                for id in &scene_ids {
                    params.push(id);
                }
                conn.execute(&sql, &params[..])
            })
            .await
    }

    pub async fn upsert_validator_finding(
        &self,
        params: UpsertValidatorFindingParams,
    ) -> Result<ValidatorFinding> {
        let id = mint_id("validator_finding");
        let id_out = id.clone();
        let byte_range_json = match params.byte_range.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let details_json = match params.details_json.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let UpsertValidatorFindingParams {
            project_id,
            branch_id,
            scene_id,
            scene_text_hash,
            context_hash,
            validator_id,
            finding_id,
            severity,
            message,
            ..
        } = params;

        self.inner
            .pool
            .write(move |conn| {
                // The SurrealDB version uses a non-unique index but treats
                // (branch, scene, validator, finding, hash) as the natural key.
                // We DELETE prior matching rows then INSERT for the same shape.
                let tx = conn.transaction()?;
                tx.execute(
                    "UPDATE validator_finding SET resolved_at = ?1 \
                     WHERE branch_id = ?2 AND scene_id = ?3 AND validator_id = ?4 \
                       AND finding_id = ?5 AND scene_text_hash = ?6 \
                       AND resolved_at IS NULL \
                       AND context_hash IS NOT ?7",
                    rusqlite::params![
                        now,
                        &branch_id,
                        &scene_id,
                        &validator_id,
                        &finding_id,
                        &scene_text_hash,
                        &context_hash,
                    ],
                )?;
                tx.execute(
                    "DELETE FROM validator_finding \
                     WHERE branch_id = ?1 AND scene_id = ?2 AND validator_id = ?3 \
                       AND finding_id = ?4 AND scene_text_hash = ?5 \
                       AND context_hash IS ?6",
                    rusqlite::params![
                        &branch_id,
                        &scene_id,
                        &validator_id,
                        &finding_id,
                        &scene_text_hash,
                        &context_hash,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO validator_finding (id, project_id, branch_id, scene_id, \
                     scene_text_hash, validator_id, finding_id, severity, message, byte_range, \
                     details_json, created_at, resolved_at, context_hash) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &scene_id,
                        &scene_text_hash,
                        &validator_id,
                        &finding_id,
                        &severity,
                        &message,
                        &byte_range_json,
                        &details_json,
                        now,
                        &context_hash,
                    ],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.get_validator_finding(&id_out).await
    }

    pub async fn get_validator_finding(&self, id: &str) -> Result<ValidatorFinding> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {VALIDATOR_FINDING_COLUMNS} FROM validator_finding WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ValidatorFinding::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("validator_finding not found"))
    }

    pub async fn list_open_validator_findings(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<ValidatorFinding>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {VALIDATOR_FINDING_COLUMNS} FROM validator_finding \
                     WHERE project_id = ?1 AND branch_id = ?2 AND resolved_at IS NULL \
                     ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        ValidatorFinding::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_validator_findings_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<ValidatorFinding>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {VALIDATOR_FINDING_COLUMNS} FROM validator_finding \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        ValidatorFinding::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Cache lookup for the Phase-4 fan-out: returns every active
    /// validator_finding row for the given branch+scene+text-hash whose
    /// `validator_id` is in `validator_ids`. The service layer treats a
    /// hit on every requested validator id as a cache hit and skips
    /// re-running the registry for that scene.
    pub async fn list_active_validator_findings_by_scene_hash(
        &self,
        branch_id: &str,
        scene_id: &str,
        scene_text_hash: &str,
        validator_ids: &[String],
    ) -> Result<Vec<ValidatorFinding>> {
        if validator_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_id = branch_id.to_string();
        let scene_id = scene_id.to_string();
        let scene_text_hash = scene_text_hash.to_string();
        let validator_ids: Vec<String> = validator_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..validator_ids.len())
                    .map(|i| format!("?{}", i + 4))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {VALIDATOR_FINDING_COLUMNS} FROM validator_finding \
                     WHERE branch_id = ?1 AND scene_id = ?2 AND scene_text_hash = ?3 \
                       AND resolved_at IS NULL AND validator_id IN ({placeholders}) \
                     ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut params: Vec<&dyn rusqlite::ToSql> =
                    Vec::with_capacity(3 + validator_ids.len());
                params.push(&branch_id);
                params.push(&scene_id);
                params.push(&scene_text_hash);
                for v in &validator_ids {
                    params.push(v);
                }
                let rows = stmt
                    .query_map(params.as_slice(), |r| ValidatorFinding::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
