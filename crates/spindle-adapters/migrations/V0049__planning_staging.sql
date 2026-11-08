-- Initial planning is separate from ADR 0003 amendments. No domain table changes.
CREATE TABLE planning_session (
 id TEXT PRIMARY KEY NOT NULL CHECK(id LIKE 'planning_session:%'),
 project_id TEXT REFERENCES project(id) ON DELETE CASCADE,
 branch_id TEXT REFERENCES bible_branch(id) ON DELETE CASCADE,
 data TEXT NOT NULL CHECK(json_valid(data)),
 created_at INTEGER NOT NULL,
 CHECK ((project_id IS NULL) = (branch_id IS NULL))
);
CREATE TABLE planning_proposal (
 id TEXT PRIMARY KEY NOT NULL CHECK(id LIKE 'planning_proposal:%'),
 session_id TEXT NOT NULL REFERENCES planning_session(id) ON DELETE CASCADE,
 payload_hash TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('staged','applied','rejected','superseded')),
 data TEXT NOT NULL CHECK(json_valid(data)),
 created_at INTEGER NOT NULL
);
CREATE INDEX idx_planning_proposal_session ON planning_proposal(session_id, status, created_at, id);
-- Reserved for Phase B's atomic domain-write/receipt implementation. A staged
-- proposal or a decision alone must never populate successful operation rows.
CREATE TABLE planning_operation_receipt (
 proposal_id TEXT NOT NULL REFERENCES planning_proposal(id) ON DELETE CASCADE,
 operation_index INTEGER NOT NULL CHECK(operation_index >= 0),
 input_hash TEXT NOT NULL,
 returned_ids TEXT NOT NULL CHECK(json_valid(returned_ids)),
 prior_state TEXT CHECK(prior_state IS NULL OR json_valid(prior_state)),
 outcome TEXT NOT NULL CHECK(outcome IN ('applied','failed')),
 recorded_at INTEGER NOT NULL,
 PRIMARY KEY(proposal_id, operation_index)
);
