-- =============================================================================
-- V0010: persist authoring scene research gates.
-- =============================================================================

ALTER TABLE authoring_run_scene ADD COLUMN research_required INTEGER;
ALTER TABLE authoring_run_scene ADD COLUMN research_tags TEXT NOT NULL DEFAULT '[]';
ALTER TABLE authoring_run_scene ADD COLUMN explicit_query TEXT;
