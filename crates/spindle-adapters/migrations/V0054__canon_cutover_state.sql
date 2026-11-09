-- =============================================================================
-- V0053: Per-project canon cutover mode (P2 PR E).
--
-- Legacy is authoritative by default. Dual-write, shadow-read and canon-read
-- may only be entered after the project's compatibility/quarantine readiness
-- gate passes; rollback to legacy/shadow-read is always allowed.
-- =============================================================================

CREATE TABLE canon_cutover_state (
    project_id                   TEXT    PRIMARY KEY NOT NULL
                                         REFERENCES project(id) ON DELETE CASCADE,
    mode                         TEXT    NOT NULL CHECK(mode IN (
        'legacy', 'dual_write', 'shadow_read', 'canon_read'
    )),
    activated_decision_watermark INTEGER NOT NULL,
    reason                       TEXT    NOT NULL,
    updated_at                   INTEGER NOT NULL
);
