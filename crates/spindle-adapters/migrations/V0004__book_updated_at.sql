-- =============================================================================
-- V0004: add `updated_at` to the book table.
--
-- The V0001 schema gave updated_at to every other manuscript-level entity
-- (chapter, scene, character, location, faction, …) but missed book. The
-- generic update_entity / update_entity_field path unconditionally sets
-- updated_at as part of every UPDATE, so any attempt to write a book row
-- (e.g. update_entity entity_type="book" field="title") failed with
-- "no such column: updated_at".
--
-- This migration:
--   1. Adds book.updated_at as a nullable INTEGER (matches chapter.updated_at
--      shape — the SurrealDB legacy schema had it that way, and we want
--      symmetric semantics).
--   2. Backfills updated_at = created_at on existing rows so the column is
--      meaningful from the start instead of NULL for legacy data.
-- =============================================================================

ALTER TABLE book ADD COLUMN updated_at INTEGER;

UPDATE book SET updated_at = created_at WHERE updated_at IS NULL;
