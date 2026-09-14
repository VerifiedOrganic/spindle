-- =============================================================================
-- V0032: generation receipts persisted with a TTL (live-run bug 4c).
--
-- A generation receipt records ONE `continue_generation` / `revise_generation`
-- / draft-route `test_agent` model output so a later `save_scene_draft` or
-- `revise_generation` can resolve a `generation_id` back to its provenance
-- (route, rating, producing agent) without re-running the model.
--
-- WHY PERSIST (the bug): receipts used to live only in a process-local
-- in-memory map on the service instance. When the primary MCP process exited
-- and a fresh process took over (the observed primary/proxy churn — bug 4a/4b),
-- every previously-issued `generation_id` evaporated, and the very next
-- explicit `save_scene_draft` failed with
-- `generation_id "…" was not found or has expired`. That receipt loss is what
-- forced the receipt-stub data-loss path (bugs 1 + 6b, now fixed). Persisting
-- the receipt here lets a receipt registered in process 1 be verified in
-- process 2 over the same database file. The in-memory cache is kept as a fast
-- read-through front; the DB is the source of truth across restarts.
--
-- WHY A TTL: the in-memory version bounded growth by evicting FIFO past a cap
-- (MAX_GENERATION_RECEIPTS = 256) but carried no wall-clock expiry — a receipt
-- lived until the process died. Persisted receipts outlive the process, so an
-- explicit time bound replaces the process lifetime as the natural GC. Default
-- TTL is 24h (GENERATION_RECEIPT_TTL, doc-commented in service.rs): long enough
-- for any realistic draft→revise→save loop across a primary restart, short
-- enough that abandoned receipts do not accumulate. Expiry is enforced ON READ
-- (`expires_at <= now` → the `was not found or has expired` error, message
-- preserved byte-for-byte), and expired rows are deleted lazily on read — no
-- background task.
--
-- WHY output_text IS STORED WHOLE (the storage decision): the wave-A save fix
-- made `output_text` NON-load-bearing on the SAVE path (the caller's full_text
-- is authoritative; the receipt only proves clearance + provenance). A content
-- hash + short head would suffice there. BUT `revise_generation` still feeds
-- `receipt.output_text` verbatim into the revision prompt
-- (build_generation_revision_prompt). If we stored only a hash+head, a
-- revise-after-restart would revise a TRUNCATED fragment — silent corruption.
-- So we store the full `output_text` (the honest, correct choice) AND a
-- `output_sha256` alongside it for provenance/debug and cheap integrity checks.
-- Scenes already store whole prose in this DB, so a receipt's text is not a new
-- storage class.
--
-- Additive and optional: a database created before V0032 upgrades cleanly with
-- zero rows; a deployment that never issues a `generation_id` is entirely
-- unaffected. This is a NEW table only — no existing column changes.
--
--   * id             — the `generation_id` itself
--     (`model_generation:{seq}:{12-char-sha-prefix}`), the caller-facing token.
--     PRIMARY KEY. Prefix-CHECKed like every sibling id column.
--   * project_id     — optional provenance (the project the generation served,
--     when known). NO FK: receipts are not project-scoped state and must survive
--     a project delete for audit; NULL when the producing call had no project.
--   * branch_id      — optional provenance, same rationale as project_id.
--   * route          — the model route (`draft`, `mine`, …) the output came
--     from. The revise/save gate requires `route = 'draft'`.
--   * agent_id       — the producing agent/model name (stamped as
--     `agent:<agent_id>` provenance on the saved scene).
--   * rating         — the content rating the output was produced at
--     (general|teen|mature|explicit), lowercased; NULL = default/unspecified.
--   * explicit_capable — 1 if the producing agent declares the `explicit`
--     rating, else 0. The explicit-integrity gate on revise/save reads this.
--   * output_sha256  — hex SHA-256 of the normalized output_text (provenance +
--     cheap integrity check; also surfaced to callers as
--     generation_output_sha256).
--   * output_text    — the FULL normalized model output (see storage decision
--     above). Still load-bearing for revise_generation.
--   * created_at     — INTEGER unix microseconds (row::time / timestamp_to_micros
--     house convention), matching every sibling table.
--   * expires_at     — INTEGER unix microseconds. `expires_at <= now` on read =
--     expired (deleted lazily, reported as not-found/expired).
-- =============================================================================

CREATE TABLE generation_receipt (
    id                TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'model_generation:%'),
    project_id        TEXT,
    branch_id         TEXT,
    route             TEXT    NOT NULL,
    agent_id          TEXT    NOT NULL,
    rating            TEXT,
    explicit_capable  INTEGER NOT NULL DEFAULT 0 CHECK (explicit_capable IN (0, 1)),
    output_sha256     TEXT    NOT NULL,
    output_text       TEXT    NOT NULL,
    created_at        INTEGER NOT NULL,
    expires_at        INTEGER NOT NULL
);

-- Drives the lazy expiry sweep on read: "delete every receipt whose expires_at
-- has passed" (WHERE expires_at <= ?now). An index on expires_at keeps that
-- sweep from scanning the whole table.
CREATE INDEX idx_generation_receipt_expires_at
    ON generation_receipt(expires_at);
