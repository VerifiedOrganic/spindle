-- =============================================================================
-- V0007: add authoring run tables.
-- =============================================================================

CREATE TABLE authoring_run (
    id                          TEXT PRIMARY KEY,
    project_id                  TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    active_branch_id            TEXT NOT NULL,
    book_number                 INTEGER NOT NULL,
    start_chapter               INTEGER NOT NULL,
    end_chapter                 INTEGER NOT NULL,
    checkpoint_interval         INTEGER NOT NULL,
    last_checkpoint_end_chapter INTEGER NOT NULL,
    artifacts_dir               TEXT NOT NULL,
    editorial_directives        TEXT NOT NULL, -- JSON array of strings
    status                      TEXT NOT NULL, -- "active", "paused", "completed", "blocked"
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER NOT NULL
);

CREATE INDEX idx_authoring_run_project ON authoring_run(project_id);

CREATE TABLE authoring_run_chapter (
    authoring_run_id            TEXT NOT NULL REFERENCES authoring_run(id) ON DELETE CASCADE,
    chapter_number              INTEGER NOT NULL,
    planned                     INTEGER NOT NULL, -- boolean 0/1
    synopsis                    TEXT NOT NULL,
    pov_character_id            TEXT,
    status                      TEXT NOT NULL, -- "pending", "in_progress", "complete"
    summary_saved               INTEGER NOT NULL, -- boolean 0/1
    summary_artifact_path       TEXT,
    PRIMARY KEY (authoring_run_id, chapter_number)
);

CREATE TABLE authoring_run_scene (
    authoring_run_id            TEXT NOT NULL REFERENCES authoring_run(id) ON DELETE CASCADE,
    chapter_number              INTEGER NOT NULL,
    scene_order                 INTEGER NOT NULL,
    character_ids               TEXT NOT NULL, -- JSON array of strings
    location_id                 TEXT NOT NULL,
    content_rating              TEXT NOT NULL,
    tone                        TEXT,
    source_path                 TEXT,
    phase                       TEXT NOT NULL, -- "pending", "draft_saved", "changes_committed", "beats_annotated"
    scene_id                    TEXT,
    scene_artifact_path         TEXT,
    draft_diagnostics           TEXT, -- JSON object
    blocked_reason              TEXT,
    PRIMARY KEY (authoring_run_id, chapter_number, scene_order),
    FOREIGN KEY (authoring_run_id, chapter_number) REFERENCES authoring_run_chapter(authoring_run_id, chapter_number) ON DELETE CASCADE
);

CREATE TABLE authoring_checkpoint (
    authoring_run_id            TEXT NOT NULL REFERENCES authoring_run(id) ON DELETE CASCADE,
    start_chapter               INTEGER NOT NULL,
    end_chapter                 INTEGER NOT NULL,
    save_point_id               TEXT NOT NULL,
    status                      TEXT NOT NULL, -- "pending_review", "reviewed"
    report_artifact_path        TEXT,
    PRIMARY KEY (authoring_run_id, start_chapter, end_chapter)
);
