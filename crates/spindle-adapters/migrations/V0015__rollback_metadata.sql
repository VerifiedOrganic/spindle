-- =============================================================================
-- V0015: add rollback metadata to style revision patch audit.
-- =============================================================================

ALTER TABLE style_revision_patch_audit ADD COLUMN rolled_back_at TEXT;
ALTER TABLE style_revision_patch_audit ADD COLUMN rollback_status TEXT NOT NULL DEFAULT 'not_rolled_back';
