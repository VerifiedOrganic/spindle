-- =============================================================================
-- V0033: add `archived_at` to the research library tables.
--
-- Live-run bug 5: `archive_entity` failed on research rows with
--   "entity table 'research_claim' has no archived_at column"
-- so junk research sources/notes/claims (e.g. an unparseable-report artifact)
-- were unremovable. Archival is the house pattern for removing entities; these
-- three tables now carry the nullable `archived_at` column that the allowlisted
-- `archive_entity` UPDATE stamps, and every research consumer excludes rows
-- where `archived_at IS NOT NULL`.
-- =============================================================================

ALTER TABLE research_source ADD COLUMN archived_at INTEGER;
ALTER TABLE research_note ADD COLUMN archived_at INTEGER;
ALTER TABLE research_claim ADD COLUMN archived_at INTEGER;
