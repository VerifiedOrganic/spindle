use super::*;

impl Repository {
    // -------------------------------------------------------------------------
    // Quantity-continuity (V0020): schemes + stamped per-subject state.
    // -------------------------------------------------------------------------

    pub async fn upsert_project_quantity_scheme(
        &self,
        project_id: &str,
        branch_id: &str,
        scheme: &spindle_core::models::QuantityScheme,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let measure = scheme.measure.clone();
        let denominations = serde_json::to_string(&scheme.denominations)?;
        let bands = serde_json::to_string(&scheme.bands)?;
        let max_band_jump = scheme.max_band_jump;
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                conn.execute(
                    "INSERT INTO project_quantity_scheme \
                     (project_id, branch_id, measure, denominations, bands, max_band_jump, \
                      created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) \
                     ON CONFLICT(project_id, branch_id, measure) DO UPDATE SET \
                       denominations = excluded.denominations, \
                       bands = excluded.bands, \
                       max_band_jump = excluded.max_band_jump, \
                       updated_at = excluded.updated_at",
                    rusqlite::params![
                        &project_id,
                        &branch_id,
                        &measure,
                        &denominations,
                        &bands,
                        max_band_jump,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn get_project_quantity_scheme(
        &self,
        project_id: &str,
        branch_id: &str,
        measure: &str,
    ) -> Result<Option<crate::sqlite::records::StoredQuantityScheme>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let measure = measure.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {} FROM project_quantity_scheme \
                     WHERE project_id = ?1 AND branch_id = ?2 AND measure = ?3",
                    crate::sqlite::records::PROJECT_QUANTITY_SCHEME_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                for candidate_branch_id in &chain {
                    if let Some(scheme) = stmt
                        .query_row(
                            rusqlite::params![&project_id, candidate_branch_id, &measure],
                            |r| crate::sqlite::records::StoredQuantityScheme::try_from(r),
                        )
                        .optional_inner()?
                    {
                        return Ok(Some(scheme));
                    }
                }
                Ok(None)
            })
            .await
    }

    pub async fn list_project_quantity_schemes(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<crate::sqlite::records::StoredQuantityScheme>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {} FROM project_quantity_scheme \
                     WHERE project_id = ?1 AND branch_id = ?2 ORDER BY measure",
                    crate::sqlite::records::PROJECT_QUANTITY_SCHEME_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = stmt
                        .query_map(rusqlite::params![&project_id, candidate_branch_id], |r| {
                            crate::sqlite::records::StoredQuantityScheme::try_from(r)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for row in rows {
                        if seen.insert(row.scheme.measure.clone()) {
                            out.push(row);
                        }
                    }
                }
                out.sort_by(|a, b| a.scheme.measure.cmp(&b.scheme.measure));
                Ok(out)
            })
            .await
    }

    pub async fn append_quantity_state(
        &self,
        params: AppendQuantityStateParams,
    ) -> Result<crate::sqlite::records::StoredQuantityState> {
        let id = mint_id("quantity_state");
        let id_lookup = id.clone();
        let AppendQuantityStateParams {
            project_id,
            branch_id,
            subject_table,
            subject_id,
            measure,
            state,
            scene_id,
            book_number,
            chapter_number,
            scene_order,
        } = params;
        let amount = state.amount;
        let unit = state.unit;
        let band = state.band;
        let change_reason = state.change_reason;
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                conn.execute(
                    "INSERT INTO quantity_state \
                     (id, project_id, branch_id, subject_table, subject_id, measure, amount, \
                      unit, band, change_reason, scene_id, book_number, chapter_number, \
                      scene_order, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &subject_table,
                        &subject_id,
                        &measure,
                        amount,
                        &unit,
                        &band,
                        &change_reason,
                        &scene_id,
                        book_number,
                        chapter_number,
                        scene_order,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM quantity_state WHERE id = ?1",
                    crate::sqlite::records::QUANTITY_STATE_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| {
                    crate::sqlite::records::StoredQuantityState::try_from(r)
                })
                .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("quantity_state vanished after insert"))
    }

    /// Most recent quantity_state row for a subject's measure at or before the
    /// packed cursor index. Drives the "current amount/band" read.
    pub async fn latest_quantity_state_at_or_before(
        &self,
        project_id: &str,
        branch_id: &str,
        subject_table: &str,
        subject_id: &str,
        measure: &str,
        cursor_index: i64,
    ) -> Result<Option<crate::sqlite::records::StoredQuantityState>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let subject_table = subject_table.to_string();
        let subject_id = subject_id.to_string();
        let measure = measure.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                // Same packing as `format::story_index`.
                let position = "(book_number * 1000000 + chapter_number * 1000 + scene_order)";
                let sql = format!(
                    "SELECT {} FROM quantity_state \
                     WHERE project_id = ?1 AND branch_id = ?2 AND subject_table = ?3 \
                       AND subject_id = ?4 AND measure = ?5 AND {position} <= ?6 \
                     ORDER BY {position} DESC, created_at DESC LIMIT 1",
                    crate::sqlite::records::QUANTITY_STATE_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut best: Option<(
                    (i32, i32, i32),
                    usize,
                    crate::sqlite::records::StoredQuantityState,
                )> = None;
                for (depth, candidate_branch_id) in chain.iter().enumerate() {
                    if let Some(candidate) = stmt
                        .query_row(
                            rusqlite::params![
                                &project_id,
                                candidate_branch_id,
                                &subject_table,
                                &subject_id,
                                &measure,
                                cursor_index,
                            ],
                            |r| crate::sqlite::records::StoredQuantityState::try_from(r),
                        )
                        .optional_inner()?
                    {
                        let pos = (
                            candidate.book_number,
                            candidate.chapter_number,
                            candidate.scene_order,
                        );
                        let replace = match &best {
                            None => true,
                            Some((best_pos, best_depth, _)) => {
                                pos > *best_pos || (pos == *best_pos && depth < *best_depth)
                            }
                        };
                        if replace {
                            best = Some((pos, depth, candidate));
                        }
                    }
                }
                Ok(best.map(|(_, _, state)| state))
            })
            .await
    }

    /// All quantity-state rows on the branch, ordered by subject + measure then
    /// story position, so consecutive stamps for one subject's measure are
    /// adjacent (drives the band-monotonicity check).
    pub async fn list_quantity_states_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<crate::sqlite::records::StoredQuantityState>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {} FROM quantity_state WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY subject_table, subject_id, measure, \
                       (book_number * 1000000 + chapter_number * 1000 + scene_order), created_at",
                    crate::sqlite::records::QUANTITY_STATE_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut overridden = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = stmt
                        .query_map([&project_id, candidate_branch_id], |r| {
                            crate::sqlite::records::StoredQuantityState::try_from(r)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    let mut branch_keys = Vec::new();
                    for row in rows {
                        let key = (
                            row.subject_table.clone(),
                            row.subject_id.clone(),
                            row.measure.clone(),
                            row.book_number,
                            row.chapter_number,
                            row.scene_order,
                        );
                        if !overridden.contains(&key) {
                            branch_keys.push(key);
                            out.push(row);
                        }
                    }
                    overridden.extend(branch_keys);
                }
                out.sort_by(|a, b| {
                    (
                        &a.subject_table,
                        &a.subject_id,
                        &a.measure,
                        a.book_number,
                        a.chapter_number,
                        a.scene_order,
                        &a.created_at,
                    )
                        .cmp(&(
                            &b.subject_table,
                            &b.subject_id,
                            &b.measure,
                            b.book_number,
                            b.chapter_number,
                            b.scene_order,
                            &b.created_at,
                        ))
                });
                Ok(out)
            })
            .await
    }

    /// The earliest quantity-state stamp for a subject's measure within
    /// `book_number` at or before `cursor_index` — the book's starting reading,
    /// for per-book trajectory display. Pure read; no digest table.
    pub async fn earliest_quantity_state_in_book(
        &self,
        project_id: &str,
        branch_id: &str,
        subject_table: &str,
        subject_id: &str,
        measure: &str,
        cursor_index: i64,
    ) -> Result<Option<crate::sqlite::records::StoredQuantityState>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let subject_table = subject_table.to_string();
        let subject_id = subject_id.to_string();
        let measure = measure.to_string();
        // The book is the high radix of the packed cursor (book*1_000_000 + ...).
        let book_number = (cursor_index / 1_000_000) as i32;
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let position = "(book_number * 1000000 + chapter_number * 1000 + scene_order)";
                let sql = format!(
                    "SELECT {} FROM quantity_state \
                     WHERE project_id = ?1 AND branch_id = ?2 AND subject_table = ?3 \
                       AND subject_id = ?4 AND measure = ?5 AND book_number = ?6 \
                       AND {position} <= ?7 \
                     ORDER BY {position} ASC, created_at ASC LIMIT 1",
                    crate::sqlite::records::QUANTITY_STATE_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut best: Option<(
                    (i32, i32, i32),
                    usize,
                    crate::sqlite::records::StoredQuantityState,
                )> = None;
                for (depth, candidate_branch_id) in chain.iter().enumerate() {
                    if let Some(candidate) = stmt
                        .query_row(
                            rusqlite::params![
                                &project_id,
                                candidate_branch_id,
                                &subject_table,
                                &subject_id,
                                &measure,
                                book_number,
                                cursor_index,
                            ],
                            |r| crate::sqlite::records::StoredQuantityState::try_from(r),
                        )
                        .optional_inner()?
                    {
                        let pos = (
                            candidate.book_number,
                            candidate.chapter_number,
                            candidate.scene_order,
                        );
                        let replace = match &best {
                            None => true,
                            Some((best_pos, best_depth, _)) => {
                                pos < *best_pos || (pos == *best_pos && depth < *best_depth)
                            }
                        };
                        if replace {
                            best = Some((pos, depth, candidate));
                        }
                    }
                }
                Ok(best.map(|(_, _, state)| state))
            })
            .await
    }
}
