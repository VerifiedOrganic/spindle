use super::*;

impl Repository {
    // =========================================================================
    // Scene
    // =========================================================================

    pub async fn get_scene(&self, id: &str) -> Result<Scene> {
        let id = id.to_string();
        let scene = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {SCENE_COLUMNS} FROM scene WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Scene::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("scene not found"))?;
        Ok(scene)
    }

    pub async fn list_scenes_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Scene>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        Scene::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Resolve the effective scene spine for a branch along its parent chain
    /// (branch scenes override ancestor scenes at the same
    /// `(book_number, chapter_number, scene_order)` position).
    pub async fn list_scenes_resolved_for_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Scene>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = stmt
                        .query_map(rusqlite::params![&project_id, candidate_branch_id], |r| {
                            Scene::try_from(r)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for row in rows {
                        let key = (row.book_number, row.chapter_number, row.scene_order);
                        if seen.insert(key) {
                            out.push(row);
                        }
                    }
                }
                out.sort_by(|a, b| {
                    (a.book_number, a.chapter_number, a.scene_order).cmp(&(
                        b.book_number,
                        b.chapter_number,
                        b.scene_order,
                    ))
                });
                Ok(out)
            })
            .await
    }

    /// Scenes strictly *before* the supplied position (book, chapter, scene_order)
    /// on the given branch, ordered most-recent first by the SQL and then
    /// reversed to chronological so the caller can `take(limit)` from the end
    /// of the recent history. Mirrors the SurrealDB-era helper used by
    /// `get_writer_state` to build the recent-scenes window.
    pub async fn list_recent_scenes_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
        limit: usize,
    ) -> Result<Vec<Scene>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND ( \
                            book_number < ?3 \
                            OR (book_number = ?3 AND chapter_number < ?4) \
                            OR (book_number = ?3 AND chapter_number = ?4 AND scene_order < ?5) \
                       ) \
                     ORDER BY book_number DESC, chapter_number DESC, scene_order DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut seen = std::collections::HashSet::new();
                let mut rows = Vec::new();
                for candidate_branch_id in &chain {
                    let branch_rows = stmt
                        .query_map(
                            rusqlite::params![
                                &project_id,
                                candidate_branch_id,
                                book_number,
                                chapter_number,
                                scene_order,
                            ],
                            |r| Scene::try_from(r),
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for row in branch_rows {
                        let key = (row.book_number, row.chapter_number, row.scene_order);
                        if seen.insert(key) {
                            rows.push(row);
                        }
                    }
                }
                rows.sort_by(|a, b| {
                    (b.book_number, b.chapter_number, b.scene_order).cmp(&(
                        a.book_number,
                        a.chapter_number,
                        a.scene_order,
                    ))
                });
                rows.truncate(limit);
                rows.reverse();
                Ok(rows)
            })
            .await
    }

    pub async fn list_scenes_by_chapter(&self, chapter_id: &str) -> Result<Vec<Scene>> {
        let chapter_id = chapter_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE chapter_id = ?1 ORDER BY scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&chapter_id], |r| Scene::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// The active-spine scenes of one chapter. This is the ONE chapter-scoped
    /// spine predicate: `book`/`chapter` rows are branch-shared, so a
    /// `chapter_id`-only filter leaks every branch's variant rows into the
    /// listing (the live bug that showed three rows at one `scene_order`).
    /// Every surface that reports or addresses the spine of a chapter —
    /// `list_chapter_scenes`, `list_book_chapters`, `compile_manuscript` via
    /// `list_scenes_by_project_and_branch`, and `find_scene_by_natural_key` —
    /// filters on the same `branch_id`, so the listing, the compiled
    /// manuscript, and the position resolver can never disagree about which
    /// scenes are in the spine.
    pub async fn list_scenes_by_chapter_and_branch(
        &self,
        chapter_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Scene>> {
        let chapter_id = chapter_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE chapter_id = ?1 AND branch_id = ?2 ORDER BY scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&chapter_id, &branch_id], |r| {
                        Scene::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_scenes_by_book(&self, book_id: &str) -> Result<Vec<Scene>> {
        let book_id = book_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE book_id = ?1 ORDER BY chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&book_id], |r| Scene::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// All scenes located *after* the supplied (book, chapter, scene_order)
    /// triple in narrative order. Used by writer-state and continuity sweeps.
    pub async fn list_scenes_after_position(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Vec<Scene>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND ( \
                            book_number > ?3 \
                         OR (book_number = ?3 AND chapter_number > ?4) \
                         OR (book_number = ?3 AND chapter_number = ?4 AND scene_order > ?5) \
                       ) \
                     ORDER BY book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(
                        rusqlite::params![
                            &project_id,
                            &branch_id,
                            book_number,
                            chapter_number,
                            scene_order
                        ],
                        |r| Scene::try_from(r),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn find_scene_by_natural_key(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Option<Scene>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND book_number = ?3 AND chapter_number = ?4 AND scene_order = ?5 \
                     LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![
                        &project_id,
                        &branch_id,
                        book_number,
                        chapter_number,
                        scene_order
                    ],
                    |r| Scene::try_from(r),
                )
                .optional_inner()
            })
            .await
    }

    /// Highest existing version_number for a scene plus 1. Used when creating
    /// a new scene_version snapshot from an in-place scene edit.
    pub async fn next_scene_version_number(&self, scene_id: &str) -> Result<i32> {
        let scene_id = scene_id.to_string();
        let n: i64 = self
            .inner
            .pool
            .read(move |conn| {
                conn.query_row(
                    "SELECT COALESCE(MAX(version_number), 0) FROM scene_version WHERE scene_id = ?1",
                    [&scene_id],
                    |r| r.get(0),
                )
            })
            .await?;
        Ok((n + 1) as i32)
    }

    /// Production scene-creation path. Mirrors the SurrealDB save_scene_draft
    /// flow: find or create the scene by natural key, snapshot the previous
    /// version into scene_version when prose changes, and mark dependent
    /// dual_persona_review rows stale + open validator_finding rows resolved.
    ///
    /// Returns `(Scene, created)` where `created` is true when this call
    /// inserted a new scene rather than updating an existing one.
    pub async fn save_scene_draft(
        &self,
        project_id: &str,
        branch_id: &str,
        input: &SaveSceneDraftInput,
    ) -> Result<(Scene, bool)> {
        self.persist_scene(project_id, branch_id, input, true, None)
            .await
    }

    /// Like `save_scene_draft`, but additionally binds an explicit generation
    /// receipt to the saved scene (migration V0034) in the SAME transaction as
    /// the scene write. The claim commits if and only if the save commits: a
    /// save that fails at any point (bad placement, constraint violation,
    /// claim conflict) rolls the claim back and the receipt stays unbound and
    /// reusable. This closes the live-run hole where a FAILED save had
    /// already burned the receipt on the bogus placement.
    pub async fn save_scene_draft_with_receipt_claim(
        &self,
        project_id: &str,
        branch_id: &str,
        input: &SaveSceneDraftInput,
        receipt_id: &str,
        scene_key: &str,
    ) -> Result<(Scene, bool)> {
        let claim = (receipt_id.to_string(), scene_key.to_string());
        self.persist_scene(project_id, branch_id, input, true, Some(claim))
            .await
    }

    pub async fn update_scene_draft_origin(
        &self,
        scene_id: &str,
        draft_origin: &str,
    ) -> Result<()> {
        let scene_id = scene_id.to_string();
        let draft_origin = draft_origin.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE scene SET draft_origin = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![
                        &draft_origin,
                        timestamp_to_micros(chrono::Utc::now()),
                        &scene_id,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub(super) async fn persist_scene(
        &self,
        project_id: &str,
        branch_id: &str,
        input: &SaveSceneDraftInput,
        mark_reviews_stale_on_update: bool,
        receipt_claim: Option<(String, String)>,
    ) -> Result<(Scene, bool)> {
        // Bound the placement so it cannot collide in the packed story-position
        // ordering (see `format::SCENE_RADIX` / `CHAPTER_RADIX`). Without this a
        // chapter past the radix would silently transpose scenes across the next
        // book boundary at exactly the hundreds-of-chapters scale we target.
        if input.book_number < 0 {
            anyhow::bail!(
                "book_number must be non-negative (got {})",
                input.book_number
            );
        }
        if !(0..crate::format::CHAPTER_RADIX as i32).contains(&input.chapter_number) {
            anyhow::bail!(
                "chapter_number {} is out of range 0..{}; it would collide in story-position ordering",
                input.chapter_number,
                crate::format::CHAPTER_RADIX
            );
        }
        if !(0..crate::format::SCENE_RADIX as i32).contains(&input.scene_order) {
            anyhow::bail!(
                "scene_order {} is out of range 0..{}; it would collide in story-position ordering",
                input.scene_order,
                crate::format::SCENE_RADIX
            );
        }
        let book = self
            .get_book_by_number(project_id, input.book_number)
            .await?;
        let chapter = self
            .get_chapter_by_number(project_id, input.book_number, input.chapter_number)
            .await?;
        let content_rating = input.content_rating.as_db_str();

        let existing = self
            .find_scene_by_natural_key(
                project_id,
                branch_id,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
            .await?;

        let project_id_owned = project_id.to_string();
        let branch_id_owned = branch_id.to_string();
        let book_id = book.id.clone();
        let chapter_id = chapter.id.clone();
        let full_text = input.full_text.clone();
        let summary = input.summary.clone();
        let tone = input.tone.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        let book_number = input.book_number;
        let chapter_number = input.chapter_number;
        let scene_order = input.scene_order;
        let location_id = input.location_id.clone();
        let receipt_claim_owned = receipt_claim;

        if let Some(existing) = existing {
            // UPDATE path: snapshot the previous prose into scene_version when
            // it changed, then UPDATE the scene, then cascade status changes.
            let scene_changed = existing.full_text != full_text
                || existing.summary != summary
                || existing.content_rating != content_rating
                || existing.tone != tone;
            let next_version_number = if scene_changed {
                Some(self.next_scene_version_number(&existing.id).await?)
            } else {
                None
            };

            let existing_id = existing.id.clone();
            let existing_full_text = existing.full_text.clone();
            let existing_summary = existing.summary.clone();
            let existing_content_rating = existing.content_rating.clone();
            let existing_tone = existing.tone.clone();
            let new_full_text = full_text.clone();
            let new_summary = summary.clone();
            let new_content_rating = content_rating.to_string();
            let new_tone = tone.clone();

            self.inner
                .pool
                .write(move |conn| {
                    let tx = conn.transaction()?;
                    // The receipt claim commits with the scene write or not at
                    // all (migration V0034 + the failed-save regression).
                    if let Some((receipt_id, scene_key)) = receipt_claim_owned.as_ref() {
                        claim_generation_receipt_in_tx(&tx, receipt_id, scene_key)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
                    }
                    if let Some(v) = next_version_number {
                        let version_id = mint_id_local("scene_version");
                        tx.execute(
                            "INSERT INTO scene_version (id, project_id, branch_id, scene_id, \
                             version_number, book_number, chapter_number, scene_order, \
                             full_text, summary, content_rating, tone, created_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                            rusqlite::params![
                                &version_id,
                                &project_id_owned,
                                &branch_id_owned,
                                &existing_id,
                                v,
                                book_number,
                                chapter_number,
                                scene_order,
                                &existing_full_text,
                                &existing_summary,
                                &existing_content_rating,
                                &existing_tone,
                                now,
                            ],
                        )?;
                    }
                    tx.execute(
                        // COALESCE preserves the existing location when this
                        // re-save omits one, so a prose-only edit never clears it.
                        "UPDATE scene SET full_text = ?1, summary = ?2, content_rating = ?3, \
                         tone = ?4, location_id = COALESCE(?5, location_id), updated_at = ?6 \
                         WHERE id = ?7",
                        rusqlite::params![
                            &new_full_text,
                            &new_summary,
                            &new_content_rating,
                            &new_tone,
                            &location_id,
                            now,
                            &existing_id,
                        ],
                    )?;
                    if scene_changed {
                        // Resolve any open validator findings on the prior version.
                        tx.execute(
                            "UPDATE validator_finding SET resolved_at = ?1 \
                             WHERE scene_id = ?2 AND branch_id = ?3 AND resolved_at IS NULL",
                            rusqlite::params![now, &existing_id, &branch_id_owned],
                        )?;
                    }
                    if mark_reviews_stale_on_update && scene_changed {
                        tx.execute(
                            "UPDATE dual_persona_review SET status = 'stale', updated_at = ?1 \
                             WHERE branch_id = ?2 AND scene_id = ?3",
                            rusqlite::params![now, &branch_id_owned, &existing_id],
                        )?;
                    }
                    tx.commit()?;
                    Ok(())
                })
                .await?;
            let updated = self.get_scene(&existing.id).await?;
            Ok((updated, false))
        } else {
            // INSERT path: brand-new scene at this natural key. Wrapped in a
            // transaction so the optional receipt claim commits atomically
            // with the scene row (or not at all).
            let scene_id = mint_id("scene");
            let scene_id_lookup = scene_id.clone();
            let rating_owned = content_rating.to_string();
            self.inner
                .pool
                .write(move |conn| {
                    let tx = conn.transaction()?;
                    if let Some((receipt_id, scene_key)) = receipt_claim_owned.as_ref() {
                        claim_generation_receipt_in_tx(&tx, receipt_id, scene_key)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
                    }
                    tx.execute(
                        "INSERT INTO scene (id, project_id, branch_id, book_id, chapter_id, \
                         book_number, chapter_number, scene_order, full_text, summary, \
                         content_rating, tone, draft_origin, location_id, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13, ?14, ?14)",
                        rusqlite::params![
                            &scene_id,
                            &project_id_owned,
                            &branch_id_owned,
                            &book_id,
                            &chapter_id,
                            book_number,
                            chapter_number,
                            scene_order,
                            &full_text,
                            &summary,
                            &rating_owned,
                            &tone,
                            &location_id,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                })
                .await?;
            let created = self.get_scene(&scene_id_lookup).await?;
            Ok((created, true))
        }
    }

    /// Scene ids on a branch whose prose or summary matches a phrase, via the
    /// FTS5 scene index. Used by the character-rename report to surface prose
    /// mentions of the old name (rename never rewrites scene text). The
    /// phrase is tokenized through `fts_safe_query`, so arbitrary names
    /// (hyphens, apostrophes, quotes) cannot break the MATCH syntax; a phrase
    /// with no alphanumeric tokens matches nothing.
    pub async fn list_scene_ids_mentioning_phrase(
        &self,
        project_id: &str,
        branch_id: &str,
        phrase: &str,
        limit: usize,
    ) -> Result<Vec<String>> {
        let Some(query) = fts_safe_query(phrase) else {
            return Ok(Vec::new());
        };
        let hits = self
            .fts_search_scenes(project_id, Some(branch_id), &query, limit)
            .await?;
        let mut ids: Vec<String> = Vec::new();
        for (scene_id, _rank, _snippet) in hits {
            if !ids.contains(&scene_id) {
                ids.push(scene_id);
            }
        }
        Ok(ids)
    }

    /// Restore an earlier scene_version's prose into the current scene row,
    /// snapshotting the current prose into a new scene_version and marking
    /// dual_persona_review rows on the scene stale. Mirrors the SurrealDB
    /// repo's `restore_scene_version_and_mark_reviews_stale`.
    pub async fn restore_scene_version_and_mark_reviews_stale(
        &self,
        scene_id: &str,
        scene_version: &SceneVersion,
    ) -> Result<Scene> {
        // The existing save_scene_draft path takes the UPDATE branch when a
        // scene at the natural key already exists, snapshots the prior prose,
        // and marks reviews stale. We synthesize the matching input and call
        // it via the project + branch on the existing scene record.
        use spindle_core::models::{ContentRating, SaveSceneDraftInput};
        let existing = self.get_scene(scene_id).await?;
        let content_rating = match scene_version.content_rating.as_str() {
            "General" | "general" => ContentRating::General,
            "Teen" | "teen" => ContentRating::Teen,
            "Mature" | "mature" => ContentRating::Mature,
            "Explicit" | "explicit" => ContentRating::Explicit,
            other => anyhow::bail!("unknown content_rating in scene_version: {other}"),
        };
        let input = SaveSceneDraftInput {
            project_id: existing.project_id.clone(),
            book_number: existing.book_number,
            chapter_number: existing.chapter_number,
            chapter_id: None,
            scene_order: existing.scene_order,
            full_text: scene_version.full_text.clone(),
            summary: scene_version.summary.clone(),
            content_rating,
            tone: scene_version.tone.clone(),
            generation_id: None,
            source_path: None,
            ..Default::default()
        };
        let (scene, _created) = self
            .save_scene_draft(&existing.project_id, &existing.branch_id, &input)
            .await?;
        Ok(scene)
    }

    /// Delete a scene. Cascades sweep its dependents (scene_version,
    /// scene_beat_annotation, revision_marker, dual_persona_review,
    /// validator_finding, canonical_fact, scene_source_link) per the FK audit.
    /// Returns the deleted scene's prior state for callers that want to log
    /// the removal.
    pub async fn delete_scene(&self, scene_id: &str) -> Result<Scene> {
        let prior = self.get_scene(scene_id).await?;
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let n = conn.execute("DELETE FROM scene WHERE id = ?1", [&scene_id])?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        Ok(prior)
    }

    /// Move a scene to a new (book, chapter, scene_order) slot. Rejects the
    /// move if the destination is already occupied by another scene.
    #[allow(clippy::too_many_arguments)]
    pub async fn move_scene(
        &self,
        scene_id: &str,
        project_id: &str,
        branch_id: &str,
        destination_book_id: &str,
        destination_chapter_id: &str,
        destination_book_number: i32,
        destination_chapter_number: i32,
        destination_scene_order: i32,
    ) -> Result<Scene> {
        let scene_id_owned = scene_id.to_string();
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let book_id = destination_book_id.to_string();
        let chapter_id = destination_chapter_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                let occupied: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND book_number = ?3 AND chapter_number = ?4 AND scene_order = ?5 \
                       AND id != ?6",
                    rusqlite::params![
                        &project_id,
                        &branch_id,
                        destination_book_number,
                        destination_chapter_number,
                        destination_scene_order,
                        &scene_id_owned,
                    ],
                    |r| r.get(0),
                )?;
                if occupied > 0 {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                        Some("destination scene already exists".into()),
                    ));
                }
                tx.execute(
                    "UPDATE scene SET book_id = ?1, chapter_id = ?2, book_number = ?3, \
                     chapter_number = ?4, scene_order = ?5, updated_at = ?6 WHERE id = ?7",
                    rusqlite::params![
                        &book_id,
                        &chapter_id,
                        destination_book_number,
                        destination_chapter_number,
                        destination_scene_order,
                        now,
                        &scene_id_owned,
                    ],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.get_scene(scene_id).await
    }

    // =========================================================================
    // Scene versions + scene source links + research log
    // =========================================================================

    pub async fn list_scene_versions(&self, scene_id: &str) -> Result<Vec<SceneVersion>> {
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_VERSION_COLUMNS} FROM scene_version \
                     WHERE scene_id = ?1 ORDER BY version_number DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&scene_id], |r| SceneVersion::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn get_scene_version(&self, scene_version_id: &str) -> Result<SceneVersion> {
        let id = scene_version_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {SCENE_VERSION_COLUMNS} FROM scene_version WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| SceneVersion::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("scene_version not found"))
    }

    /// Upsert a scene_source_link row for a scene. There's at most one link
    /// per scene; later calls update the existing row.
    pub async fn upsert_scene_source_link(
        &self,
        project_id: &str,
        scene_id: &str,
        source_path: &str,
        content_sha256: &str,
        source_start_offset: Option<i64>,
        source_end_offset: Option<i64>,
    ) -> Result<SceneSourceLink> {
        let id = mint_id("scene_source_link");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let scene_id_owned = scene_id.to_string();
        let source_path = source_path.to_string();
        let content_sha256 = content_sha256.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let scene_id = scene_id_owned.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM scene_source_link WHERE scene_id = ?1",
                        [&scene_id],
                    )?;
                    tx.execute(
                        "INSERT INTO scene_source_link (id, project_id, scene_id, source_path, \
                         content_sha256, source_start_offset, source_end_offset, linked_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                        rusqlite::params![
                            &id, &project_id, &scene_id, &source_path, &content_sha256,
                            source_start_offset, source_end_offset, now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        let _ = id_out;
        self.get_scene_source_link_for_scene(scene_id)
            .await?
            .ok_or_else(|| anyhow!("scene_source_link vanished after upsert"))
    }

    pub async fn get_scene_source_link_for_scene(
        &self,
        scene_id: &str,
    ) -> Result<Option<SceneSourceLink>> {
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_SOURCE_LINK_COLUMNS} FROM scene_source_link \
                     WHERE scene_id = ?1 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&scene_id], |r| SceneSourceLink::try_from(r))
                    .optional_inner()
            })
            .await
    }

    pub async fn delete_scene_source_links_for_scene(&self, scene_id: &str) -> Result<()> {
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM scene_source_link WHERE scene_id = ?1",
                    [&scene_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_scene_source_links_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<SceneSourceLink>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_SOURCE_LINK_COLUMNS} FROM scene_source_link \
                     WHERE project_id = ?1 ORDER BY linked_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| SceneSourceLink::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Revision marker
    // =========================================================================

    /// UPSERT a revision marker keyed by
    /// (branch_id, scene_id, marker_type, target_record_id). Matches the
    /// SurrealDB unique index — repeated upserts overwrite the existing row's
    /// status / position / note / created_at.
    pub async fn upsert_revision_marker(&self, marker: &RevisionMarker) -> Result<RevisionMarker> {
        let project_id = marker.project_id.clone();
        let branch_id = marker.branch_id.clone();
        let scene_id = marker.scene_id.clone();
        let marker_type = marker.marker_type.clone();
        let target_record_id = marker.target_record_id.clone();
        let position = marker.position.clone();
        let note = marker.note.clone();
        let status = marker.status.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        let new_id = mint_id("revision_marker");
        let new_id_owned = new_id.clone();

        self.inner
            .pool
            .write({
                let branch_id = branch_id.clone();
                let scene_id = scene_id.clone();
                let marker_type = marker_type.clone();
                let target_record_id = target_record_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    // Delete an existing row matching the unique key (target may be NULL).
                    if let Some(target) = &target_record_id {
                        tx.execute(
                            "DELETE FROM revision_marker \
                             WHERE branch_id = ?1 AND scene_id = ?2 \
                               AND marker_type = ?3 AND target_record_id = ?4",
                            rusqlite::params![&branch_id, &scene_id, &marker_type, target],
                        )?;
                    } else {
                        tx.execute(
                            "DELETE FROM revision_marker \
                             WHERE branch_id = ?1 AND scene_id = ?2 \
                               AND marker_type = ?3 AND target_record_id IS NULL",
                            rusqlite::params![&branch_id, &scene_id, &marker_type],
                        )?;
                    }
                    tx.execute(
                        "INSERT INTO revision_marker (id, project_id, branch_id, scene_id, \
                         marker_type, target_record_id, position, note, status, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        rusqlite::params![
                            &new_id,
                            &project_id,
                            &branch_id,
                            &scene_id,
                            &marker_type,
                            &target_record_id,
                            &position,
                            &note,
                            &status,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_revision_marker(&new_id_owned).await
    }

    pub async fn resolve_revision_marker(&self, marker_id: &str) -> Result<RevisionMarker> {
        let marker_id_owned = marker_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let n = conn.execute(
                    "UPDATE revision_marker SET status = 'resolved' WHERE id = ?1",
                    [&marker_id_owned],
                )?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        self.get_revision_marker(marker_id).await
    }

    pub async fn get_revision_marker(&self, id: &str) -> Result<RevisionMarker> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {REVISION_MARKER_COLUMNS} FROM revision_marker WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| RevisionMarker::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("revision_marker not found"))
    }

    pub async fn list_revision_markers_for_scene(
        &self,
        branch_id: &str,
        scene_id: &str,
    ) -> Result<Vec<RevisionMarker>> {
        let branch_id = branch_id.to_string();
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {REVISION_MARKER_COLUMNS} FROM revision_marker \
                     WHERE branch_id = ?1 AND scene_id = ?2 \
                     ORDER BY marker_type, position"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&branch_id, &scene_id], |r| {
                        RevisionMarker::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_revision_markers(&self, project_id: &str) -> Result<Vec<RevisionMarker>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {REVISION_MARKER_COLUMNS} FROM revision_marker \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| RevisionMarker::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_scenes(&self, project_id: &str) -> Result<Vec<Scene>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 \
                     ORDER BY branch_id, book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Scene::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_scene_versions(&self, project_id: &str) -> Result<Vec<SceneVersion>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_VERSION_COLUMNS} FROM scene_version \
                     WHERE project_id = ?1 ORDER BY scene_id, version_number DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| SceneVersion::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
