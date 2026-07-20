-- =============================================================================
-- V0028: authoring-run checkpoint policy opt-in (evolution §3.3, P3.1/P3.2).
--
-- Adds the per-run checkpoint policy and the per-checkpoint auto-automation
-- outcome to the authoring run tables. Every column is a NULLable addition — a
-- database created before V0028 (any V0027-era run) upgrades cleanly and every
-- existing row deserializes byte-identically, because the disabled/default
-- state is NULL = manual:
--
--   * authoring_run.checkpoint_policy — NULL (pre-upgrade + default) = 'manual':
--     the classic 4-step operator checkpoint flow (deep check → record audit →
--     dual-persona reviews → review_checkpoint), byte-identical to today (I1).
--     Only 'auto_advisory' or 'auto_strict' opts a run into the in-process
--     auto-checkpoint automation (evolution §3.3): the harness runs the deep
--     consistency check, records the audit, runs sampled dual-persona reviews
--     via the `review` route, and self-clears the checkpoint on a severity
--     threshold — auto_advisory approves iff no finding is warning-or-worse,
--     auto_strict iff zero findings of any severity. Validated app-side at
--     authoring_start_run against {manual, auto_advisory, auto_strict}; not a
--     DB CHECK, so the policy vocabulary can grow without a migration.
--
--   * authoring_checkpoint.auto_outcome — NULL = manual policy, or the
--     automation has not run for this checkpoint yet. Otherwise the recorded
--     outcome of the in-process automation: 'approved' (self-cleared under
--     policy), 'blocked' (findings held it pending_review), or 'manual' (one or
--     more sampled scenes fell back to manual dual-persona review because the
--     `review` route was not rating-cleared — evolution §3.3 I3). This is what
--     authoring_status reports honestly (evolution I8) — a blocked/manual
--     checkpoint never reads as auto-approved.
--
--   * authoring_checkpoint.pending_manual_scene_ids — JSON array of the scene
--     ids whose sampled review fell back to manual (rating not covered). NULL =
--     none. Ids only — the prose of those scenes was never dispatched anywhere
--     (I3). NULL/absent reads as the empty list.
--
-- Mirrors V0025/V0026's authoring_run + child-table ALTER additions: plain
-- NULLable ADD COLUMNs, threaded through the records/repository read+write.
-- =============================================================================

ALTER TABLE authoring_run ADD COLUMN checkpoint_policy TEXT;

ALTER TABLE authoring_checkpoint ADD COLUMN auto_outcome TEXT;
ALTER TABLE authoring_checkpoint ADD COLUMN pending_manual_scene_ids TEXT;
