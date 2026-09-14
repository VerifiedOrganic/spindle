-- =============================================================================
-- V0045: learned fiction anti-slop false-positive suppressions (Phase 5).
--
-- When style learning (V0031) captures an operator edit that keeps a
-- scanner-flagged excerpt, that (shelf_id, normalized excerpt) is stored here
-- and applied on later scans. Soft-on-save and hard-on-verify locks are
-- unchanged: suppressions only drop matching hits.
--
-- Additive: a project that never learns has no rows and is unaffected.
-- =============================================================================

CREATE TABLE anti_slop_suppression (
    id                 TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'anti_slop_suppression:%'),
    project_id         TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id          TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    shelf_id           TEXT    NOT NULL,
    excerpt_normalized TEXT    NOT NULL,
    source             TEXT    NOT NULL DEFAULT 'style_edit'
                               CHECK (source IN ('style_edit', 'manual')),
    created_at         INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_anti_slop_suppression_dedupe
    ON anti_slop_suppression(project_id, branch_id, shelf_id, excerpt_normalized);

CREATE INDEX idx_anti_slop_suppression_project_branch
    ON anti_slop_suppression(project_id, branch_id);
