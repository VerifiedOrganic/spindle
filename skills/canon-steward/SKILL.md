---
name: canon-steward
description: >
  Use when reviewing, ratifying, or applying the canon deltas mined from committed scenes —
  the reviewed-diff path that replaces hand-authored canon tool calls. This includes mining a
  scene into proposed deltas, reading the ratify queue, judging each proposal against its
  verbatim evidence, editing a payload before applying it, and applying or rejecting deltas.
  Triggers for "mine this scene for canon", "review the canon deltas", "what's in the ratify
  queue", "apply the staged facts", "ratify the mined changes", "accept the canon proposals",
  or any request about turning a committed scene's prose into ratified Bible canon. Also the
  post-scene bookkeeping step an authoring run hands off to.
---

# Canon Steward

Every committed scene can be mined into *proposed* canon deltas — typed, per-class change
proposals, each grounded in a verbatim quote from the prose — that you, the operator, ratify
before they touch the Bible. This inverts Spindle's bookkeeping: instead of hand-authoring
`register_canonical_fact` / `update_relationship` / `commit_character_state` calls after every
scene, you review a diff. The steward's job is judgment, not typing: read the evidence, decide,
correct if needed, apply or reject.

The class set, payload shapes, and lifecycle are fixed by `docs/adr/0001-canon-delta-classes.md`.
Applying a delta never opens a new write path — each class maps to exactly one existing,
already-tested write tool. A delta cannot express anything the tool surface can't; that ceiling
is intentional.

## Workflow

### 1. Mine a committed scene

Call `set_active_project` once per session so follow-up calls inherit the project and active
branch. Then mine one committed scene:

```
mine_scene_canon({ project_id, scene_id })
```

One rating-gated model call reads the scene's prose and stages proposed deltas. The result
`status` is one of:

- `staged` — deltas persisted (see `staged`, `discarded_count`, `superseded_count`).
- `skipped` — no cleared model route for the scene's content rating, or the scene is empty
  (`skip_reason` names why). A skip is honest: it never reads as a clean mine.
- `model_output_rejected` — the model's JSON was malformed; nothing was staged.

Re-mining a scene **supersedes its prior `staged` deltas** and stages fresh ones. Already
`applied` or `rejected` rows are never superseded — decisions are history.

### 2. Read the ratify queue

Never decide from the mine output alone; list the queue and read each proposal in full:

```
list_canon_deltas({ project_id, status: "staged" })                       // whole branch
list_canon_deltas({ project_id, status: "staged", scene_id })              // one scene
list_canon_deltas({ project_id, chapter_range: { book_number, start, end } }) // a span
```

`scene_id` and `chapter_range` are mutually exclusive. Each delta carries its `delta_class`,
typed `payload`, verbatim `evidence` quote (≤300 chars, sanitized from the prose), `confidence`
(high | medium | low), `target_id` (the entity it modifies, or absent for a new one), and
`status`. Results are in deterministic order.

### 3. Judge each proposal against its evidence

**Evidence discipline is the whole game.** The `evidence` field is a verbatim substring of the
scene's prose — the mine pass discards any proposal whose quote is not literally present. Read
it. Ask: does this quote actually establish this fact? A `relationship_shift` with trust −2 must
be earned by the quoted line, not inferred from vibes. A `promise_payoff_candidate` must show
the promise genuinely paying off, not merely being mentioned. If the evidence doesn't support the
payload, reject or edit — do not rubber-stamp.

Weigh `confidence`: a `low`-confidence proposal is still stageable but deserves more scrutiny
than a `high` one. Cross-check the payload's `target_id` against the Bible (`get_entity`,
`search_bible`) when a proposal modifies an existing entity.

**Never bulk-apply without reading evidence.** Applying the whole queue sight-unseen defeats the
ratification economy — you become a passthrough for the model, which is exactly what the staged
queue exists to prevent. Read every quote you apply.

### 4. Decide (apply / reject, with optional correction)

Ratify a batch in one call. Each decision is `apply` or `reject`, with an optional `edit`
(a corrected payload) and an optional `note`:

```
decide_canon_deltas({
  project_id,
  decided_by: "operator",            // optional; defaults to "operator"
  decisions: [
    { delta_id, action: "apply" },
    { delta_id, action: "apply",  edit: { /* corrected payload */ } },
    { delta_id, action: "reject", note: "narrator is unreliable here" }
  ]
})
```

Semantics you must internalize:

- **Pre-flight is all-or-nothing.** Every decision is validated *before any write*: the delta
  exists, belongs to the project, and is currently `staged`; apply payloads deserialize to the
  target tool's shape and their referenced entities exist. A single pre-flight failure aborts the
  whole call with **zero writes** — every row stays staged. Fix the offender and resubmit.
- **Finality.** Deciding a row that is already `applied`, `rejected`, or `superseded` is an input
  error that fails the whole call. Decisions are permanent history; there is no un-decide. If a
  ratified fact was wrong, correct it forward with a new canonical fact (`supersedes_fact_id`),
  not by re-deciding.
- **Edit-before-apply.** An `edit` replaces the staged payload — the edited payload is what gets
  applied *and* what is recorded on the row. Use it to fix a value the model got slightly wrong
  rather than rejecting and re-mining.
- **Notes are advisory.** A `note` is echoed back in the output but not persisted on the row (the
  `canon_delta` table has no note column). Capture durable rationale with `record_note` if it must
  survive.

The output reports one result per decision (`applied` | `rejected` | `failed` | `not_reached`)
plus counts. On `applied` for a class that creates a row (`canonical_fact`, `promise_planted`,
`entity_candidate`, and others), `applied_record_id` carries the new id.

**Mid-apply honesty.** If an apply fails *after* pre-flight (a real write error — rare, e.g. a
referenced row was concurrently changed), the dispatcher stops: earlier applies in the batch stay
applied (they are real, recorded canon — nothing is silently rolled back), the failing row is
reported `failed` with its error and stays staged, and later rows read `not_reached` and stay
staged. Re-list, understand the failure, and resubmit the remainder.

### 5. Secret-link deltas get extra scrutiny

A `knowledge_learned` delta carrying `secret_of_fact_id`, or any delta whose payload carries a
secret link, is a **circle-of-trust expansion** — you are deciding that a character now legitimately
knows a secret. Per the ADR, circle expansion is *always* a human decision; it is excluded from
any auto-accept policy unconditionally. Before applying:

- Confirm the referenced fact is genuinely secret and the reveal is intended canon (the character
  was told on-page or off-page), not a leak the prose committed by accident.
- Read the evidence quote carefully: does the scene actually reveal the secret to this character,
  at this placement? A premature circle expansion leaks the secret backward in time.

If unsure whether a reveal is canon, that is a plot/canon choice — ask the operator; do not apply
on a guess.

## Subagent orchestration (Claude Code / grok)

Reviewing a chapter's worth of mined deltas splits cleanly by scene, and each scene's evidence
judgment is independent read-only reasoning. If your harness supports subagents (Claude Code's
Task/Agent tool, grok's subagents), fan the *review* out; otherwise run the same review inline —
same steps, less concurrency.

**Write discipline (non-negotiable):** subagents review and report only. Every `decide_canon_deltas`
call — the only tool that writes canon — stays in the main context. The steward decides and
applies; subagents never call `decide_canon_deltas`, `mine_scene_canon` (it stages rows), or any
`register_*`/`update_*`/`commit_*`/`record_*`/`create_*`/`annotate_*` write. They call
`list_canon_deltas`, `get_entity`, `search_bible`, `get_scene_context`, and
`find_scenes_referencing`, and return per-delta recommendations.

Fan-out for a mined chapter range:

- **One subagent per scene with staged deltas** — dispatch each with the scene id; the subagent
  calls `list_canon_deltas({ scene_id })`, reads every delta's evidence quote against the scene's
  prose and the Bible, and returns a recommendation per delta: apply / apply-with-edit (with the
  corrected payload) / reject (with a reason) / escalate-to-operator (secret-link or plot/canon
  choice), each anchored to the evidence quote.
- The steward (main context) merges the recommendations, exercises final judgment on the escalated
  and secret-link deltas, and issues the `decide_canon_deltas` batch itself.

Without a subagent mechanism, walk the scenes one at a time in the main context — read each queue,
judge the evidence, decide.

## Skill chains

- **authoring-supervisor** hands off here for post-scene mining after a drafting run's scenes are
  committed.
- After ratifying, hand off to **continuity-editor** to run a consistency sweep over the newly
  applied canon, or to **bible-librarian** to inspect what landed.
- For a correction to an already-ratified fact, use **editor** / **worldbuilder** with
  `register_canonical_fact` + `supersedes_fact_id` — never by re-deciding a decided row.

---

## References

- `docs/adr/0001-canon-delta-classes.md` — the fourteen delta classes, their payload shapes,
  the class→write-tool mapping, and the staging/decision lifecycle. The source of truth.
