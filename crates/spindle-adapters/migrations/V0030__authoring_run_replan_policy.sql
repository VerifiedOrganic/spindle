-- =============================================================================
-- V0030: living-outline replan opt-in on authoring runs (ADR 0003, evolution
-- §3.5). Threads the run-level replan policy plus a per-chapter replan outcome,
-- mirroring the mining opt-in (V0025) shape exactly:
--
--   * authoring_run.replan_policy — NULL = disabled (pre-upgrade + default): the
--     run never replans, byte-identical to before. 'propose_all' runs a replan
--     pass after each chapter summary, staging amendment proposals against the
--     not-yet-drafted future chapters for operator ratification (never
--     auto-applied — ADR D5). Validated app-side against {disabled, propose_all};
--     'disabled' canonicalizes to NULL so a disabled run persists exactly as a
--     pre-upgrade row.
--   * authoring_run_chapter.replan_status — NULL = the chapter's post-summary
--     replan pass has not run (disabled run, or the summary is not yet saved);
--     otherwise 'staged' | 'skipped' | 'no_targets' | 'no_summary' | 'error'.
--   * authoring_run_chapter.replan_detail — human-readable outcome detail (staged
--     amendment count or the skip/no-targets reason). Never carries prose
--     (evolution I8) — counts / status words only.
--
-- All three are additive, NULLable ADD COLUMNs — no existing column changes — so
-- a database created before V0030 upgrades cleanly and its rows keep
-- deserializing byte-identically (NULL reads as disabled / not-attempted). The
-- classes are validated app-side (not a DB CHECK) to stay additive per the ADR
-- reversal-cost note, matching mine_status/mine_detail.
-- =============================================================================

ALTER TABLE authoring_run ADD COLUMN replan_policy TEXT;

ALTER TABLE authoring_run_chapter ADD COLUMN replan_status TEXT;

ALTER TABLE authoring_run_chapter ADD COLUMN replan_detail TEXT;
