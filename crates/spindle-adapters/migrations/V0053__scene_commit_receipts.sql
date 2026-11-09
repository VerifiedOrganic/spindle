-- V0053: durable idempotency receipts for the atomic `commit_scene` boundary.
--
-- A client-generated idempotency key identifies one accepted commit payload.
-- Replaying the same key with the same payload returns the stored compact
-- receipt; reusing the key with a different payload is rejected. The receipt is
-- written in the same SQLite transaction as the prose and its approved derived
-- state, so a failed commit never claims a key.
CREATE TABLE scene_commit_receipt (
    operation_id    TEXT    PRIMARY KEY,
    project_id      TEXT    NOT NULL,
    branch_id       TEXT    NOT NULL,
    scene_id        TEXT    NOT NULL,
    idempotency_key TEXT    NOT NULL UNIQUE,
    payload_hash    TEXT    NOT NULL,
    receipt_json    TEXT    NOT NULL CHECK (json_valid(receipt_json)),
    created_at      INTEGER NOT NULL
);

CREATE INDEX idx_scene_commit_receipt_scene
    ON scene_commit_receipt (scene_id, created_at);
