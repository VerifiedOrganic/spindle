-- =============================================================================
-- V0026: authoring-run in-run verify/revise opt-in (evolution §3.2, P2.2).
--
-- Adds the per-run bounded-revise budget and the per-scene verify/revise state
-- to the authoring run tables. Every column is a NULLable/defaulted addition —
-- a database created before V0026 (any V0025-era run) upgrades cleanly and each
-- existing run row deserializes byte-identically, because the disabled/default
-- state is NULL / 0:
--
--   * authoring_run.max_revise_attempts — NULL (pre-upgrade + default) = 0 =
--     revise disabled; a saved draft goes straight to commit exactly as before.
--     Only 1 or 2 opts a run into the per-scene VerifyScene step after each
--     draft, feeding warning-or-worse findings back as a bounded revision.
--     Validated app-side at authoring_start_run against 0..=2; not a DB CHECK,
--     so the bound can move without a migration.
--
--   * authoring_run_scene.verify_status — NULL = verification not attempted for
--     this scene. Otherwise the recorded outcome: 'clean' | 'findings' |
--     'parked_findings' | 'error'. The scheduler reads it to decide whether to
--     verify, revise, or proceed to commit; the run-status render reports it
--     honestly (evolution I8) — a skipped/errored verify never reads clean.
--
--   * authoring_run_scene.verify_detail — human-readable detail for the outcome
--     (finding counts, the parked reason, or the error). Never carries prose.
--
--   * authoring_run_scene.revise_attempts — how many bounded revision passes the
--     scene has already consumed. NOT NULL DEFAULT 0 so existing rows read as
--     "never revised" and the bound comparison is well-defined.
--
--   * authoring_run_scene.last_finding_fingerprint — a deterministic digest of
--     the last verify's warning-or-worse finding set (convergence guard). NULL
--     until the first findings verify. If a re-verify after revision produces an
--     identical fingerprint, the scene is parked instead of re-revised, so the
--     loop can converge or stop but never oscillate on the same findings.
--
-- Mirrors V0010/V0025's authoring_run_scene ALTER additions: plain
-- NULLable/defaulted ADD COLUMNs, threaded through the records/repository
-- read+write.
-- =============================================================================

ALTER TABLE authoring_run ADD COLUMN max_revise_attempts INTEGER;

ALTER TABLE authoring_run_scene ADD COLUMN verify_status TEXT;
ALTER TABLE authoring_run_scene ADD COLUMN verify_detail TEXT;
ALTER TABLE authoring_run_scene ADD COLUMN revise_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE authoring_run_scene ADD COLUMN last_finding_fingerprint TEXT;
