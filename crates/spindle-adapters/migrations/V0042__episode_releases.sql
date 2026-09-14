CREATE TABLE episode_release (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    book_number INTEGER NOT NULL,
    chapter_number INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    source_hash TEXT NOT NULL,
    payload TEXT NOT NULL CHECK (json_valid(payload)),
    UNIQUE (project_id, book_number, chapter_number, revision)
);
CREATE TRIGGER episode_release_immutable BEFORE UPDATE ON episode_release
BEGIN SELECT RAISE(ABORT, 'episode releases are immutable; append a correction revision'); END;
