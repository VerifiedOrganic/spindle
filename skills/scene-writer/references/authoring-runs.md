# Supervised Authoring Runs and Canon Mining

Load this reference when drafting a scene inside a supervised multi-chapter authoring run (`authoring-supervisor`) or handing off post-commit canon mining (`canon-steward`).

## 1. Host Drafting Inside an Authoring Run

During a supervised run in default `hybrid` mode (`mode: "hybrid"`), `authoring_execute_next` pauses at the scene drafting step (`draft book scene X.Y`) and returns a host-draft instruction:

1. **Draft immediately**: A host-draft instruction means write and save the scene now using the context packet. Do not stop to ask permission for routine drafting inside an authorized chapter range.
2. **Save with continuity package**: Call `authoring_save_scene_draft` (instead of standalone `save_scene_draft`) with `authorship: "assistant"`, the complete prose, and its structured continuity package (`character_states`, `canonical_facts`, `relationship_updates`, `beats`, `continuity_notes`, and `knowledge_learned` including any `secret_of_fact_id`). If the scene introduces no durable canon changes, include a `continuity_notes` entry stating that explicitly.
3. **Advance the run**: After `authoring_save_scene_draft` succeeds and any immediate style or consistency warnings are resolved, call `authoring_execute_next` to advance through post-save verification, commit, beat annotation, chapter summary, and checkpoint steps. Use `authoring_status` to inspect run phase, pending checkpoints, or blockers after a reconnect.

## 2. Post-Commit Canon Mining Handoff

When an authoring run enables `mining_policy: "propose_all"` (or when mining a committed scene manually):

1. Call `mine_scene_canon` to extract candidate canon deltas from the accepted scene prose without auto-mutating the Story Bible.
2. Inspect staged proposals with `list_canon_deltas` (`status: "staged"`) and adjudicate decided items with `decide_canon_deltas` using `action: "apply"` or `"reject"` (leave deferred proposals `staged` without submitting a decision, or hand off to the `canon-steward` workflow for full review).

<!-- tool: authoring_execute_next -->
<!-- tool: authoring_save_scene_draft -->
<!-- tool: authoring_status -->
<!-- tool: save_scene_draft -->
<!-- tool: decide_canon_deltas -->
<!-- tool: list_canon_deltas -->
<!-- tool: mine_scene_canon -->
