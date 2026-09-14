CREATE TABLE model_call (
    id TEXT PRIMARY KEY,
    project_id TEXT,
    recorded_at INTEGER NOT NULL,
    payload TEXT NOT NULL CHECK (json_valid(payload))
);
CREATE INDEX model_call_project_time ON model_call(project_id, recorded_at);
