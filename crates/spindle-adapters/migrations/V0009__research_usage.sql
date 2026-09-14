-- =============================================================================
-- V0009: Add research usage tracking.
-- =============================================================================

CREATE TABLE research_usage (
    id                    TEXT PRIMARY KEY NOT NULL CHECK (id LIKE 'research_usage:%'),
    project_id            TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id             TEXT NOT NULL,
    run_id                TEXT NOT NULL,
    step_checkpoint_id    TEXT,
    scene_id              TEXT NOT NULL REFERENCES scene(id) ON DELETE CASCADE,
    source_ids            TEXT NOT NULL CHECK (json_valid(source_ids)),
    note_ids              TEXT NOT NULL CHECK (json_valid(note_ids)),
    claim_ids             TEXT NOT NULL CHECK (json_valid(claim_ids)),
    query_pack_input      TEXT NOT NULL CHECK (json_valid(query_pack_input)),
    context_hash          TEXT NOT NULL,
    created_at            INTEGER NOT NULL
);

CREATE INDEX idx_research_usage_project ON research_usage(project_id);
CREATE INDEX idx_research_usage_branch ON research_usage(branch_id);
CREATE INDEX idx_research_usage_run ON research_usage(run_id);
CREATE INDEX idx_research_usage_scene ON research_usage(scene_id);
