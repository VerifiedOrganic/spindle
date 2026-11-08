-- =============================================================================
-- V0047: model call telemetry attribution columns.
--
-- Extends `model_call` (V0041) with structured, prose-free attribution columns
-- mirroring the extended `ModelCallRecord` JSON payload:
--   - `scene_id`            — scene associated with the call when applicable
--   - `run_id`              — authoring run id (`authoring_run:...`) when applicable
--   - `route`               — logical model route (`draft`, `review`, `mine`, ...)
--   - `prompt_template_id`  — stable prompt template identifier
--   - `prompt_version`      — stable prompt template version (e.g. `v1`)
--   - `context_packet_hash` — SHA-256 hex digest of the assembled context sent
--   - `temperature`         — sampling temperature when known
--   - `top_p`               — nucleus sampling top_p when known
--   - `max_tokens`          — max completion token cap when known
--
-- All columns are nullable so pre-V0047 rows upgrade in place without data loss.
-- Backfills `scene_id` and `route` from existing `payload` JSON on upgrade.
-- =============================================================================

ALTER TABLE model_call ADD COLUMN scene_id TEXT;
ALTER TABLE model_call ADD COLUMN run_id TEXT;
ALTER TABLE model_call ADD COLUMN route TEXT;
ALTER TABLE model_call ADD COLUMN prompt_template_id TEXT;
ALTER TABLE model_call ADD COLUMN prompt_version TEXT;
ALTER TABLE model_call ADD COLUMN context_packet_hash TEXT;
ALTER TABLE model_call ADD COLUMN temperature REAL;
ALTER TABLE model_call ADD COLUMN top_p REAL;
ALTER TABLE model_call ADD COLUMN max_tokens INTEGER;

UPDATE model_call
SET scene_id = json_extract(payload, '$.scene_id'),
    route = json_extract(payload, '$.route')
WHERE scene_id IS NULL OR route IS NULL;

CREATE INDEX idx_model_call_run_time ON model_call(run_id, recorded_at);
CREATE INDEX idx_model_call_scene_time ON model_call(scene_id, recorded_at);
CREATE INDEX idx_model_call_template_time ON model_call(prompt_template_id, prompt_version, recorded_at);
