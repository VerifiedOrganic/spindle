-- =============================================================================
-- V0023: secret-knowledge gating (circle-of-trust) — Part A schema.
--
-- See docs/secret-knowledge-gating-design.md §3. Every column is ADDITIVE and
-- DEFAULTED/nullable so existing rows keep deserializing and a project that
-- never marks a secret behaves byte-identically to before this migration.
--
--   * canonical_fact.secret — 0/1 flag marking a fact as held in confidence.
--     Default 0 (public). Existing rows read as secret=false.
--   * canonical_fact.concealment_note — optional drafting guidance rendered
--     into the [SECRETS IN PLAY] envelope (Part B). Nullable, default NULL.
--   * knowledge_fact.secret_of_fact_id — nullable link from a per-character
--     knowledge row back to the secret canonical fact it grants circle
--     membership in. The circle is thereafter DERIVED, never duplicated:
--     circle(fact) = characters with a knowledge_fact row where
--     secret_of_fact_id = fact.id. References canonical_fact(id); existing
--     rows read as NULL (no secret linkage).
--
-- The FK REFERENCES clause on an ALTER ... ADD COLUMN follows the precedent set
-- by V0021 (scene.location_id REFERENCES location(id)); SQLite allows it on a
-- nullable column with a NULL default. No ON DELETE action is specified so the
-- default (NO ACTION) applies — a link is only ever removed by the app when the
-- knowledge row is deleted, so a dangling FK cannot arise in normal operation.
--
-- The partial index mirrors the design's exact form and keeps the circle-
-- derivation join (WHERE secret_of_fact_id = ?) selective without indexing the
-- overwhelmingly-NULL majority of knowledge_fact rows. SQLite (bundled) supports
-- partial indexes.
-- =============================================================================

ALTER TABLE canonical_fact ADD COLUMN secret INTEGER NOT NULL DEFAULT 0;
ALTER TABLE canonical_fact ADD COLUMN concealment_note TEXT;
ALTER TABLE knowledge_fact ADD COLUMN secret_of_fact_id TEXT REFERENCES canonical_fact(id);

CREATE INDEX idx_knowledge_fact_secret_link ON knowledge_fact(secret_of_fact_id)
    WHERE secret_of_fact_id IS NOT NULL;
