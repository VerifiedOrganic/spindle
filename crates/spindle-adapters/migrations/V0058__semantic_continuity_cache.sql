-- V0058: validated semantic-continuity judge reports.
--
-- Keyed by the full invalidation identity from the 0.4 semantic design:
-- scene prose hash, relevant canon version, candidate-set hash, contract
-- version, resolved route, and model identity. Only server-validated typed
-- reports are stored; raw model prose never lands here. A stale scene, canon,
-- candidate set, route, or model simply misses the cache.
--
-- Additive and optional: existing workspaces upgrade with zero rows.

CREATE TABLE semantic_continuity_cache (
    scene_id           TEXT    NOT NULL REFERENCES scene(id) ON DELETE CASCADE,
    scene_sha256       TEXT    NOT NULL,
    canon_version      TEXT    NOT NULL,
    candidate_set_hash TEXT    NOT NULL,
    contract_version   TEXT    NOT NULL,
    route              TEXT    NOT NULL,
    model_identity     TEXT    NOT NULL,
    report_json        TEXT    NOT NULL CHECK (json_valid(report_json)),
    created_at         INTEGER NOT NULL,
    PRIMARY KEY (
        scene_id,
        scene_sha256,
        canon_version,
        candidate_set_hash,
        contract_version,
        route,
        model_identity
    )
);

CREATE INDEX idx_semantic_continuity_cache_scene
    ON semantic_continuity_cache(scene_id);
