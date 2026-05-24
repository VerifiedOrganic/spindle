-- =============================================================================
-- V0006: add validator input context hashes to cached findings.
--
-- A Phase-4 cache hit must match both the scene text hash and the continuity
-- metadata snapshot each validator reads. Without this column, a missed
-- explicit invalidation path can serve stale rows when facts, rules, voice
-- profiles, timeline events, interventions, or style directives change.
-- =============================================================================

ALTER TABLE validator_finding ADD COLUMN context_hash TEXT;

CREATE INDEX validator_finding_scene_validator_context_hash
    ON validator_finding(branch_id, scene_id, validator_id, scene_text_hash, context_hash);
