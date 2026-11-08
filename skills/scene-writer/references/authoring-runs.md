# Supervised Authoring Runs and Canon Mining

Load this reference when drafting a scene inside a supervised multi-chapter authoring run (`authoring-supervisor`) or handing off post-commit canon mining (`canon-steward`).

## 1. Host Drafting Inside an Authoring Run

During a supervised run in `hybrid` or `client_writes` mode, `authoring_execute_next` pauses at the scene drafting step and returns the assembled context packet for the host assistant to write:

1. **Draft immediately**: A host-draft instruction means write and save the scene now using the returned context packet. Do not stop to ask permission for routine drafting inside an authorized chapter range.
2. **Save with continuity package**: Call `authoring_save_scene_draft` (instead of standalone `save_scene_draft`) with the complete prose and its structured continuity package (`character_states`, `relationship_updates`, `canonical_facts`, `knowledge_learned` including any `secret_of_fact_id`, `promise_updates`, `beats`, and optional `scene_clock`).
3. **Advance the run**: After `authoring_save_scene_draft` succeeds and any immediate style or consistency warnings are resolved, call `authoring_execute_next` to advance through post-save verification, chapter summary, and checkpoint gates. Use `authoring_status` to inspect run phase, pending checkpoints, or blockers after a reconnect.

## 2. Post-Commit Canon Mining Handoff

When an authoring run enables `mining_policy = "propose_all"` (or when mining a completed scene manually):

1. Call `mine_scene_canon` to extract candidate canon deltas from the accepted scene prose without auto-mutating the Story Bible.
2. Inspect staged proposals with `list_canon_deltas` and ratify or reject them with `decide_canon_deltas` (or hand off to the `canon-steward` workflow for full review).

<!-- tool: authoring_execute_next -->
<!-- tool: authoring_save_scene_draft -->
<!-- tool: authoring_status -->
<!-- tool: save_scene_draft -->
<!-- tool: decide_canon_deltas -->
<!-- tool: list_canon_deltas -->
<!-- tool: mine_scene_canon -->
