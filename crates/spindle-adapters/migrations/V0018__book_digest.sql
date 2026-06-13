-- =============================================================================
-- V0018: per-book "story so far" digest.
--
-- Auto-maintained (deterministically) whenever a chapter summary is saved, so a
-- later book's drafting context can surface a compressed synopsis of prior
-- books instead of relying on the recency-limited chapter summaries. One row per
-- (project, branch, book).
-- =============================================================================

CREATE TABLE book_digest (
    id                   TEXT PRIMARY KEY,
    project_id           TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id            TEXT NOT NULL,
    book_number          INTEGER NOT NULL,
    synopsis             TEXT NOT NULL,
    open_threads         TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(open_threads)),
    last_chapter_covered INTEGER NOT NULL DEFAULT 0,
    token_estimate       INTEGER NOT NULL DEFAULT 0,
    truncated            INTEGER NOT NULL DEFAULT 0,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    UNIQUE(project_id, branch_id, book_number)
);

CREATE INDEX idx_book_digest_project_branch
    ON book_digest(project_id, branch_id, book_number);
