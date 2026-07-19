-- =============================================================================
-- V0025: authoring-run canon-mining opt-in (evolution §3.1, P1 run integration).
--
-- Adds the per-run mining policy and the per-scene mining outcome to the
-- authoring run tables. All three columns are NULLable additions — a database
-- created before V0025 (any V0024-era run) upgrades cleanly and every existing
-- run row deserializes byte-identically, because NULL is the disabled/default
-- state:
--
--   * authoring_run.mining_policy — NULL (pre-upgrade + default) = mining
--     disabled; the run's scheduler behaves exactly as before. Only the string
--     'propose_all' opts a run into the MineScene step between commit and beats.
--     Validated app-side at authoring_start_run against {disabled, propose_all};
--     not a DB CHECK, so the policy vocabulary can grow without a migration.
--
--   * authoring_run_scene.mine_status — NULL = mining not attempted for this
--     scene. Otherwise the recorded outcome: 'staged' | 'skipped' |
--     'model_output_rejected' | 'error'. A Some(_) value is what the scheduler
--     treats as "mining done" so the pass fires at most once per scene, and it
--     is what the run-status render reports honestly (evolution I8) — a skip
--     never reads as a clean mine.
--
--   * authoring_run_scene.mine_detail — human-readable detail for the outcome
--     (staged delta count or the skip/error reason). Never carries prose.
--
-- Mirrors V0010's authoring_run_scene ALTER additions (research fields): plain
-- NULLable ADD COLUMNs, threaded through the records/repository read+write.
-- =============================================================================

ALTER TABLE authoring_run ADD COLUMN mining_policy TEXT;

ALTER TABLE authoring_run_scene ADD COLUMN mine_status TEXT;
ALTER TABLE authoring_run_scene ADD COLUMN mine_detail TEXT;
