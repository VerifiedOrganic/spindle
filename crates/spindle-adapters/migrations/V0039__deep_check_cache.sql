-- =============================================================================
-- V0039: per-scene cache for the model-backed (Tier 2) deep-check passes.
--
-- Numbered V0039, not V0035: HEAD already shipped V0036 (min scene word
-- count). Refinery abort_missing refuses a versioned file whose version is
-- below the last applied version but is not itself applied, so inserting
-- V0035 after V0036 would prevent any already-migrated workspace from
-- opening. Fresh databases apply 36 → 37 → 38 → 39 in order.
--
-- THE PROBLEM: `check_consistency` with `deep_check: true` runs five
-- model-backed tiers (world_rule_compliance, temporal_coherence,
-- promise_payoff, scene_purpose, secret_behavioral_leak), each looping ONE
-- model call per scene. Over a chapter range that is 5 x N sequential calls.
-- With a reasoning model at 30-60s per call, a five-chapter range is tens of
-- minutes — far past any MCP client's request timeout. Worse, the audit held
-- everything in memory and returned it only at the very end, so a client
-- timeout discarded every completed scene's analysis and the retry restarted
-- from zero. The operator paid for the same tokens again and again and could
-- never reach the end.
--
-- THE FIX: cache the model's RAW OUTPUT per (scene, prose version, tier). A
-- retry then re-parses the cached text for every scene already analyzed and
-- only calls the model for the ones still missing, so a long deep check makes
-- forward progress across retries instead of restarting. Combined with the
-- bounded-concurrency fan-out in the service layer, a range that never
-- finished now converges.
--
-- WHY CACHE RAW OUTPUT RATHER THAN PARSED FINDINGS: the five tiers each parse
-- into a different finding shape, and their parsers are where the
-- issue-construction logic (severity defaults, evidence formatting, suggested
-- actions) lives. Caching the pre-parse text keeps ONE uniform cache for all
-- five tiers and lets a parser improvement take effect on the next run instead
-- of being frozen into stored rows. The model call is the expensive part; the
-- parse is free.
--
-- WHY KEYED ON scene_revision_fingerprint: the cache must never outlive the
-- prose it describes. The fingerprint is the same scene-revision token the
-- dual-persona review uses, so editing a scene naturally misses the cache and
-- re-analyzes, while a re-run over untouched prose hits it. No invalidation
-- pass and no staleness window: a stale key is simply never read.
--
--   * scene_id                   — the analyzed scene. FK with ON DELETE
--     CASCADE: a deleted scene's cached analysis is meaningless, and unlike a
--     generation receipt this is derived data with no audit value.
--   * scene_revision_fingerprint — the prose version analyzed. Part of the key.
--   * check_type                 — which deep tier produced the output, so the
--     five tiers never read each other's text.
--   * output_text                — the model's raw response, parsed on read by
--     whichever tier owns `check_type`.
--   * created_at                 — INTEGER unix microseconds (house convention).
--
-- Additive and optional: a database created before V0039 upgrades with zero
-- rows, and every deep check simply misses the cache on its first pass, which
-- is exactly the pre-V0039 behavior. This is a NEW table only.
-- =============================================================================

CREATE TABLE deep_check_cache (
    scene_id                   TEXT    NOT NULL REFERENCES scene(id) ON DELETE CASCADE,
    scene_revision_fingerprint TEXT    NOT NULL,
    check_type                 TEXT    NOT NULL,
    output_text                TEXT    NOT NULL,
    created_at                 INTEGER NOT NULL,
    PRIMARY KEY (scene_id, scene_revision_fingerprint, check_type)
);

-- Sweeping superseded entries when a scene is edited ("delete every cached row
-- for this scene that is not the current fingerprint") scans by scene_id.
CREATE INDEX idx_deep_check_cache_scene ON deep_check_cache(scene_id);
