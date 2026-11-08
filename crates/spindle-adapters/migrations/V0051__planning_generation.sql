-- Durable route-generation attempts, prompt/schema hashes, usage receipts,
-- validation findings and host-fallback requests. Never stores credentials.
CREATE TABLE planning_generation_attempt (
 id TEXT PRIMARY KEY NOT NULL CHECK(id LIKE 'planning_generation_attempt:%'),
 session_id TEXT NOT NULL REFERENCES planning_session(id) ON DELETE CASCADE,
 level TEXT NOT NULL,
 retry_key TEXT,
 outcome TEXT NOT NULL CHECK(outcome IN ('staged','needs_author','needs_route','model_output_rejected','budget_exhausted')),
 proposal_id TEXT REFERENCES planning_proposal(id) ON DELETE SET NULL,
 data TEXT NOT NULL CHECK(json_valid(data)),
 created_at INTEGER NOT NULL
);
CREATE INDEX idx_planning_generation_session ON planning_generation_attempt(session_id, created_at, id);
CREATE UNIQUE INDEX idx_planning_generation_retry ON planning_generation_attempt(session_id, retry_key) WHERE retry_key IS NOT NULL;
