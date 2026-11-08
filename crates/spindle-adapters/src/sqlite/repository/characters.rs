use super::*;

impl Repository {
    /// Character states persisted strictly after the supplied position on
    /// the given branch. Used by `revise_scene` to flag state markers
    /// that may be invalidated when a scene's prose is rewritten — every
    /// later state was computed against the original scene context.
    pub async fn list_character_states_after_position(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Vec<CharacterState>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {CHARACTER_STATE_COLUMNS} FROM character_state \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND ( \
                            book_number > ?3 \
                         OR (book_number = ?3 AND chapter_number > ?4) \
                         OR (book_number = ?3 AND chapter_number = ?4 AND scene_order > ?5) \
                       ) \
                     ORDER BY book_number, chapter_number, scene_order, created_at DESC"
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
                                scene_order
                            ],
                            |r| CharacterState::try_from(r),
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for row in branch_rows {
                        let key = (
                            row.character_id.clone(),
                            row.book_number,
                            row.chapter_number,
                            row.scene_order,
                        );
                        if seen.insert(key) {
                            rows.push(row);
                        }
                    }
                }
                rows.sort_by(|a, b| {
                    (a.book_number, a.chapter_number, a.scene_order).cmp(&(
                        b.book_number,
                        b.chapter_number,
                        b.scene_order,
                    ))
                });
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Character + voice/emotional profile reads
    // =========================================================================

    pub async fn get_character(&self, id: &str) -> Result<Character> {
        let id = id.to_string();
        let character = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {CHARACTER_COLUMNS} FROM character WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Character::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("character not found"))?;
        Ok(character)
    }

    /// Rename a character in one transaction: moves the display name AND the
    /// `normalized_name` uniqueness key, and preserves the old name as an
    /// alias (unless only casing/spacing changed, or the old name is already
    /// aliased). Returns the updated row plus whether the old name was added
    /// as an alias.
    ///
    /// The unique index `idx_character_project_name(project_id,
    /// normalized_name)` is project-wide (not branch-scoped), so the
    /// collision check is too: a character on an alternative branch holds its
    /// name against every branch.
    pub async fn rename_character(
        &self,
        character_id: &str,
        new_name: &str,
    ) -> Result<(Character, bool)> {
        let character_id = character_id.to_string();
        let new_name = new_name.to_string();
        let (character, old_name_kept_as_alias) = self
            .inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                let outcome = rename_character_in_tx(&tx, &character_id, &new_name)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
                tx.commit()?;
                Ok(outcome)
            })
            .await?;
        Ok((character, old_name_kept_as_alias))
    }

    /// Non-archived arcs of one character on a branch whose notes or
    /// milestones JSON mention `needle` (case-insensitive LIKE). Used by the
    /// character-rename report; the arc's character_id link itself is
    /// unaffected by renames.
    pub async fn list_character_arc_ids_mentioning(
        &self,
        project_id: &str,
        branch_id: &str,
        character_id: &str,
        needle: &str,
    ) -> Result<Vec<String>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let character_id = character_id.to_string();
        let pattern = format!("%{needle}%");
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT id FROM character_arc \
                     WHERE project_id = ?1 AND branch_id = ?2 AND character_id = ?3 \
                       AND archived_at IS NULL \
                       AND (COALESCE(notes, '') LIKE ?4 OR milestones LIKE ?4) \
                     ORDER BY created_at",
                )?;
                let rows = stmt
                    .query_map(
                        rusqlite::params![&project_id, &branch_id, &character_id, &pattern],
                        |r| r.get::<_, String>(0),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_characters_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Character>> {
        list_branch_scoped(
            &self.inner.pool,
            CHARACTER_COLUMNS,
            "character",
            "normalized_name",
            project_id,
            branch_id,
            |r| Character::try_from(r),
            |c| c.normalized_name.clone(),
        )
        .await
    }

    pub async fn get_character_voice_profile(
        &self,
        character_id: &str,
    ) -> Result<CharacterVoiceProfile> {
        let character_id = character_id.to_string();
        let profile = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_VOICE_PROFILE_COLUMNS} FROM character_voice_profile \
                     WHERE character_id = ?1 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&character_id], |r| CharacterVoiceProfile::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("voice profile not found"))?;
        Ok(profile)
    }

    pub async fn get_character_emotional_profile(
        &self,
        character_id: &str,
    ) -> Result<CharacterEmotionalProfile> {
        let character_id = character_id.to_string();
        let profile = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_EMOTIONAL_PROFILE_COLUMNS} FROM character_emotional_profile \
                     WHERE character_id = ?1 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&character_id], |r| CharacterEmotionalProfile::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("emotional profile not found"))?;
        Ok(profile)
    }

    // =========================================================================
    // Character arc
    // =========================================================================

    pub async fn create_character_arc(
        &self,
        input: &CreateCharacterArcInput,
    ) -> Result<CharacterArc> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("character_arc");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let character_id = input.character_id.clone();
        let arc_type = input.arc_type.clone();
        let starting_state = input.starting_state.clone();
        let ending_state = input.ending_state.clone();
        let thematic_purpose = input.thematic_purpose.clone();
        let milestones: Vec<StoredCharacterArcMilestone> =
            input.milestones.iter().cloned().map(Into::into).collect();
        let milestones_json =
            serde_json::to_string(&milestones).context("serializing milestones")?;
        let connected_theme_json = serde_json::to_string(&input.connected_theme_ids)
            .context("serializing connected_theme_ids")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO character_arc (id, project_id, branch_id, character_id, \
                     arc_type, starting_state, ending_state, milestones, thematic_purpose, \
                     connected_theme_ids, status, progress, notes, archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'planned', 0.0, NULL, NULL, ?11, ?11)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &character_id,
                        &arc_type,
                        &starting_state,
                        &ending_state,
                        &milestones_json,
                        &thematic_purpose,
                        &connected_theme_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_character_arc(&id_out).await
    }

    pub async fn get_character_arc(&self, id: &str) -> Result<CharacterArc> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {CHARACTER_ARC_COLUMNS} FROM character_arc WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| CharacterArc::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("character_arc not found"))
    }

    // =========================================================================
    // Character creation (4-table transaction)
    // =========================================================================

    /// Create a character along with its voice profile, emotional profile, and
    /// the baseline `character_state` snapshot in a single transaction.
    ///
    /// Atomic semantics: any failure rolls all four inserts back, so a
    /// half-formed character can never appear in the database.
    pub async fn create_character(
        &self,
        input: &CreateCharacterInput,
    ) -> Result<(
        Character,
        CharacterVoiceProfile,
        CharacterEmotionalProfile,
        CharacterState,
    )> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let character_id = mint_id("character");
        let voice_id = mint_id("character_voice_profile");
        let emotional_id = mint_id("character_emotional_profile");
        let state_id = mint_id("character_state");
        let now = timestamp_to_micros(chrono::Utc::now());

        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let summary = input.summary.clone();
        let role = input.role.clone();
        let realm = input.realm.clone();
        let aliases_json = serde_json::to_string(&input.aliases)?;

        // Voice profile (with tone + established_in_scene_id from input).
        let voice = input.voice_profile.clone();
        let vocab_json = serde_json::to_string(&voice.vocabulary)?;
        let sentence_json = serde_json::to_string(&voice.sentence_structure)?;
        let tics_json = serde_json::to_string(&voice.tics)?;
        let forbidden_json = serde_json::to_string(&voice.forbidden_words)?;
        let example_json = serde_json::to_string(&voice.example_lines)?;
        let voice_tone = voice.tone;
        let voice_established = voice.established_in_scene_id;

        // Emotional profile.
        let emo = input.emotional_profile.clone();
        let base_emotions_json = serde_json::to_string(&emo.base_emotions)?;
        let suppressed_json = serde_json::to_string(&emo.suppressed)?;
        let triggers_json = serde_json::to_string(&emo.triggers)?;
        let defense_json = serde_json::to_string(&emo.defense_mechanisms)?;
        let flex_stored: Option<StoredFlexRange> = emo.flex_range.map(Into::into);
        let flex_json = match flex_stored.as_ref() {
            Some(f) => Some(serde_json::to_string(f)?),
            None => None,
        };

        // Initial character_state (defaults match the SurrealDB repo).
        let patch = input.initial_state.clone().unwrap_or(CharacterStatePatch {
            emotional_state: std::collections::BTreeMap::new(),
            goals: Some(Vec::new()),
            status: Some(Vec::new()),
            notes: Some(Vec::new()),
            source_summary: Some("baseline".to_string()),
        });
        let emotional_state_json = serde_json::to_string(&patch.emotional_state)?;
        let goals_json = serde_json::to_string(&patch.goals.unwrap_or_default())?;
        let status_json = serde_json::to_string(&patch.status.unwrap_or_default())?;
        let notes_json = serde_json::to_string(&patch.notes.unwrap_or_default())?;
        let source_summary = patch.source_summary;

        let character_id_out = character_id.clone();
        let voice_id_out = voice_id.clone();
        let emotional_id_out = emotional_id.clone();
        let state_id_out = state_id.clone();

        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO character (id, project_id, branch_id, name, normalized_name, \
                     summary, role, realm, appearance, notes, created_at, updated_at, aliases) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?9, ?10)",
                    rusqlite::params![
                        &character_id,
                        &project_id,
                        &branch_id,
                        &name,
                        &normalized,
                        &summary,
                        &role,
                        &realm,
                        now,
                        &aliases_json,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO character_voice_profile (id, character_id, vocabulary, \
                     sentence_structure, tics, forbidden_words, example_lines, tone, \
                     established_in_scene_id, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    rusqlite::params![
                        &voice_id,
                        &character_id,
                        &vocab_json,
                        &sentence_json,
                        &tics_json,
                        &forbidden_json,
                        &example_json,
                        &voice_tone,
                        &voice_established,
                        now,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO character_emotional_profile (id, character_id, base_emotions, \
                     suppressed, triggers, defense_mechanisms, flex_range, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    rusqlite::params![
                        &emotional_id,
                        &character_id,
                        &base_emotions_json,
                        &suppressed_json,
                        &triggers_json,
                        &defense_json,
                        &flex_json,
                        now,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO character_state (id, project_id, branch_id, character_id, \
                     scene_id, book_number, chapter_number, scene_order, emotional_state, \
                     goals, status, notes, source_summary, created_at) \
                     VALUES (?1, ?2, ?3, ?4, NULL, 1, 1, 0, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        &state_id,
                        &project_id,
                        &branch_id,
                        &character_id,
                        &emotional_state_json,
                        &goals_json,
                        &status_json,
                        &notes_json,
                        &source_summary,
                        now,
                    ],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;

        let character = self.get_character(&character_id_out).await?;
        let voice = self
            .get_character_voice_profile_by_id(&voice_id_out)
            .await?;
        let emotional = self
            .get_character_emotional_profile_by_id(&emotional_id_out)
            .await?;
        let state = self.get_character_state_by_id(&state_id_out).await?;
        Ok((character, voice, emotional, state))
    }

    /// Direct-by-id getter for the freshly inserted voice profile. Public so
    /// callers can re-fetch by id; the existing get_character_voice_profile
    /// looks up by character_id which is the more common access pattern.
    /// UPSERT the voice profile for a character. The profile is per-character
    /// (not per-branch), so the unique key is character_id alone.
    pub async fn set_character_voice_profile(
        &self,
        character_id: &str,
        profile: &spindle_core::models::CharacterVoiceProfileData,
    ) -> Result<CharacterVoiceProfile> {
        let new_id = mint_id("character_voice_profile");
        let character_id_owned = character_id.to_string();
        let tone = profile.tone.clone();
        let vocabulary = serde_json::to_string(&profile.vocabulary)?;
        let sentence_structure = serde_json::to_string(&profile.sentence_structure)?;
        let tics = serde_json::to_string(&profile.tics)?;
        let forbidden_words = serde_json::to_string(&profile.forbidden_words)?;
        let example_lines = serde_json::to_string(&profile.example_lines)?;
        let established = profile.established_in_scene_id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let character_id = character_id_owned.clone();
                move |conn| {
                    conn.execute(
                        "INSERT INTO character_voice_profile (id, character_id, vocabulary, \
                         sentence_structure, tics, forbidden_words, example_lines, tone, \
                         established_in_scene_id, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10) \
                         ON CONFLICT (character_id) DO UPDATE SET \
                            vocabulary = excluded.vocabulary, \
                            sentence_structure = excluded.sentence_structure, \
                            tics = excluded.tics, \
                            forbidden_words = excluded.forbidden_words, \
                            example_lines = excluded.example_lines, \
                            tone = excluded.tone, \
                            established_in_scene_id = excluded.established_in_scene_id, \
                            updated_at = excluded.updated_at",
                        rusqlite::params![
                            &new_id,
                            &character_id,
                            &vocabulary,
                            &sentence_structure,
                            &tics,
                            &forbidden_words,
                            &example_lines,
                            &tone,
                            &established,
                            now,
                        ],
                    )?;
                    Ok(())
                }
            })
            .await?;
        self.get_character_voice_profile(&character_id_owned).await
    }

    pub async fn get_character_voice_profile_by_id(
        &self,
        id: &str,
    ) -> Result<CharacterVoiceProfile> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_VOICE_PROFILE_COLUMNS} FROM character_voice_profile \
                     WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| CharacterVoiceProfile::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("voice profile not found"))
    }

    pub async fn get_character_emotional_profile_by_id(
        &self,
        id: &str,
    ) -> Result<CharacterEmotionalProfile> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_EMOTIONAL_PROFILE_COLUMNS} FROM character_emotional_profile \
                     WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| CharacterEmotionalProfile::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("emotional profile not found"))
    }

    pub async fn get_character_state_by_id(&self, id: &str) -> Result<CharacterState> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {CHARACTER_STATE_COLUMNS} FROM character_state WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| CharacterState::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("character_state not found"))
    }

    pub async fn append_progression_event(
        &self,
        params: AppendProgressionEventParams,
    ) -> Result<ProgressionEvent> {
        let id = mint_id("progression_event");
        let id_out = id.clone();
        let delta_json = serde_json::to_string(&params.delta_json)?;
        let placement_stored: Option<StoredStoryPlacement> = params.placement.map(Into::into);
        let placement_json = match placement_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let created_at = timestamp_to_micros(params.created_at);
        let AppendProgressionEventParams {
            project_id,
            branch_id,
            subject_table,
            subject_id,
            overlay_id,
            kind,
            source_scene_id,
            ..
        } = params;

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO progression_event (id, project_id, branch_id, subject_table, \
                     subject_id, overlay_id, kind, delta_json, source_scene_id, placement, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &subject_table, &subject_id, &overlay_id,
                        &kind, &delta_json, &source_scene_id, &placement_json, created_at,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_progression_event(&id_out).await
    }

    pub async fn get_progression_event(&self, id: &str) -> Result<ProgressionEvent> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PROGRESSION_EVENT_COLUMNS} FROM progression_event WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ProgressionEvent::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("progression_event not found"))
    }

    /// Resolve the most recent character_state for (character, scene) at or
    /// before that scene's narrative position on a specific branch (inheriting
    /// from ancestor branches along `parent_branch_id`). Returns None if no
    /// state has been committed.
    pub async fn resolve_character_state_for_branch(
        &self,
        branch_id: &str,
        character_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Option<CharacterState>> {
        let branch_id = branch_id.to_string();
        let character_id = character_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                resolve_character_state_for_branch_in_conn(
                    conn,
                    &branch_id,
                    &character_id,
                    book_number,
                    chapter_number,
                    scene_order,
                )
            })
            .await
    }

    // =========================================================================
    // Character state (append-only snapshots per scene)
    // =========================================================================

    pub async fn append_character_state(
        &self,
        params: AppendCharacterStateParams,
    ) -> Result<CharacterState> {
        let id = mint_id("character_state");
        let id_lookup = id.clone();
        let project_id = params.project_id;
        let branch_id = params.branch_id;
        let character_id = params.character_id;
        let scene_id = params.scene_id;
        let book_number = params.book_number;
        let chapter_number = params.chapter_number;
        let scene_order = params.scene_order;
        let emotional_json = serde_json::to_string(&params.patch.emotional_state)
            .context("serializing emotional_state")?;
        let goals_json = serde_json::to_string(&params.patch.goals.unwrap_or_default())
            .context("serializing goals")?;
        let status_json = serde_json::to_string(&params.patch.status.unwrap_or_default())
            .context("serializing status")?;
        let notes_json = serde_json::to_string(&params.patch.notes.unwrap_or_default())
            .context("serializing notes")?;
        let source_summary = params.patch.source_summary;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO character_state (id, project_id, branch_id, character_id, \
                     scene_id, book_number, chapter_number, scene_order, emotional_state, \
                     goals, status, notes, source_summary, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &character_id,
                        &scene_id,
                        book_number,
                        chapter_number,
                        scene_order,
                        &emotional_json,
                        &goals_json,
                        &status_json,
                        &notes_json,
                        &source_summary,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {CHARACTER_STATE_COLUMNS} FROM character_state WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| CharacterState::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("character_state vanished after insert"))
    }

    /// Most recent character_state row for a character at or before
    /// (book, chapter, scene_order), resolved against the project's active
    /// branch (and its ancestor chain). Used by snapshot/briefing flows.
    pub async fn resolve_character_state(
        &self,
        character_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Option<CharacterState>> {
        let character_id = character_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let Some((project_id, char_branch_id)) = conn
                    .query_row(
                        "SELECT project_id, branch_id FROM character WHERE id = ?1",
                        [&character_id],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                    )
                    .optional_inner()?
                else {
                    return Ok(None);
                };
                let active_branch: Option<String> = conn
                    .query_row(
                        "SELECT active_branch_id FROM project WHERE id = ?1",
                        [&project_id],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .optional_inner()?
                    .flatten();
                let target_branch = active_branch.unwrap_or(char_branch_id);
                resolve_character_state_for_branch_in_conn(
                    conn,
                    &target_branch,
                    &character_id,
                    book_number,
                    chapter_number,
                    scene_order,
                )
            })
            .await
    }

    pub async fn list_character_states_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CharacterState>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {CHARACTER_STATE_COLUMNS} FROM character_state \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY character_id, book_number, chapter_number, scene_order, created_at DESC, rowid DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = stmt
                        .query_map(rusqlite::params![&project_id, candidate_branch_id], |r| {
                            CharacterState::try_from(r)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for row in rows {
                        let key = (
                            row.character_id.clone(),
                            row.book_number,
                            row.chapter_number,
                            row.scene_order,
                        );
                        if seen.insert(key) {
                            out.push(row);
                        }
                    }
                }
                out.sort_by(|a, b| {
                    (&a.character_id, a.book_number, a.chapter_number, a.scene_order).cmp(&(
                        &b.character_id,
                        b.book_number,
                        b.chapter_number,
                        b.scene_order,
                    ))
                });
                Ok(out)
            })
            .await
    }

    pub async fn list_character_arcs_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CharacterArc>> {
        list_branch_scoped(
            &self.inner.pool,
            CHARACTER_ARC_COLUMNS,
            "character_arc",
            "character_id, created_at",
            project_id,
            branch_id,
            |r| CharacterArc::try_from(r),
            |a| (a.character_id.clone(), a.arc_type.clone()),
        )
        .await
    }

    // -------------------------------------------------------------------------
    // `list_X_by_project` convenience wrappers that resolve to the project's
    // active branch and delegate to the branch-scoped lister. Mirrors the
    // SurrealDB repo's pattern where service-layer callers don't always have
    // a branch_id at hand.
    // -------------------------------------------------------------------------

    pub async fn list_characters_by_project(&self, project_id: &str) -> Result<Vec<Character>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_characters_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_character_arcs_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<CharacterArc>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_character_arcs_by_project_and_branch(project_id, &branch_id)
            .await
    }

    /// List every character_voice_profile row tied to characters visible on the
    /// given branch (including characters inherited from ancestor branches).
    pub async fn list_voice_profiles_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CharacterVoiceProfile>> {
        let characters = self
            .list_characters_by_project_and_branch(project_id, branch_id)
            .await?;
        if characters.is_empty() {
            return Ok(Vec::new());
        }
        let char_ids: Vec<String> = characters.into_iter().map(|c| c.id).collect();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..char_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {CHARACTER_VOICE_PROFILE_COLUMNS} FROM character_voice_profile \
                     WHERE character_id IN ({placeholders})"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let params: Vec<&dyn rusqlite::ToSql> = char_ids
                    .iter()
                    .map(|id| id as &dyn rusqlite::ToSql)
                    .collect();
                let rows = stmt
                    .query_map(&params[..], |r| CharacterVoiceProfile::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_characters(&self, project_id: &str) -> Result<Vec<Character>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_COLUMNS} FROM character \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Character::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_character_voice_profiles(
        &self,
        project_id: &str,
    ) -> Result<Vec<CharacterVoiceProfile>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_VOICE_PROFILE_COLUMNS} FROM character_voice_profile \
                     WHERE character_id IN ( \
                       SELECT id FROM character WHERE project_id = ?1 \
                     ) ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| CharacterVoiceProfile::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_character_emotional_profiles(
        &self,
        project_id: &str,
    ) -> Result<Vec<CharacterEmotionalProfile>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_EMOTIONAL_PROFILE_COLUMNS} FROM character_emotional_profile \
                     WHERE character_id IN ( \
                       SELECT id FROM character WHERE project_id = ?1 \
                     ) ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| CharacterEmotionalProfile::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_character_states(&self, project_id: &str) -> Result<Vec<CharacterState>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_STATE_COLUMNS} FROM character_state \
                     WHERE project_id = ?1 \
                     ORDER BY character_id, book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| CharacterState::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_character_arcs(&self, project_id: &str) -> Result<Vec<CharacterArc>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_ARC_COLUMNS} FROM character_arc \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| CharacterArc::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_progression_events(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProgressionEvent>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PROGRESSION_EVENT_COLUMNS} FROM progression_event \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ProgressionEvent::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
