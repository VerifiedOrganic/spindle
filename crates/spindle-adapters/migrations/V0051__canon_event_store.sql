-- =============================================================================
-- V0051: Append-only canon claims, decisions, typed events, source mappings,
-- and rebuildable projection/checkpoint tables (P2 PR C).
--
-- Purely additive: legacy tables and authoring paths remain untouched until
-- Stage D/E backfill and cutover.
-- =============================================================================

CREATE TABLE canon_claim (
    id                   TEXT    PRIMARY KEY NOT NULL CHECK(id LIKE 'canon_claim:%'),
    project_id           TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id            TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    scene_id             TEXT,
    revision_id          TEXT    NOT NULL,
    source               TEXT    NOT NULL CHECK(source IN ('independent', 'drafter')),
    semantic_fingerprint TEXT    NOT NULL,
    status               TEXT    NOT NULL CHECK(status IN (
        'provisional', 'ratified', 'rejected', 'withdrawn', 'source_invalidated'
    )),
    tier                 TEXT    NOT NULL CHECK(tier IN ('ratified', 'provisional')),
    provenance           TEXT    NOT NULL CHECK(provenance IN (
        'extracted', 'drafter_package', 'manual_author',
        'legacy_unverified', 'merged', 'system_restoration'
    )),
    data                 TEXT    NOT NULL CHECK(json_valid(data)),
    created_at           INTEGER NOT NULL
);

CREATE INDEX idx_canon_claim_branch_revision
    ON canon_claim(project_id, branch_id, scene_id, revision_id, created_at);

CREATE INDEX idx_canon_claim_fingerprint
    ON canon_claim(project_id, branch_id, semantic_fingerprint);

CREATE TABLE canon_decision (
    decision_seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    id                   TEXT    NOT NULL UNIQUE CHECK(id LIKE 'canon_decision:%'),
    project_id           TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id            TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    action               TEXT    NOT NULL CHECK(action IN (
        'stage_provisional', 'ratify', 'reject', 'withdraw', 'supersede',
        'invalidate_source', 'branch_mask', 'scene_visibility',
        'restore_watermark', 'merge_accept'
    )),
    actor                TEXT    NOT NULL,
    policy_version       TEXT    NOT NULL,
    source_revision_id   TEXT    NOT NULL,
    semantic_fingerprint TEXT    NOT NULL,
    idempotency_key      TEXT    NOT NULL UNIQUE,
    projection_watermark INTEGER NOT NULL,
    previous_decision_id TEXT,
    target_scene_id      TEXT,
    reason               TEXT    NOT NULL,
    data                 TEXT    NOT NULL CHECK(json_valid(data)),
    created_at           INTEGER NOT NULL,
    UNIQUE (branch_id, source_revision_id, semantic_fingerprint, action)
);

CREATE INDEX idx_canon_decision_branch_seq
    ON canon_decision(project_id, branch_id, decision_seq);

CREATE TABLE canon_event (
    append_seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    id                   TEXT    NOT NULL UNIQUE CHECK(id LIKE 'canon_event:%'),
    project_id           TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id            TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    decision_id          TEXT    NOT NULL REFERENCES canon_decision(id) ON DELETE CASCADE,
    decision_seq         INTEGER NOT NULL,
    tier                 TEXT    NOT NULL CHECK(tier IN ('ratified', 'provisional')),
    policy_action        TEXT    NOT NULL CHECK(policy_action IN (
        'stage_provisional', 'ratify', 'reject', 'withdraw', 'supersede',
        'invalidate_source', 'branch_mask', 'scene_visibility',
        'restore_watermark', 'merge_accept'
    )),
    subject_id           TEXT    NOT NULL,
    predicate_family     TEXT    NOT NULL CHECK(predicate_family IN (
        'life_action', 'location_clock', 'possession', 'injury', 'quantity',
        'knowledge_disclosure', 'attribute_world_state', 'relationship',
        'promise', 'rule_temporal', 'stream_control'
    )),
    predicate            TEXT    NOT NULL,
    operation            TEXT    NOT NULL,
    book_number          INTEGER,
    chapter_number       INTEGER,
    scene_order          INTEGER,
    within_scene_order   INTEGER CHECK(within_scene_order IS NULL OR within_scene_order >= 0),
    in_world_time        TEXT    CHECK(in_world_time IS NULL OR json_valid(in_world_time)),
    scene_id             TEXT,
    source_revision_id   TEXT    NOT NULL,
    source_claim_ids     TEXT    NOT NULL CHECK(json_valid(source_claim_ids)),
    semantic_fingerprint TEXT    NOT NULL,
    idempotency_key      TEXT    NOT NULL UNIQUE,
    evidence_hash        TEXT    NOT NULL,
    provenance           TEXT    NOT NULL CHECK(provenance IN (
        'extracted', 'drafter_package', 'manual_author',
        'legacy_unverified', 'merged', 'system_restoration'
    )),
    supersedes_event_id  TEXT,
    withdraws_event_id   TEXT,
    payload              TEXT    NOT NULL CHECK(json_valid(payload)),
    data                 TEXT    NOT NULL CHECK(json_valid(data)),
    created_at           INTEGER NOT NULL,
    CHECK (
        (book_number IS NULL AND chapter_number IS NULL AND scene_order IS NULL)
        OR (book_number >= 1 AND chapter_number >= 1 AND scene_order >= 1)
    ),
    UNIQUE (branch_id, source_revision_id, semantic_fingerprint, policy_action)
);

CREATE INDEX idx_canon_event_stream_position
    ON canon_event(
        project_id,
        branch_id,
        subject_id,
        predicate,
        book_number,
        chapter_number,
        scene_order,
        within_scene_order,
        append_seq
    );

CREATE INDEX idx_canon_event_branch_seq
    ON canon_event(project_id, branch_id, append_seq);

CREATE INDEX idx_canon_event_scene_revision
    ON canon_event(project_id, branch_id, scene_id, source_revision_id);

CREATE TABLE canon_source_map (
    project_id         TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id          TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    source_table       TEXT    NOT NULL,
    legacy_primary_key TEXT    NOT NULL,
    migration_version  INTEGER NOT NULL CHECK(migration_version >= 1),
    status             TEXT    NOT NULL CHECK(status IN ('mapped', 'quarantined', 'duplicate_skipped')),
    provenance         TEXT    NOT NULL CHECK(provenance IN (
        'extracted', 'drafter_package', 'manual_author',
        'legacy_unverified', 'merged', 'system_restoration'
    )),
    claim_ids          TEXT    NOT NULL CHECK(json_valid(claim_ids)),
    event_ids          TEXT    NOT NULL CHECK(json_valid(event_ids)),
    decision_id        TEXT,
    quarantine_reason  TEXT,
    raw_snapshot       TEXT    NOT NULL CHECK(json_valid(raw_snapshot)),
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    PRIMARY KEY (project_id, source_table, legacy_primary_key, migration_version)
);

CREATE INDEX idx_canon_source_map_branch
    ON canon_source_map(project_id, branch_id, source_table, status);

CREATE TABLE canon_projection_state (
    project_id         TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id          TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    tier               TEXT    NOT NULL CHECK(tier IN ('ratified', 'provisional')),
    projection_kind    TEXT    NOT NULL CHECK(projection_kind IN (
        'entity_state', 'relationship', 'quantity', 'knowledge',
        'rule_constraint', 'timeline_clock', 'open_thread'
    )),
    stream_key         TEXT    NOT NULL,
    subject_id         TEXT    NOT NULL,
    decision_watermark INTEGER NOT NULL,
    event_watermark    INTEGER NOT NULL,
    reducer_version    INTEGER NOT NULL,
    source_event_ids   TEXT    NOT NULL CHECK(json_valid(source_event_ids)),
    data               TEXT    NOT NULL CHECK(json_valid(data)),
    updated_at         INTEGER NOT NULL,
    PRIMARY KEY (project_id, branch_id, tier, projection_kind, stream_key)
);

CREATE TABLE canon_projection_checkpoint (
    id                 TEXT    PRIMARY KEY NOT NULL CHECK(id LIKE 'canon_checkpoint:%'),
    project_id         TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id          TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    save_point_id      TEXT    REFERENCES save_point(id) ON DELETE SET NULL,
    tier               TEXT    NOT NULL CHECK(tier IN ('ratified', 'provisional')),
    decision_watermark INTEGER NOT NULL,
    event_watermark    INTEGER NOT NULL,
    reducer_version    INTEGER NOT NULL,
    schema_version     INTEGER NOT NULL,
    lineage            TEXT    NOT NULL CHECK(json_valid(lineage)),
    branch_watermarks  TEXT    NOT NULL CHECK(json_valid(branch_watermarks)),
    state_snapshot     TEXT    NOT NULL CHECK(json_valid(state_snapshot)),
    created_at         INTEGER NOT NULL
);

CREATE INDEX idx_canon_projection_checkpoint_branch
    ON canon_projection_checkpoint(project_id, branch_id, save_point_id, created_at);
