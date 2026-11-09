use super::*;

/// In-transaction inserts backing the atomic canonical batch create tools.
/// A failure in any item aborts the shared `Repository::with_transaction`
/// closure and rolls the whole batch back.
impl CanonTransaction<'_> {
    pub fn create_term_in_tx(
        &self,
        project_id: &str,
        branch_id: &str,
        item: &spindle_core::models::BatchCreateTermItem,
    ) -> Result<String> {
        let id = mint_id("term");
        let normalized = normalize_name(&item.term_text);
        let now = timestamp_to_micros(chrono::Utc::now());
        self.raw_tx().execute(
            "INSERT INTO term (id, project_id, branch_id, term_text, normalized_term, \
             pronunciation, definition, usage_context, origin, notes, archived_at, \
             created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, ?10, ?10)",
            rusqlite::params![
                &id,
                project_id,
                branch_id,
                &item.term_text,
                &normalized,
                &item.pronunciation,
                &item.definition,
                &item.usage_context,
                &item.origin,
                now,
            ],
        )?;
        Ok(id)
    }

    pub fn create_motif_in_tx(
        &self,
        project_id: &str,
        branch_id: &str,
        item: &spindle_core::models::BatchCreateMotifItem,
    ) -> Result<String> {
        let id = mint_id("motif");
        let normalized = normalize_name(&item.name);
        let connected_theme_json = serde_json::to_string(&item.connected_theme_ids)
            .context("serializing connected_theme_ids")?;
        let now = timestamp_to_micros(chrono::Utc::now());
        self.raw_tx().execute(
            "INSERT INTO motif (id, project_id, branch_id, name, normalized_name, \
             description, max_uses_per_chapter, connected_theme_ids, notes, \
             archived_at, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?9)",
            rusqlite::params![
                &id,
                project_id,
                branch_id,
                &item.name,
                &normalized,
                &item.description,
                item.max_uses_per_chapter,
                &connected_theme_json,
                now,
            ],
        )?;
        Ok(id)
    }

    pub fn upsert_character_voice_profile_in_tx(
        &self,
        character_id: &str,
        profile: &spindle_core::models::CharacterVoiceProfileData,
    ) -> Result<()> {
        let new_id = mint_id("character_voice_profile");
        let vocabulary = serde_json::to_string(&profile.vocabulary)?;
        let sentence_structure = serde_json::to_string(&profile.sentence_structure)?;
        let tics = serde_json::to_string(&profile.tics)?;
        let forbidden_words = serde_json::to_string(&profile.forbidden_words)?;
        let example_lines = serde_json::to_string(&profile.example_lines)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        self.raw_tx().execute(
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
                character_id,
                &vocabulary,
                &sentence_structure,
                &tics,
                &forbidden_words,
                &example_lines,
                &profile.tone,
                &profile.established_in_scene_id,
                now,
            ],
        )?;
        Ok(())
    }
}
