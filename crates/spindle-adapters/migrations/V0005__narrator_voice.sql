-- =============================================================================
-- V0005: add `narrator_voice` to the project table.
--
-- Character voice profiles describe how an individual character speaks in
-- dialogue. They cannot capture the *narrator* voice, which — for first-person
-- or close-third narration — IS the prose style of the whole book. Without a
-- first-class home for it, a "sarcastic, funny" narrator was silently written
-- as "quiet, dry, literary" because the only voice signal was character-scoped.
--
-- This migration adds a nullable JSON column holding a serialized
-- `spindle_core::style::NarratorVoice` (comedy_density, pacing_feel,
-- interiority_ratio, emotional_register, chapter_ending_style, notes).
--
--   * Nullable: existing projects have no narrator voice; NULL means "unset".
--   * No backfill: absence is meaningful, so legacy rows stay NULL.
--   * JSON validity is enforced at the application layer (serde), mirroring how
--     reader_contract round-trips; we avoid a CHECK so the column can be NULL.
-- =============================================================================

ALTER TABLE project ADD COLUMN narrator_voice TEXT;
