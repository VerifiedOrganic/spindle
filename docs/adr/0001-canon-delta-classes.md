# ADR 0001 — Canon-delta classes and the staging contract

**Status:** Accepted (owner: "Do it", 2026-07-19) · **Supersede, never edit.**
**Context docs:** `docs/spindle-evolution-design.md` §3.1 (canon mining), §7 (risk register);
`docs/secret-knowledge-gating-design.md` §2.3 (reveals).

## Context

P1 inverts Spindle's bookkeeping economy: every committed scene is mined into *proposed* canon
deltas the operator ratifies, instead of hand-authored tool calls. The delta **class set and
payload shapes are a one-way door**: staged rows persist in operator databases, the ratify tools
and (later) the console render them, and mining prompts are tuned per class. Renaming or
reshaping a class after ship strands staged rows and breaks replay of the decided_* audit trail.

## Decision

### D1 — The class set (v1)

Fourteen classes. Each maps to **exactly one existing write tool** on apply — the miner never
gains a novel write path; ratification is replay of tools that already exist and are already
tested.

| Class | Applies via | Payload core (JSON, typed DTO in spindle-core) |
| --- | --- | --- |
| `canonical_fact` | `register_canonical_fact` | the tool's input minus project/branch (injected at apply) |
| `promise_planted` | `create_narrative_promise` | promise_type, description, planted_at (=mined scene), planned_payoff? |
| `promise_payoff_candidate` | `update_promise_status` | narrative_promise_id, proposed_status="paid_off", payoff_scene_ref |
| `promise_reinforced` | `update_promise_status` | narrative_promise_id, proposed_status="reinforced" |
| `relationship_shift` | `update_relationship` | relationship ref, trust/tension deltas, dynamics note |
| `character_state` | `commit_character_state` | character_id, state fields, position stamp |
| `knowledge_learned` | `record_knowledge` | character_id, fact, learned_at (=scene placement), `secret_of_fact_id?` (reveal deltas expand circles per secrets design §2.3.3) |
| `beat_annotation` | `annotate_scene_beats` | motif/theme/conflict ids, intensity |
| `try_fail_cycle` | `update_entity` (conflict) | conflict_id, cycle description appended |
| `consequence_delivered` | `update_entity` (conflict) | conflict_id, consequence index, delivered=true |
| `escalation_demonstrated` | `update_entity` (conflict) | conflict_id, stage index, demonstrated_at (=scene placement) — populates V0022 |
| `arc_milestone_reached` | `update_entity` (character_arc) | character_arc_id, milestone label, reached_at (=scene placement) — populates V0022 |
| `quantity_change` | `commit_quantity_state` | character_id, measure, amount/band, change_reason |
| `entity_candidate` | `create_character` / `create_location` / `create_term` | kind + the create tool's input; **never auto-accepted** regardless of policy |

### D2 — The staging row (fixed shape)

As specified in evolution §3.1 (`canon_delta` table): id, project/branch, scene_id provenance,
optional authoring_run_id, delta_class, target_id?, payload (typed JSON), evidence (≤300-char
sanitized prose quote — mandatory; a delta with no quotable evidence is not stageable),
confidence (high|medium|low), status (`staged | applied | rejected | superseded`), decided_at/by.

### D3 — Lifecycle rules

- **Supersede-on-remine:** re-mining a scene marks that scene's prior `staged` rows `superseded`
  and stages fresh ones. `applied`/`rejected` rows are never superseded — decisions are history.
- **Apply is transactional per scene** and may carry an operator `edit` of the payload
  (ratify-with-correction); the edited payload is what's recorded and applied.
- **Auto-accept policy** is per-class, opt-in, default empty; `entity_candidate` and any class
  whose payload carries `secret_of_fact_id` are excluded from auto-accept unconditionally
  (circle expansion is always a human decision).
- Rating discipline: mining is a prose-bearing pass — dispatch through the rating-gated
  chokepoint (evolution §4); un-cleared scenes skip mining with a `pass_skipped` record, never
  downgrade.

## Consequences

- The ratify queue, console, and acceptance-rate metrics key on `delta_class` strings — they are
  a public vocabulary from first ship.
- Because apply = existing tools, every delta inherits those tools' validation (incl.
  secret-link validation on `knowledge_learned`) for free; conversely, a delta cannot express
  anything the tool surface can't, which is the intended ceiling.
- The V0022 marker fields (milestones, escalation) gain their automatic population path,
  closing the loop the thread audits opened.

## Reversal cost

High for class renames/reshapes (orphaned staged rows, broken audit replay, retuned prompts);
low for *additions* (new classes are additive by construction). Removing a class requires a
superseding ADR plus a migration that terminalizes its staged rows.

## Alternatives considered

- **Free-form deltas (model emits arbitrary tool calls):** rejected — unbounded write surface,
  no per-class acceptance metrics, no auto-accept policy possible.
- **One generic `fact` class:** rejected — collapses the ratify queue into undifferentiated
  blobs; per-class confidence/precision measurement is the mechanism for earning auto-accept.
- **Direct-write mining with undo:** rejected outright — violates evolution invariant I4
  (ratified canon writes).
