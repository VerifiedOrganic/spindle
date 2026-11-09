-- =============================================================================
-- V0055: Layered derived memory (scene -> chapter -> arc -> book), dependency
-- edges, and durable lazy-recompute jobs (P2 PR G).
--
-- Summaries are derived memory only; they never establish canon. Freshness is
-- explicit so a stale summary can be shown as stale or omitted, never silently
-- treated as clean.
-- =============================================================================

CREATE TABLE memory_summary (
    id                  TEXT    PRIMARY KEY NOT NULL CHECK(id LIKE 'memory_summary:%'),
    project_id          TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id           TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    layer               TEXT    NOT NULL CHECK(layer IN ('scene', 'chapter', 'arc', 'book')),
    scope_key           TEXT    NOT NULL,
    book_number         INTEGER NOT NULL,
    chapter_number      INTEGER NOT NULL,
    scene_order         INTEGER NOT NULL,
    covered_end_book    INTEGER NOT NULL,
    covered_end_chapter INTEGER NOT NULL,
    covered_end_scene   INTEGER NOT NULL,
    status              TEXT    NOT NULL CHECK(status IN ('fresh', 'stale', 'recomputing')),
    updated_at          INTEGER NOT NULL,
    data                TEXT    NOT NULL CHECK(json_valid(data)),
    UNIQUE (project_id, branch_id, layer, scope_key)
);

CREATE INDEX idx_memory_summary_cursor
    ON memory_summary(
        project_id, branch_id, layer,
        covered_end_book, covered_end_chapter, covered_end_scene
    );

CREATE TABLE memory_summary_dependency (
    summary_id            TEXT NOT NULL REFERENCES memory_summary(id) ON DELETE CASCADE,
    depends_on_summary_id TEXT NOT NULL REFERENCES memory_summary(id) ON DELETE CASCADE,
    source_kind           TEXT NOT NULL,
    PRIMARY KEY (summary_id, depends_on_summary_id)
);

CREATE INDEX idx_memory_summary_dependency_parent
    ON memory_summary_dependency(depends_on_summary_id);

CREATE TABLE memory_recompute_job (
    id            TEXT    PRIMARY KEY NOT NULL CHECK(id LIKE 'memory_job:%'),
    project_id    TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id     TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    summary_id    TEXT    NOT NULL,
    reason        TEXT    NOT NULL,
    dedup_key     TEXT    NOT NULL UNIQUE,
    status        TEXT    NOT NULL CHECK(status IN ('pending', 'claimed', 'done', 'failed')),
    attempts      INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT,
    next_attempt  INTEGER NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE INDEX idx_memory_recompute_job_ready
    ON memory_recompute_job(project_id, branch_id, status, next_attempt);
