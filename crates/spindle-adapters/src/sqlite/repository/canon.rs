use super::*;

impl Repository {
    // ── Canon deltas (ADR 0001 — canon mining & ratification) ────────────────

    /// Stage a proposed canon delta (ADR 0001 D2). Validates:
    ///   * `delta_class` ∈ `CANON_DELTA_CLASSES` (unknown classes are rejected —
    ///     forward-compat additions ship via a new constant entry, never a
    ///     free-form label);
    ///   * `evidence` non-empty (trimmed) and ≤300 **chars** (char-safe, so a
    ///     multibyte quote is measured correctly);
    ///   * `confidence` ∈ {high, medium, low}.
    ///
    /// Records the decision seam nothing here — apply-dispatch to write tools is
    /// the service layer's job (see [`Repository::decide_canon_delta`]).
    pub async fn stage_canon_delta(
        &self,
        params: StageCanonDeltaParams,
    ) -> Result<crate::sqlite::records::StoredCanonDelta> {
        let StageCanonDeltaParams {
            project_id,
            branch_id,
            scene_id,
            authoring_run_id,
            delta_class,
            target_id,
            payload,
            evidence,
            confidence,
        } = params;

        if !spindle_core::models::is_canon_delta_class(&delta_class) {
            return Err(anyhow!(
                "unknown canon delta class '{delta_class}' (not in CANON_DELTA_CLASSES)"
            ));
        }
        if evidence.trim().is_empty() {
            return Err(anyhow!(
                "canon delta evidence is mandatory — a delta with no quotable evidence is not stageable"
            ));
        }
        let evidence_chars = evidence.chars().count();
        if evidence_chars > 300 {
            return Err(anyhow!(
                "canon delta evidence must be ≤300 chars (got {evidence_chars})"
            ));
        }
        if !matches!(confidence.as_str(), "high" | "medium" | "low") {
            return Err(anyhow!(
                "canon delta confidence must be one of high|medium|low (got '{confidence}')"
            ));
        }

        let payload_str =
            serde_json::to_string(&payload).context("serializing canon delta payload")?;
        let id = mint_id("canon_delta");
        let id_lookup = id.clone();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                conn.execute(
                    "INSERT INTO canon_delta \
                     (id, project_id, branch_id, scene_id, authoring_run_id, delta_class, \
                      target_id, payload, evidence, confidence, status, decided_at, decided_by, \
                      created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'staged', NULL, NULL, \
                             ?11, ?12)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &scene_id,
                        &authoring_run_id,
                        &delta_class,
                        &target_id,
                        &payload_str,
                        &evidence,
                        &confidence,
                        now,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.read_canon_delta(&id_lookup)
            .await?
            .ok_or_else(|| anyhow!("canon_delta vanished after insert"))
    }

    /// Fetch one canon delta by id, or `None` if absent. Public read used by the
    /// service-layer apply dispatcher to pre-flight each decision (row exists,
    /// ownership, staged status) before any write.
    pub async fn read_canon_delta_public(
        &self,
        id: &str,
    ) -> Result<Option<crate::sqlite::records::StoredCanonDelta>> {
        self.read_canon_delta(id).await
    }

    /// Fetch one canon delta by id, or `None` if absent.
    pub(super) async fn read_canon_delta(
        &self,
        id: &str,
    ) -> Result<Option<crate::sqlite::records::StoredCanonDelta>> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM canon_delta WHERE id = ?1",
                    crate::sqlite::records::CANON_DELTA_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| {
                    crate::sqlite::records::StoredCanonDelta::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    /// List canon deltas on a branch, optionally filtered by `status` and/or
    /// provenance `scene_id`. Deterministic order: `(created_at, id)`.
    pub async fn list_canon_deltas(
        &self,
        project_id: &str,
        branch_id: &str,
        status: Option<&str>,
        scene_id: Option<&str>,
    ) -> Result<Vec<crate::sqlite::records::StoredCanonDelta>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let status = status.map(str::to_string);
        let scene_id = scene_id.map(str::to_string);
        self.inner
            .pool
            .read(move |conn| {
                // Bind only the filters that are present. Params are pushed in
                // the same order the placeholders are appended.
                let mut clauses = String::from("project_id = ?1 AND branch_id = ?2");
                let mut binds: Vec<&dyn rusqlite::ToSql> = vec![&project_id, &branch_id];
                if let Some(status) = status.as_ref() {
                    clauses.push_str(&format!(" AND status = ?{}", binds.len() + 1));
                    binds.push(status);
                }
                if let Some(scene_id) = scene_id.as_ref() {
                    clauses.push_str(&format!(" AND scene_id = ?{}", binds.len() + 1));
                    binds.push(scene_id);
                }
                let sql = format!(
                    "SELECT {} FROM canon_delta WHERE {clauses} ORDER BY created_at, id",
                    crate::sqlite::records::CANON_DELTA_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(binds.as_slice(), |r| {
                        crate::sqlite::records::StoredCanonDelta::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Record the operator's ratification of a staged canon delta (ADR 0001
    /// D3). Errors if the delta is not currently `staged` — decisions are final
    /// history, never revised. When `edited_payload` is `Some`, it replaces the
    /// stored payload (ratify-with-correction). Stamps `decided_at`/`decided_by`
    /// and sets `status` to `applied`/`rejected`.
    ///
    /// This records the decision **only**. Apply-dispatch to the class's write
    /// tool is the service layer's job — the repository decides, the service
    /// applies.
    pub async fn decide_canon_delta(
        &self,
        id: &str,
        decision: CanonDeltaDecision,
        decided_by: &str,
        edited_payload: Option<serde_json::Value>,
    ) -> Result<crate::sqlite::records::StoredCanonDelta> {
        let id_owned = id.to_string();
        let id_lookup = id_owned.clone();
        let decided_by = decided_by.to_string();
        let new_status = decision.status();
        let edited_payload_str = match edited_payload {
            Some(value) => Some(
                serde_json::to_string(&value).context("serializing edited canon delta payload")?,
            ),
            None => None,
        };
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                // Guard: only a `staged` row may be decided. The status
                // predicate in the UPDATE makes the transition atomic; a zero
                // rowcount means the row was absent or already terminal.
                let affected = if let Some(payload_str) = edited_payload_str {
                    conn.execute(
                        "UPDATE canon_delta \
                         SET status = ?1, payload = ?2, decided_at = ?3, decided_by = ?4, \
                             updated_at = ?3 \
                         WHERE id = ?5 AND status = 'staged'",
                        rusqlite::params![new_status, payload_str, now, decided_by, id_owned],
                    )?
                } else {
                    conn.execute(
                        "UPDATE canon_delta \
                         SET status = ?1, decided_at = ?2, decided_by = ?3, updated_at = ?2 \
                         WHERE id = ?4 AND status = 'staged'",
                        rusqlite::params![new_status, now, decided_by, id_owned],
                    )?
                };
                Ok(affected)
            })
            .await
            .and_then(|affected| {
                if affected == 0 {
                    Err(anyhow!(
                        "canon delta '{id_lookup}' is not staged (already decided or absent — \
                         decisions are final)"
                    ))
                } else {
                    Ok(())
                }
            })?;
        self.read_canon_delta(&id_lookup)
            .await?
            .ok_or_else(|| anyhow!("canon_delta '{id_lookup}' vanished after decide"))
    }

    /// Supersede-on-remine (ADR 0001 D3): flip only this scene's `staged` canon
    /// deltas to `superseded`. `applied`/`rejected`/already-`superseded` rows are
    /// untouched — decisions are history. Returns the number of rows flipped.
    pub async fn supersede_scene_deltas(&self, scene_id: &str) -> Result<u64> {
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                let affected = conn.execute(
                    "UPDATE canon_delta SET status = 'superseded', updated_at = ?1 \
                     WHERE scene_id = ?2 AND status = 'staged'",
                    rusqlite::params![now, scene_id],
                )?;
                Ok(affected as u64)
            })
            .await
    }

    /// Active (not superseded) canonical facts on a branch whose value text,
    /// JSON value, or aliases mention `needle` (case-insensitive LIKE). Used
    /// by the character-rename report; facts that reference a character by id
    /// are unaffected by renames and deliberately not scanned.
    pub async fn list_canonical_fact_ids_mentioning(
        &self,
        project_id: &str,
        branch_id: &str,
        needle: &str,
    ) -> Result<Vec<String>> {
        let needle_lower = needle.to_lowercase();
        let facts = self
            .list_active_canonical_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        Ok(facts
            .into_iter()
            .filter(|f| {
                f.value_text
                    .as_deref()
                    .is_some_and(|v| v.to_lowercase().contains(&needle_lower))
                    || f.value_json
                        .as_ref()
                        .is_some_and(|v| v.to_string().to_lowercase().contains(&needle_lower))
                    || f.aliases
                        .iter()
                        .any(|a| a.to_lowercase().contains(&needle_lower))
            })
            .map(|f| f.id)
            .collect())
    }

    /// Knowledge facts on a branch whose text mentions `needle`
    /// (case-insensitive LIKE). Used by the character-rename report: secret
    /// circles and recorded knowledge embed character names in rendered fact
    /// prose that no index covers.
    pub async fn list_knowledge_fact_ids_mentioning(
        &self,
        project_id: &str,
        branch_id: &str,
        needle: &str,
    ) -> Result<Vec<String>> {
        let needle_lower = needle.to_lowercase();
        let facts = self
            .list_knowledge_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        Ok(facts
            .into_iter()
            .filter(|f| f.fact.to_lowercase().contains(&needle_lower))
            .map(|f| f.id)
            .collect())
    }

    // =========================================================================
    // Canonical fact (continuity tracking)
    // =========================================================================

    pub async fn create_canonical_fact(
        &self,
        params: CreateCanonicalFactParams,
    ) -> Result<CanonicalFact> {
        let id = mint_id("canonical_fact");
        let id_out = id.clone();
        let aliases_json = serde_json::to_string(&params.aliases)?;
        let value_json_str = match params.value_json.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let valid_from_stored: Option<StoredStoryPlacement> = params.valid_from.map(Into::into);
        let valid_from_json = match valid_from_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let valid_until_stored: Option<StoredStoryPlacement> = params.valid_until.map(Into::into);
        let valid_until_json = match valid_until_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let CreateCanonicalFactParams {
            project_id,
            branch_id,
            scene_id,
            book_number,
            chapter_number,
            subject_table,
            subject_id,
            predicate,
            value_kind,
            value_text,
            value_number,
            unit,
            scope,
            legacy_untyped,
            secret,
            concealment_note,
            ..
        } = params;
        let _ = legacy_untyped; // column dropped after v029; kept in params for caller parity.
        let secret_flag = if secret { 1 } else { 0 };

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO canonical_fact (id, project_id, branch_id, scene_id, \
                     source_scene_id, book_number, chapter_number, subject_table, subject_id, \
                     predicate, value_kind, value_number, value_text, value_json, unit, aliases, \
                     scope, valid_from, valid_until, superseded_by, created_at, updated_at, \
                     secret, concealment_note) \
                     VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                             ?15, ?16, ?17, ?18, NULL, ?19, ?19, ?20, ?21)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &scene_id,
                        book_number,
                        chapter_number,
                        &subject_table,
                        &subject_id,
                        &predicate,
                        &value_kind,
                        value_number,
                        &value_text,
                        &value_json_str,
                        &unit,
                        &aliases_json,
                        &scope,
                        &valid_from_json,
                        &valid_until_json,
                        now,
                        secret_flag,
                        &concealment_note,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_canonical_fact(&id_out).await
    }

    pub async fn get_canonical_fact(&self, id: &str) -> Result<CanonicalFact> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| CanonicalFact::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("canonical_fact not found"))
    }

    /// Attach a canonical fact to the scene that dramatises it (V0038).
    /// This is the "bind later" half of registering a planned-and-pending
    /// fact without a scene. The row must already exist; validation that the
    /// fact and scene share a project/branch and that the fact is live lives
    /// in the service layer.
    pub async fn bind_canonical_fact_to_scene(
        &self,
        canonical_fact_id: &str,
        scene_id: &str,
    ) -> Result<()> {
        let canonical_fact_id = canonical_fact_id.to_string();
        let scene_id = scene_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let n = conn.execute(
                    "UPDATE canonical_fact SET scene_id = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&scene_id, now, &canonical_fact_id],
                )?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Test-only seam: clear the `secret` flag on a canonical fact row. Used by
    /// the `decide_canon_deltas` mid-apply-failure test to make a
    /// `knowledge_learned` reveal's write fail *after* pre-flight validated the
    /// fact was secret — `record_knowledge` re-checks the flag at apply time and
    /// rejects a reveal linking to a now-non-secret fact. This proves the
    /// dispatcher stops honestly and does not silently roll back the earlier
    /// applies. A plain delete is unusable here: the circle row's
    /// `secret_of_fact_id` FK would block it. Not part of the production surface.
    #[cfg(test)]
    pub async fn clear_canonical_fact_secret_for_test(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute("UPDATE canonical_fact SET secret = 0 WHERE id = ?1", [&id])?;
                Ok(())
            })
            .await
    }

    /// Canonical facts about a specific subject (table+id), ordered by
    /// position in the manuscript. Used by continuity-check flows that
    /// need every claim about an entity in narrative order.
    pub async fn list_canonical_facts_by_subject(
        &self,
        project_id: &str,
        branch_id: &str,
        subject_table: &str,
        subject_id: Option<&str>,
    ) -> Result<Vec<CanonicalFact>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let subject_table = subject_table.to_string();
        let subject_id = subject_id.map(|s| s.to_string());
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let (sql, has_subject) = if subject_id.is_some() {
                    (
                        format!(
                            "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                             WHERE project_id = ?1 AND branch_id = ?2 \
                               AND subject_table = ?3 AND subject_id = ?4 \
                             ORDER BY book_number, chapter_number"
                        ),
                        true,
                    )
                } else {
                    (
                        format!(
                            "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                             WHERE project_id = ?1 AND branch_id = ?2 \
                               AND subject_table = ?3 \
                             ORDER BY book_number, chapter_number"
                        ),
                        false,
                    )
                };
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut overridden = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = if has_subject {
                        stmt.query_map(
                            rusqlite::params![
                                &project_id,
                                candidate_branch_id,
                                &subject_table,
                                subject_id.as_deref().unwrap()
                            ],
                            |r| CanonicalFact::try_from(r),
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                    } else {
                        stmt.query_map(
                            rusqlite::params![&project_id, candidate_branch_id, &subject_table],
                            |r| CanonicalFact::try_from(r),
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    let mut branch_keys = Vec::new();
                    for row in rows {
                        let key = (
                            row.subject_table.clone(),
                            row.subject_id.clone().unwrap_or_default(),
                            row.predicate.clone(),
                        );
                        if !overridden.contains(&key) {
                            branch_keys.push(key);
                            out.push(row);
                        }
                    }
                    overridden.extend(branch_keys);
                }
                out.sort_by(|a, b| {
                    (a.book_number, a.chapter_number, &a.created_at).cmp(&(
                        b.book_number,
                        b.chapter_number,
                        &b.created_at,
                    ))
                });
                Ok(out)
            })
            .await
    }

    /// All canonical facts for a project's active branch, ordered by
    /// position in the manuscript.
    /// Active canonical facts on the project's active branch — convenience
    /// wrapper around the branch-scoped variant.
    pub async fn list_active_canonical_facts_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<CanonicalFact>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_active_canonical_facts_by_project_and_branch(project_id, &branch_id)
            .await
    }

    /// Project-wide canonical facts (`subject_table = 'project'`) active on
    /// a specific branch, filtered to those whose position is at-or-before
    /// the supplied placement. Mirrors the SurrealDB reference of the same
    /// name in 705b835^ (repository.rs:4813..4833). Used by
    /// `get_scene_context` and `get_chapter_briefing` to surface global
    /// constraints alongside subject-scoped ones.
    pub async fn list_canonical_facts_for_project_wide(
        &self,
        project_id: &str,
        branch_id: &str,
        placement: &spindle_core::models::StoryPlacement,
    ) -> Result<Vec<CanonicalFact>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let book_number = placement.book_number;
        let chapter_number = placement.chapter_number;
        let scene_order = placement.scene_order.unwrap_or(0);
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND superseded_by IS NULL \
                       AND subject_table = 'project' \
                       AND book_number <= ?3 \
                       AND (book_number < ?3 OR chapter_number <= ?4) \
                     ORDER BY book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let placement = spindle_core::models::StoryPlacement {
                    book_number,
                    chapter_number,
                    scene_order: Some(scene_order),
                    note: None,
                };
                let mut overridden = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = stmt
                        .query_map(
                            rusqlite::params![
                                &project_id,
                                candidate_branch_id,
                                book_number,
                                chapter_number
                            ],
                            |r| CanonicalFact::try_from(r),
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    let mut branch_keys = Vec::new();
                    for row in rows {
                        if !fact_at_or_before(&row, &placement) {
                            continue;
                        }
                        let key = (
                            row.subject_table.clone(),
                            row.subject_id.clone().unwrap_or_default(),
                            row.predicate.clone(),
                        );
                        if !overridden.contains(&key) {
                            branch_keys.push(key);
                            out.push(row);
                        }
                    }
                    overridden.extend(branch_keys);
                }
                out.sort_by(|a, b| {
                    (a.book_number, a.chapter_number, &a.created_at).cmp(&(
                        b.book_number,
                        b.chapter_number,
                        &b.created_at,
                    ))
                });
                Ok(out)
            })
            .await
    }

    /// Canonical facts scoped to a list of subjects (per-table+id), active
    /// on a branch and at-or-before the supplied placement. Mirrors the
    /// SurrealDB reference (repository.rs:4748..4812 in 705b835^).
    pub async fn list_canonical_facts_for_subjects(
        &self,
        project_id: &str,
        branch_id: &str,
        subjects: &[spindle_core::subject::Subject],
        placement: &spindle_core::models::StoryPlacement,
    ) -> Result<Vec<CanonicalFact>> {
        // Deduplicate (table, id) pairs and drop project-wide subjects, which
        // are handled by the project-wide variant above.
        let mut seen = std::collections::BTreeSet::new();
        let mut pairs: Vec<(String, String)> = Vec::new();
        for subject in subjects {
            let Some(subject_id) = subject.id() else {
                continue;
            };
            let table = subject.table().as_str().to_string();
            let key = format!("{table}:{subject_id}");
            if seen.insert(key) {
                pairs.push((table, subject_id.to_string()));
            }
        }
        if pairs.is_empty() {
            return Ok(Vec::new());
        }

        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let book_number = placement.book_number;
        let chapter_number = placement.chapter_number;
        let scene_order = placement.scene_order.unwrap_or(0);
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let mut subject_clauses = Vec::with_capacity(pairs.len());
                for idx in 0..pairs.len() {
                    let table_ph = 5 + idx * 2;
                    let id_ph = table_ph + 1;
                    subject_clauses.push(format!(
                        "(subject_table = ?{table_ph} AND subject_id = ?{id_ph})"
                    ));
                }
                let where_subjects = subject_clauses.join(" OR ");
                let sql = format!(
                    "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND superseded_by IS NULL \
                       AND book_number <= ?3 \
                       AND (book_number < ?3 OR chapter_number <= ?4) \
                       AND ({where_subjects}) \
                     ORDER BY book_number, chapter_number"
                );

                let mut stmt = conn.prepare_cached(&sql)?;
                let placement = spindle_core::models::StoryPlacement {
                    book_number,
                    chapter_number,
                    scene_order: Some(scene_order),
                    note: None,
                };
                let mut overridden = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let mut params: Vec<Box<dyn rusqlite::ToSql>> =
                        Vec::with_capacity(4 + pairs.len() * 2);
                    params.push(Box::new(project_id.clone()));
                    params.push(Box::new(candidate_branch_id.clone()));
                    params.push(Box::new(book_number));
                    params.push(Box::new(chapter_number));
                    for (table, subject_id) in &pairs {
                        params.push(Box::new(table.clone()));
                        params.push(Box::new(subject_id.clone()));
                    }
                    let param_refs: Vec<&dyn rusqlite::ToSql> =
                        params.iter().map(|b| b.as_ref()).collect();

                    let rows = stmt
                        .query_map(&param_refs[..], |r| CanonicalFact::try_from(r))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    let mut branch_keys = Vec::new();
                    for row in rows {
                        if row.subject_table == "project" || !fact_at_or_before(&row, &placement) {
                            continue;
                        }
                        let key = (
                            row.subject_table.clone(),
                            row.subject_id.clone().unwrap_or_default(),
                            row.predicate.clone(),
                        );
                        if !overridden.contains(&key) {
                            branch_keys.push(key);
                            out.push(row);
                        }
                    }
                    overridden.extend(branch_keys);
                }
                out.sort_by(|a, b| {
                    (a.book_number, a.chapter_number, &a.created_at).cmp(&(
                        b.book_number,
                        b.chapter_number,
                        &b.created_at,
                    ))
                });
                Ok(out)
            })
            .await
    }

    pub async fn list_canonical_facts_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<CanonicalFact>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_canonical_facts_by_project_and_branch(project_id, &branch_id)
            .await
    }

    /// Active (non-superseded) canonical facts on a branch, ordered by
    /// position. Filters out rows whose `superseded_by` points at a still-
    /// valid newer fact, and inherits ancestor facts along `parent_branch_id`
    /// unless overridden on a nearer branch by `(subject_table, subject_id, predicate)`.
    pub async fn list_active_canonical_facts_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CanonicalFact>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                     WHERE project_id = ?1 AND branch_id = ?2 AND superseded_by IS NULL \
                     ORDER BY book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut overridden = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = stmt
                        .query_map(rusqlite::params![&project_id, candidate_branch_id], |r| {
                            CanonicalFact::try_from(r)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    let mut branch_keys = Vec::new();
                    for row in rows {
                        let key = (
                            row.subject_table.clone(),
                            row.subject_id.clone().unwrap_or_default(),
                            row.predicate.clone(),
                        );
                        if !overridden.contains(&key) {
                            branch_keys.push(key);
                            out.push(row);
                        }
                    }
                    overridden.extend(branch_keys);
                }
                out.sort_by(|a, b| {
                    (a.book_number, a.chapter_number, &a.created_at).cmp(&(
                        b.book_number,
                        b.chapter_number,
                        &b.created_at,
                    ))
                });
                Ok(out)
            })
            .await
    }

    pub async fn set_canonical_fact_legacy_untyped(
        &self,
        fact_id: &str,
        _legacy_untyped: bool,
    ) -> Result<()> {
        // legacy_untyped column was dropped in v029. The SurrealDB API
        // accepted a bool but it's a no-op against the post-v029 schema.
        // Kept for caller-parity through the migration; remove after
        // services migrate to a no-flag API.
        let _ = fact_id;
        Ok(())
    }

    pub async fn supersede_canonical_fact(
        &self,
        old_fact_id: &str,
        new_fact_id: &str,
    ) -> Result<()> {
        let old_fact_id = old_fact_id.to_string();
        let new_fact_id = new_fact_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE canonical_fact SET superseded_by = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&new_fact_id, now, &old_fact_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_canonical_facts_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CanonicalFact>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut overridden = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = stmt
                        .query_map(rusqlite::params![&project_id, candidate_branch_id], |r| {
                            CanonicalFact::try_from(r)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    let mut branch_keys = Vec::new();
                    for row in rows {
                        let key = (
                            row.subject_table.clone(),
                            row.subject_id.clone().unwrap_or_default(),
                            row.predicate.clone(),
                        );
                        if !overridden.contains(&key) {
                            branch_keys.push(key);
                            out.push(row);
                        }
                    }
                    overridden.extend(branch_keys);
                }
                out.sort_by(|a, b| {
                    (a.book_number, a.chapter_number, &a.created_at).cmp(&(
                        b.book_number,
                        b.chapter_number,
                        &b.created_at,
                    ))
                });
                Ok(out)
            })
            .await
    }

    // =========================================================================
    // Knowledge facts + knows edges
    // =========================================================================

    pub async fn upsert_knowledge_fact(
        &self,
        params: UpsertKnowledgeFactParams,
    ) -> Result<KnowledgeFact> {
        let id = mint_id("knowledge_fact");
        let id_out = id.clone();
        let learned_at_stored: Option<StoredStoryPlacement> = params.learned_at.map(Into::into);
        let learned_at_json = match learned_at_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let tags_json = serde_json::to_string(&params.tags)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        let reader_visible = if params.reader_visible { 1 } else { 0 };
        let UpsertKnowledgeFactParams {
            project_id,
            branch_id,
            character_id,
            fact,
            normalized_fact,
            source_summary,
            confidence,
            source_import_session_id,
            secret_of_fact_id,
            ..
        } = params;

        self.inner
            .pool
            .write({
                let project_id = project_id.clone();
                let branch_id = branch_id.clone();
                let character_id = character_id.clone();
                let normalized_fact = normalized_fact.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM knowledge_fact \
                         WHERE project_id = ?1 AND branch_id = ?2 AND character_id = ?3 \
                           AND normalized_fact = ?4",
                        rusqlite::params![&project_id, &branch_id, &character_id, &normalized_fact],
                    )?;
                    tx.execute(
                        "INSERT INTO knowledge_fact (id, project_id, branch_id, character_id, \
                         fact, normalized_fact, source_summary, learned_at, confidence, tags, \
                         reader_visible, source_import_session_id, created_at, updated_at, \
                         secret_of_fact_id) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, ?14)",
                        rusqlite::params![
                            &id,
                            &project_id,
                            &branch_id,
                            &character_id,
                            &fact,
                            &normalized_fact,
                            &source_summary,
                            &learned_at_json,
                            confidence,
                            &tags_json,
                            reader_visible,
                            &source_import_session_id,
                            now,
                            &secret_of_fact_id,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_knowledge_fact(&id_out).await
    }

    /// Secret-knowledge gating (design §2.1, declaration path): link a single
    /// holder into a secret fact's circle of trust.
    ///
    /// Upsert semantics chosen for the holder-row collision the design calls
    /// out: the unique index on `(project, branch, character, normalized_fact)`
    /// means a holder may already carry a knowledge row for the same rendered
    /// fact text. Rather than DELETE+re-INSERT (which would clobber a
    /// pre-existing `learned_at`, `tags`, or `reader_visible`), when a matching
    /// row exists we *UPDATE only its `secret_of_fact_id`* (and touch
    /// `updated_at`), preserving every other field. When no row exists we
    /// INSERT a fresh one with `learned_at = None` (known from the start) and
    /// `reader_visible = true` (dramatic-irony default per design §2.5).
    ///
    /// Returns the linked knowledge_fact.
    pub async fn link_secret_holder(
        &self,
        params: LinkSecretHolderParams,
    ) -> Result<KnowledgeFact> {
        let LinkSecretHolderParams {
            project_id,
            branch_id,
            character_id,
            fact_text,
            normalized_fact,
            source_summary,
            secret_of_fact_id,
        } = params;
        let now = timestamp_to_micros(chrono::Utc::now());
        let tags_json = serde_json::to_string::<Vec<String>>(&Vec::new())?;

        let linked_id = self
            .inner
            .pool
            .write(move |conn| {
                let existing_id: Option<String> = conn
                    .query_row(
                        "SELECT id FROM knowledge_fact \
                         WHERE project_id = ?1 AND branch_id = ?2 AND character_id = ?3 \
                           AND normalized_fact = ?4",
                        rusqlite::params![&project_id, &branch_id, &character_id, &normalized_fact],
                        |r| r.get::<_, String>(0),
                    )
                    .optional_inner()?;

                match existing_id {
                    Some(id) => {
                        // Collision: link the existing row without disturbing
                        // its learned_at / tags / reader_visible.
                        conn.execute(
                            "UPDATE knowledge_fact \
                             SET secret_of_fact_id = ?1, updated_at = ?2 WHERE id = ?3",
                            rusqlite::params![&secret_of_fact_id, now, &id],
                        )?;
                        Ok(id)
                    }
                    None => {
                        let id = mint_id_local("knowledge_fact");
                        conn.execute(
                            "INSERT INTO knowledge_fact (id, project_id, branch_id, character_id, \
                             fact, normalized_fact, source_summary, learned_at, confidence, tags, \
                             reader_visible, source_import_session_id, created_at, updated_at, \
                             secret_of_fact_id) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, 1, NULL, ?9, ?9, \
                                     ?10)",
                            rusqlite::params![
                                &id,
                                &project_id,
                                &branch_id,
                                &character_id,
                                &fact_text,
                                &normalized_fact,
                                &source_summary,
                                &tags_json,
                                now,
                                &secret_of_fact_id,
                            ],
                        )?;
                        Ok(id)
                    }
                }
            })
            .await?;
        self.get_knowledge_fact(&linked_id).await
    }

    /// Secret-knowledge gating (design §2.1): the derived circle of trust for a
    /// secret fact — every character with a knowledge_fact row linked to it,
    /// paired with the story index at which they entered the circle
    /// (`learned_at`; `None` = known from the start / always in the circle).
    /// Feeds the pure `resolve_secret_visibility` resolver.
    pub async fn secret_circle_members(
        &self,
        project_id: &str,
        branch_id: &str,
        fact_id: &str,
    ) -> Result<Vec<(String, Option<StoredStoryPlacement>)>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let fact_id = fact_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let mut stmt = conn.prepare_cached(
                    "SELECT character_id, learned_at FROM knowledge_fact \
                     WHERE project_id = ?1 AND branch_id = ?2 AND secret_of_fact_id = ?3",
                )?;
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = stmt
                        .query_map(
                            rusqlite::params![&project_id, candidate_branch_id, &fact_id],
                            |r| {
                                let character_id: String = r.get(0)?;
                                let learned_at: Option<StoredStoryPlacement> =
                                    crate::sqlite::row::opt_json(r, 1)?;
                                Ok((character_id, learned_at))
                            },
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for (character_id, learned_at) in rows {
                        if seen.insert(character_id.clone()) {
                            out.push((character_id, learned_at));
                        }
                    }
                }
                Ok(out)
            })
            .await
    }

    pub async fn get_knowledge_fact(&self, id: &str) -> Result<KnowledgeFact> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {KNOWLEDGE_FACT_COLUMNS} FROM knowledge_fact WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| KnowledgeFact::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("knowledge_fact not found"))
    }

    pub async fn upsert_knows(&self, params: UpsertKnowsParams) -> Result<Knows> {
        let learned_at_stored: Option<StoredStoryPlacement> = params.learned_at.map(Into::into);
        let learned_at_json = match learned_at_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let reader_visible = if params.reader_visible { 1 } else { 0 };
        let UpsertKnowsParams {
            project_id,
            branch_id,
            character_id,
            knowledge_fact_id,
            source_summary,
            confidence,
            source_import_session_id,
            ..
        } = params;

        self.inner
            .pool
            .write({
                let project_id = project_id.clone();
                let branch_id = branch_id.clone();
                let character_id = character_id.clone();
                let knowledge_fact_id = knowledge_fact_id.clone();
                move |conn| {
                    conn.execute(
                        "INSERT INTO knows (in_id, out_id, project_id, branch_id, source_summary, \
                         learned_at, confidence, reader_visible, source_import_session_id, \
                         created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10) \
                         ON CONFLICT (branch_id, in_id, out_id) DO UPDATE SET \
                             source_summary = excluded.source_summary, \
                             learned_at = excluded.learned_at, \
                             confidence = excluded.confidence, \
                             reader_visible = excluded.reader_visible, \
                             source_import_session_id = excluded.source_import_session_id, \
                             updated_at = excluded.updated_at",
                        rusqlite::params![
                            &character_id,
                            &knowledge_fact_id,
                            &project_id,
                            &branch_id,
                            &source_summary,
                            &learned_at_json,
                            confidence,
                            reader_visible,
                            &source_import_session_id,
                            now,
                        ],
                    )?;
                    Ok(())
                }
            })
            .await?;
        self.get_knows(&branch_id, &character_id, &knowledge_fact_id)
            .await
    }

    pub async fn get_knows(
        &self,
        branch_id: &str,
        character_id: &str,
        knowledge_fact_id: &str,
    ) -> Result<Knows> {
        let branch_id = branch_id.to_string();
        let character_id = character_id.to_string();
        let knowledge_fact_id = knowledge_fact_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {KNOWS_COLUMNS} FROM knows \
                     WHERE branch_id = ?1 AND in_id = ?2 AND out_id = ?3"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![&branch_id, &character_id, &knowledge_fact_id],
                    |r| Knows::try_from(r),
                )
                .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("knows edge not found"))
    }

    /// List every knowledge_fact row attached to a project, across
    /// branches. Mirrors the existing `_by_project_and_branch` variant; the
    /// import-hydration pass uses the broader view when seeding canon.
    pub async fn list_knowledge_facts_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<KnowledgeFact>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {KNOWLEDGE_FACT_COLUMNS} FROM knowledge_fact \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| KnowledgeFact::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Future knowledge
    // =========================================================================

    pub async fn create_future_knowledge(
        &self,
        input: &CreateFutureKnowledgeInput,
    ) -> Result<FutureKnowledge> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("future_knowledge");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let character_id = input.character_id.clone();
        let summary = input.knowledge_summary.clone();
        let source = input.source.clone();
        let learned_at: StoredStoryPlacement = input.learned_at.clone().into();
        let learned_at_json = serde_json::to_string(&learned_at)?;
        let expires_at_stored: Option<StoredStoryPlacement> =
            input.expires_at.clone().map(Into::into);
        let expires_json = match expires_at_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let notes_json = serde_json::to_string(&input.notes)?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO future_knowledge (id, project_id, branch_id, character_id, \
                     knowledge_summary, source, learned_at, expires_at, notes, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &character_id, &summary, &source,
                        &learned_at_json, &expires_json, &notes_json, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_future_knowledge(&id_out).await
    }

    pub async fn get_future_knowledge(&self, id: &str) -> Result<FutureKnowledge> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {FUTURE_KNOWLEDGE_COLUMNS} FROM future_knowledge WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| FutureKnowledge::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("future_knowledge not found"))
    }

    pub async fn list_future_knowledge_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<FutureKnowledge>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_future_knowledge_by_project_and_branch(project_id, &branch_id)
            .await
    }

    /// Explicit-branch variant for the Phase-4 retcon scanner, which resolves
    /// along the branch parent chain up to `main`.
    pub async fn list_future_knowledge_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<FutureKnowledge>> {
        list_branch_scoped(
            &self.inner.pool,
            FUTURE_KNOWLEDGE_COLUMNS,
            "future_knowledge",
            "character_id, created_at",
            project_id,
            branch_id,
            |r| FutureKnowledge::try_from(r),
            |f| (f.character_id.clone(), f.knowledge_summary.clone()),
        )
        .await
    }

    /// List every knowledge_fact row on a branch (inheriting from ancestor
    /// branches along `parent_branch_id`), ordered by created_at.
    pub async fn list_knowledge_facts_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<KnowledgeFact>> {
        let mut facts = list_branch_scoped(
            &self.inner.pool,
            KNOWLEDGE_FACT_COLUMNS,
            "knowledge_fact",
            "created_at",
            project_id,
            branch_id,
            |r| KnowledgeFact::try_from(r),
            |f| (f.character_id.clone(), f.normalized_fact.clone()),
        )
        .await?;
        facts.sort_by_key(|a| a.created_at);
        Ok(facts)
    }

    pub async fn list_all_future_knowledge(
        &self,
        project_id: &str,
    ) -> Result<Vec<FutureKnowledge>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {FUTURE_KNOWLEDGE_COLUMNS} FROM future_knowledge \
                     WHERE project_id = ?1 ORDER BY character_id, created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| FutureKnowledge::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_canonical_facts(&self, project_id: &str) -> Result<Vec<CanonicalFact>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                     WHERE project_id = ?1 \
                     ORDER BY branch_id, book_number, chapter_number, created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| CanonicalFact::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_knows(&self, project_id: &str) -> Result<Vec<Knows>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {KNOWS_COLUMNS} FROM knows \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Knows::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
