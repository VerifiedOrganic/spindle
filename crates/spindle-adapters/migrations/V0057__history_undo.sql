-- Unified history: audit records for compensating undo operations. Source
-- operations stay in their native tables (scene_version, style application,
-- style patch audit, save_point); this table records which source was undone
-- and which new operation compensated it.
CREATE TABLE history_undo (
    id                         TEXT PRIMARY KEY,
    project_id                 TEXT NOT NULL,
    branch_id                  TEXT NOT NULL,
    source_operation_id        TEXT NOT NULL,
    compensating_operation_id  TEXT,
    kind                       TEXT NOT NULL,
    summary                    TEXT NOT NULL,
    created_at                 INTEGER NOT NULL
);

CREATE INDEX idx_history_undo_project_branch
    ON history_undo(project_id, branch_id, created_at DESC);
CREATE INDEX idx_history_undo_source
    ON history_undo(source_operation_id);
