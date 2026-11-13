use super::canon_backfill::mark_legacy_rows_mapped_in_conn;
use super::canon_policy::{legacy_claim, legacy_snapshot_claim};
use super::*;
use spindle_core::canon::*;

impl Repository {
    /// All relates_to rows involving any of the given characters (either
    /// in_id or out_id), resolved along the branch parent chain.
    pub async fn list_relationships_for_characters(
        &self,
        branch_id: &str,
        character_ids: &[String],
    ) -> Result<Vec<RelatesTo>> {
        if character_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_id = branch_id.to_string();
        let character_ids: Vec<String> = character_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, None, &branch_id)?;
                let placeholders = (0..character_ids.len())
                    .map(|i| format!("?{}", i + 2))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {RELATES_TO_COLUMNS} FROM relates_to \
                     WHERE branch_id = ?1 \
                       AND (in_id IN ({placeholders}) OR out_id IN ({placeholders})) \
                     ORDER BY relationship_type"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let mut params: Vec<&dyn rusqlite::ToSql> =
                        Vec::with_capacity(character_ids.len() + 1);
                    params.push(candidate_branch_id);
                    for id in &character_ids {
                        params.push(id);
                    }
                    let rows = stmt
                        .query_map(&params[..], |r| RelatesTo::try_from(r))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for row in rows {
                        let key = relationship_pair_key(&row.in_id, &row.out_id);
                        if seen.insert(key) {
                            out.push(row);
                        }
                    }
                }
                Ok(out)
            })
            .await
    }

    // =========================================================================
    // Relationships (relates_to edge table)
    // =========================================================================
    //
    // Public API break vs SurrealDB repository: the schema-side relates_to
    // table has no surrogate id — it's keyed by composite PK
    // (branch_id, in_id, out_id). So update_relationship takes the character
    // pair + branch instead of a relationship_id RecordId.

    /// Create a relationship (or no-op-update if it already exists in either
    /// direction). Matches the SurrealDB create_relationship semantics:
    /// look up by (a→b) OR (b→a); if found, update its type/trust/tension;
    /// else insert a→b.
    pub async fn create_relationship(
        &self,
        branch_id: &str,
        input: &CreateRelationshipInput,
    ) -> Result<RelatesTo> {
        let branch_id_owned = branch_id.to_string();
        let a = input.character_a_id.clone();
        let b = input.character_b_id.clone();
        let rel_type = input.relationship_type.clone();
        let trust = input.initial_trust;
        let tension = input.initial_tension;
        let dynamics_json =
            serde_json::to_string(&input.dynamics).context("serializing dynamics")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        let (in_id, out_id) = self
            .inner
            .pool
            .write({
                let branch_id_owned = branch_id_owned.clone();
                let a = a.clone();
                let b = b.clone();
                let rel_type = rel_type.clone();
                let dynamics_json = dynamics_json.clone();
                move |conn| {
                    let tx = conn.savepoint()?;
                    // Look for an existing row in either direction on this branch.
                    let mut find = tx.prepare_cached(
                        "SELECT in_id, out_id FROM relates_to \
                         WHERE branch_id = ?1 \
                           AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                         LIMIT 1",
                    )?;
                    let mut rows = find.query(rusqlite::params![&branch_id_owned, &a, &b])?;
                    let existing: Option<(String, String)> = if let Some(row) = rows.next()? {
                        Some((row.get(0)?, row.get(1)?))
                    } else {
                        None
                    };
                    drop(rows);
                    drop(find);

                    let (in_id, out_id) = if let Some((existing_in, existing_out)) = existing {
                        tx.execute(
                            "UPDATE relates_to SET relationship_type = ?1, trust = ?2, \
                             tension = ?3, dynamics = ?4, updated_at = ?5 \
                             WHERE branch_id = ?6 AND in_id = ?7 AND out_id = ?8",
                            rusqlite::params![
                                &rel_type,
                                trust,
                                tension,
                                &dynamics_json,
                                now,
                                &branch_id_owned,
                                &existing_in,
                                &existing_out,
                            ],
                        )?;
                        tx.execute(
                            "DELETE FROM relationship_scene_delta \
                             WHERE branch_id = ?1 AND in_id = ?2 AND out_id = ?3",
                            rusqlite::params![&branch_id_owned, &existing_in, &existing_out],
                        )?;
                        (existing_in, existing_out)
                    } else {
                        tx.execute(
                            "INSERT INTO relates_to (in_id, out_id, branch_id, \
                             relationship_type, trust, tension, dynamics, reason, last_scene_id, \
                             updated_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8)",
                            rusqlite::params![
                                &a,
                                &b,
                                &branch_id_owned,
                                &rel_type,
                                trust,
                                tension,
                                &dynamics_json,
                                now,
                            ],
                        )?;
                        tx.execute(
                            "DELETE FROM relationship_scene_delta \
                             WHERE branch_id = ?1 \
                               AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2))",
                            rusqlite::params![&branch_id_owned, &a, &b],
                        )?;
                        (a.clone(), b.clone())
                    };
                    tx.commit()?;
                    Ok((in_id, out_id))
                }
            })
            .await?;

        self.get_relationship(branch_id, &in_id, &out_id).await
    }

    /// Apply delta updates to a relationship's trust + tension with per-(scene, source)
    /// idempotency so recommitting a revised scene, retrying after failure, or
    /// applying a mined `relationship_shift` on top of a drafter continuity package
    /// replaces the scene's contribution rather than double-counting.
    pub async fn update_relationship(
        &self,
        branch_id: &str,
        input: &UpdateRelationshipInput,
    ) -> Result<RelatesTo> {
        self.update_relationship_with_source(branch_id, input, "scene")
            .await
    }

    pub async fn update_relationship_with_source(
        &self,
        branch_id: &str,
        input: &UpdateRelationshipInput,
        source: &str,
    ) -> Result<RelatesTo> {
        let branch_id_owned = branch_id.to_string();
        let input_owned = input.clone();
        let source_owned = source.to_string();
        self.with_transaction(move |tx| {
            let scene = input_owned
                .scene_id
                .as_deref()
                .and_then(|scene_id| tx.get_scene(scene_id).ok());
            let effective_branch_id = if let Some(scene) = &scene {
                tx.resolve_effective_scene_branch(&scene.project_id, &branch_id_owned)?
            } else {
                branch_id_owned.clone()
            };
            let has_details =
                input_owned.relationship_type.is_some() || input_owned.dynamics.is_some();
            let has_delta =
                input_owned.trust_delta.is_some() || input_owned.tension_delta.is_some();
            let relationship = if scene.is_some() {
                let details_row = if has_details || !has_delta {
                    Some(tx.update_relationship_details(&effective_branch_id, &input_owned)?)
                } else {
                    None
                };
                if has_delta {
                    tx.update_relationship(&effective_branch_id, &input_owned, &source_owned)?
                } else {
                    details_row.expect("details row fetched when no scene delta is supplied")
                }
            } else {
                tx.update_relationship_details(&effective_branch_id, &input_owned)?
            };
            if let Some(scene) = &scene {
                let revision_id = format!(
                    "legacy:relationship:{}:{}:{}:{}",
                    scene.id, source_owned, input_owned.character_a_id, input_owned.character_b_id
                );
                let placement = StoryPlacement {
                    book_number: scene.book_number,
                    chapter_number: scene.chapter_number,
                    scene_order: Some(scene.scene_order),
                    note: None,
                };
                let snapshot_claim = legacy_snapshot_claim(
                    &scene.project_id,
                    &effective_branch_id,
                    &revision_id,
                    Some(&scene.id),
                    &placement,
                    &relationship.in_id,
                    format!(
                        "legacy.relates_to.{}|{}",
                        relationship.in_id, relationship.out_id
                    ),
                    &relationship,
                    Some(0),
                    0,
                )?;
                // Legacy treats (A, B) and (B, A) as one pair and returns its
                // persisted orientation; canon relationship streams are
                // directional, so mirror that orientation rather than the
                // caller's argument order.
                let mut claims = vec![snapshot_claim];
                if has_delta {
                    claims.push(legacy_claim(
                        &scene.project_id,
                        &effective_branch_id,
                        &revision_id,
                        Some(&scene.id),
                        &placement,
                        ClaimEntity::Resolved {
                            id: relationship.in_id.clone(),
                        },
                        ClaimAssertion::Relationship {
                            operation: RelationshipOperation::Delta,
                            other: ClaimEntity::Resolved {
                                id: relationship.out_id.clone(),
                            },
                            trust: input_owned.trust_delta.unwrap_or(0),
                            tension: input_owned.tension_delta.unwrap_or(0),
                        },
                        Some(0),
                        1,
                    ));
                }
                let mirrored = tx.mirror_canon_claims(
                    &scene.project_id,
                    &effective_branch_id,
                    &revision_id,
                    Some(&scene.id),
                    claims,
                )?;
                if mirrored.is_some() {
                    let pair_key = format!(
                        "{}|{}|{}",
                        relationship.branch_id, relationship.in_id, relationship.out_id
                    );
                    let mut keys = vec![("relates_to".to_string(), pair_key)];
                    if has_delta {
                        let delta_key = format!(
                            "{}|{}|{}|{}|{}",
                            relationship.branch_id,
                            relationship.in_id,
                            relationship.out_id,
                            scene.id,
                            source_owned
                        );
                        keys.push(("relationship_scene_delta".to_string(), delta_key));
                    }
                    mark_legacy_rows_mapped_in_conn(
                        &tx.tx,
                        &scene.project_id,
                        &effective_branch_id,
                        &keys,
                        timestamp_to_micros(chrono::Utc::now()),
                    )?;
                }
            } else {
                let project_id = tx.get_character(&relationship.in_id)?.project_id;
                let revision_id = format!(
                    "legacy:relationship-details:{}:{}:{}:{}",
                    effective_branch_id,
                    relationship.in_id,
                    relationship.out_id,
                    timestamp_to_micros(chrono::Utc::now())
                );
                // A canon-only edit has no manuscript position; 0/0 maps to an
                // unscoped canon event rather than inventing scene zero.
                let placement = StoryPlacement {
                    book_number: 0,
                    chapter_number: 0,
                    scene_order: None,
                    note: Some("outside scene".to_string()),
                };
                let snapshot_claim = legacy_snapshot_claim(
                    &project_id,
                    &effective_branch_id,
                    &revision_id,
                    None,
                    &placement,
                    &relationship.in_id,
                    format!(
                        "legacy.relates_to.{}|{}",
                        relationship.in_id, relationship.out_id
                    ),
                    &relationship,
                    None,
                    0,
                )?;
                let mirrored = tx.mirror_canon_claims(
                    &project_id,
                    &effective_branch_id,
                    &revision_id,
                    None,
                    vec![snapshot_claim],
                )?;
                if mirrored.is_some() {
                    let pair_key = format!(
                        "{}|{}|{}",
                        relationship.branch_id, relationship.in_id, relationship.out_id
                    );
                    mark_legacy_rows_mapped_in_conn(
                        &tx.tx,
                        &project_id,
                        &effective_branch_id,
                        &[("relates_to".to_string(), pair_key)],
                        timestamp_to_micros(chrono::Utc::now()),
                    )?;
                }
            }
            Ok(relationship)
        })
        .await
    }

    /// Write absolute `trust` / `tension` values onto an existing
    /// relationship row (matching either orientation of the character
    /// pair). Distinct from `update_relationship`, which applies deltas.
    /// Used by `import_hydrate_bible` because import snapshots carry
    /// canonical absolutes from the source manuscript, not deltas.
    #[allow(clippy::too_many_arguments)]
    pub async fn set_relationship_absolute(
        &self,
        branch_id: &str,
        character_a_id: &str,
        character_b_id: &str,
        trust: i32,
        tension: i32,
        reason: Option<String>,
        last_scene_id: Option<String>,
    ) -> Result<RelatesTo> {
        let branch_id_owned = branch_id.to_string();
        let a = character_a_id.to_string();
        let b = character_b_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        let (in_id, out_id) = self
            .inner
            .pool
            .write({
                let branch_id_owned = branch_id_owned.clone();
                let a = a.clone();
                let b = b.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    let (in_id, out_id): (String, String) = tx.query_row(
                        "SELECT in_id, out_id FROM relates_to \
                             WHERE branch_id = ?1 \
                               AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                             LIMIT 1",
                        rusqlite::params![&branch_id_owned, &a, &b],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?;
                    tx.execute(
                        "UPDATE relates_to SET \
                            trust = ?1, \
                            tension = ?2, \
                            reason = COALESCE(?3, reason), \
                            last_scene_id = COALESCE(?4, last_scene_id), \
                            updated_at = ?5 \
                         WHERE branch_id = ?6 AND in_id = ?7 AND out_id = ?8",
                        rusqlite::params![
                            trust,
                            tension,
                            &reason,
                            &last_scene_id,
                            now,
                            &branch_id_owned,
                            &in_id,
                            &out_id,
                        ],
                    )?;
                    tx.execute(
                        "DELETE FROM relationship_scene_delta \
                         WHERE branch_id = ?1 AND in_id = ?2 AND out_id = ?3",
                        rusqlite::params![&branch_id_owned, &in_id, &out_id],
                    )?;
                    tx.commit()?;
                    Ok((in_id, out_id))
                }
            })
            .await?;
        self.get_relationship(branch_id, &in_id, &out_id).await
    }

    /// Fetch a single relationship by composite PK (resolving along the branch
    /// parent chain if not overridden locally).
    pub async fn get_relationship(
        &self,
        branch_id: &str,
        in_id: &str,
        out_id: &str,
    ) -> Result<RelatesTo> {
        let branch_id = branch_id.to_string();
        let in_id = in_id.to_string();
        let out_id = out_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, None, &branch_id)?;
                let sql = format!(
                    "SELECT {RELATES_TO_COLUMNS} FROM relates_to \
                     WHERE branch_id = ?1 \
                       AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                     LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                for candidate_branch_id in &chain {
                    if let Some(rel) = stmt
                        .query_row(
                            rusqlite::params![candidate_branch_id, &in_id, &out_id],
                            |r| RelatesTo::try_from(r),
                        )
                        .optional_inner()?
                    {
                        return Ok(Some(rel));
                    }
                }
                Ok(None)
            })
            .await?
            .ok_or_else(|| anyhow!("relationship not found"))
    }

    pub async fn list_relationships_by_branch(&self, branch_id: &str) -> Result<Vec<RelatesTo>> {
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, None, &branch_id)?;
                let sql = format!(
                    "SELECT {RELATES_TO_COLUMNS} FROM relates_to \
                     WHERE branch_id = ?1 ORDER BY relationship_type"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = stmt
                        .query_map([candidate_branch_id], |r| RelatesTo::try_from(r))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for row in rows {
                        let key = relationship_pair_key(&row.in_id, &row.out_id);
                        if seen.insert(key) {
                            out.push(row);
                        }
                    }
                }
                Ok(out)
            })
            .await
    }

    pub async fn list_all_relationships_by_branch_ids(
        &self,
        branch_ids: &[String],
    ) -> Result<Vec<RelatesTo>> {
        if branch_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_ids: Vec<String> = branch_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..branch_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {RELATES_TO_COLUMNS} FROM relates_to \
                     WHERE branch_id IN ({placeholders}) \
                     ORDER BY branch_id, relationship_type"
                );
                let params: Vec<&dyn rusqlite::ToSql> = branch_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        RelatesTo::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
