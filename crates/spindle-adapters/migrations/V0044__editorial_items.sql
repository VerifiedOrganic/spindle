CREATE TABLE editorial_item (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id TEXT NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    dedupe_key TEXT NOT NULL UNIQUE,
    payload TEXT NOT NULL CHECK (json_valid(payload))
);
CREATE INDEX editorial_item_scope ON editorial_item(project_id, branch_id);
