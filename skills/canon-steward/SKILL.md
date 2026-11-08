---
name: canon-steward
description: >
  Use when reviewing, ratifying, or applying canon deltas mined from committed scenes or plan
  amendments proposed for future chapters. Covers mining scenes into staged proposals, inspecting
  the ratify queue, judging verbatim evidence, editing payloads, and applying or rejecting deltas.
---

# Canon Steward

Committed scenes can be mined into *proposed* canon deltas — typed, per-class change proposals grounded in verbatim prose quotes — that the operator ratifies before they touch the Story Bible. Instead of hand-authoring `register_canonical_fact`, `update_relationship`, or `commit_character_state` calls after every scene, you review a diff. Each delta class maps to an existing write tool (`docs/adr/0001-canon-delta-classes.md`).

Canon commits through an inherited scene validate and write against the active descendant branch's facts and overrides. Relationship shifts replace that scene's prior contribution; save points preserve relationship totals with the per-scene delta ledger. Inherited fact replacements leave parent canon unchanged until merge, and voice-profile writes require a character owned by the active branch.

## Independent Extraction Coverage

When an independent extraction audit is provided, treat its claims as shadow proposals. Inspect each family's `inspected_none`, `claims_present`, or `unknown` status and the evidence bound to the exact accepted-text hash. Unknown coverage and extraction failure remain pending work; legacy package projection does not supply an independent audit, and drafter notes or raw model confidence alone never ratify a claim.

## Workflow

### 1. Mine a Committed Scene

Call `set_active_project` once per session, then mine a committed scene with `mine_scene_canon` (`project_id`, `scene_id`). One rating-gated model call stages proposed deltas (`status`: `staged`, `skipped` with `skip_reason`, or `model_output_rejected`). Re-mining a scene supersedes prior `staged` deltas while preserving already `applied` or `rejected` rows.

### 2. Read the Ratify Queue

List staged proposals with `list_canon_deltas` (`status: "staged"`, optionally filtered by `scene_id` or `chapter_range` — mutually exclusive). Each delta includes `delta_class`, typed `payload`, verbatim `evidence` quote (<=300 chars), `confidence` (`high | medium | low`), `target_id`, and `status`.

### 3. Judge Each Proposal Against Verbatim Evidence

Never bulk-apply without reading the evidence:

- Verify the `evidence` quote genuinely establishes the proposed fact, relationship shift, or promise payoff.
- Cross-check `target_id` against existing Bible records with `get_entity`, `search_bible`, `get_scene_context`, or `find_scenes_referencing`.
- **Secret-link scrutiny**: Any `knowledge_learned` delta carrying `secret_of_fact_id` expands a secret's circle of trust. Circle expansion is always a human decision and never auto-accepted. Verify the referenced fact is a genuine secret and that the scene intentionally reveals it at this story position before applying.

### 4. Decide (Apply or Reject, with Optional Edit)

Submit decisions via `decide_canon_deltas` (`decisions: [{ delta_id, action: "apply" | "reject", edit, note }]`):

- **All-or-nothing pre-flight**: Every decision is validated before any write (delta exists, is `staged`, and payload deserializes with valid entity references). A pre-flight error aborts the entire call with zero writes.
- **Finality**: Decided rows (`applied`, `rejected`, `superseded`) cannot be re-decided. Correct an applied fact forward using `register_canonical_fact` with `supersedes_fact_id`.
- **Edit-before-apply**: Supplying `edit` replaces the staged payload and records the corrected payload on the row.
- **Durable notes**: Decision `note` strings are echoed in output; use `record_note` when rationale must persist in the Bible.
- **Mid-apply honesty**: If a write error occurs after pre-flight, earlier applies remain committed, the failing row reports `failed` (staying `staged`), and subsequent rows report `not_reached` (staying `staged`).

## Plan Amendments (Living Outline)

When a run uses `replan_policy: "propose_all"`, `replan_chapter` stages proposed outline edits for future undrafted chapters. Inspect them with `list_plan_amendments` and ratify or reject them with `decide_plan_amendments`:

- Judge each amendment's `rationale` against summarized chapter reality.
- **Immutability guard**: Amendments targeting chapters that already have drafted scenes are rejected.
- Applying an amendment snapshots `prior_state` and increments `plan_revision` (`docs/adr/0003-plan-amendment-classes.md`).

## Read-Only Subagent Review

When reviewing deltas across multiple scenes, subagents may run read-only evidence checks (`list_canon_deltas`, `get_entity`, `search_bible`, `get_scene_context`, `find_scenes_referencing`) and return per-delta recommendations. Subagents never call `decide_canon_deltas`, `decide_plan_amendments`, `mine_scene_canon`, `replan_chapter`, or any state-mutating tool.

<!-- tool: commit_character_state -->
<!-- tool: decide_canon_deltas -->
<!-- tool: decide_plan_amendments -->
<!-- tool: find_scenes_referencing -->
<!-- tool: get_entity -->
<!-- tool: get_scene_context -->
<!-- tool: list_canon_deltas -->
<!-- tool: list_plan_amendments -->
<!-- tool: mine_scene_canon -->
<!-- tool: record_note -->
<!-- tool: register_canonical_fact -->
<!-- tool: replan_chapter -->
<!-- tool: search_bible -->
<!-- tool: set_active_project -->
<!-- tool: update_relationship -->
