-- Durable attempt summaries; per-operation receipts remain the recovery source.
CREATE TABLE planning_apply_attempt (
 id TEXT PRIMARY KEY NOT NULL,
 session_id TEXT NOT NULL REFERENCES planning_session(id) ON DELETE CASCADE,
 actor TEXT NOT NULL,
 data TEXT NOT NULL CHECK(json_valid(data)),
 created_at INTEGER NOT NULL
);
-- Search indexing is deferred outside the domain/receipt transaction. A failed
-- refresh remains queued and never reverses an accepted domain write.
CREATE TABLE planning_embedding_refresh (
 project_id TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
 branch_id TEXT NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
 entity_id TEXT NOT NULL,
 entity_table TEXT NOT NULL,
 title TEXT NOT NULL,
 excerpt TEXT NOT NULL,
 content TEXT NOT NULL,
 PRIMARY KEY(project_id,branch_id,entity_id)
);
ALTER TABLE planning_operation_receipt ADD COLUMN error TEXT;

ALTER TABLE planning_operation_receipt ADD COLUMN actor TEXT NOT NULL DEFAULT '';
ALTER TABLE planning_operation_receipt ADD COLUMN result TEXT NOT NULL DEFAULT 'null' CHECK(json_valid(result));

ALTER TABLE planning_operation_receipt ADD COLUMN decision_note TEXT NOT NULL DEFAULT '';
