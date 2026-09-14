CREATE TABLE reader_memory (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id TEXT NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    book_number INTEGER NOT NULL,
    chapter_number INTEGER NOT NULL,
    payload TEXT NOT NULL CHECK (json_valid(payload)),
    UNIQUE(project_id, branch_id, book_number, chapter_number)
);
