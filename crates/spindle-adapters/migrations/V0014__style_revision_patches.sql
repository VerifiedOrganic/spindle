-- =============================================================================
-- V0014: add style revision patch audit history.
-- =============================================================================

CREATE TABLE style_revision_patch_audit (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    target_ids_json TEXT NOT NULL,
    before_hashes_json TEXT NOT NULL,
    after_hashes_json TEXT NOT NULL,
    model_receipt_json TEXT
);

CREATE INDEX idx_style_revision_patch_audit_project
    ON style_revision_patch_audit(project_id, applied_at);
