# Spindle Authoring Supervisor

The Spindle Authoring Supervisor is an MCP-native authoring pipeline that enables an interactive, agent-driven long-form drafting workflow. It manages the multi-step drafting, verification, review, and checkpoint loop safely while synchronizing the run state in a SQLite database.

The default authoring posture is autonomous quality supervision: the active
assistant should keep writing and improving prose without asking the operator to
approve every local craft decision. Checkpoints are still hard quality gates,
but reviewer findings are treated as instructions to improve the manuscript,
not as a prompt for permission. The assistant should fix or carry forward
findings automatically unless the review requires a plot, canon,
content-boundary, relationship, or author-intent decision.

## Architecture & Database Schema

Unlike the traditional CLI-based `spindle-harness` which relies on external harness state files, the Authoring Supervisor persists run states directly in the SQLite database to facilitate robust resumes, state tracking, and integration with the MCP tool router.

The tables defined in migration `V0007__authoring_runs.sql` are:

1. **`authoring_run`**: Stores metadata for the overall multi-chapter drafting run, including the project, active branch, start/end chapters, checkpoint intervals, editorial directives, status, and timestamps.
2. **`authoring_run_chapter`**: Tracks chapter-level progress, plans, synopses, POV character assignments, status, and summary artifacts.
3. **`authoring_run_scene`**: Tracks scene-level progress, including the participating characters, location, content rating, phase (`pending`, `draft_saved`, `changes_committed`, `beats_annotated`), scene ID, and artifact paths.
4. **`authoring_checkpoint`**: Tracks checkpoint ranges, statuses (`pending_review`, `reviewed`), and report artifact paths.

## MCP Tools Interface

The supervisor exposes 9 MCP tools through the `spindle-mcp` server:

| Tool Name | Description | Key Input Fields | Key Output Fields |
|---|---|---|---|
| `authoring_prepare_run` | Verifies plans and resources are ready before drafting. | `project_id`, `book_number`, `start_chapter`, `end_chapter` | `ready_to_draft`, `missing_requirements` |
| `authoring_start_run` | Initializes a new authoring run. | `project_id`, `book_number`, `start_chapter`, `end_chapter`, `checkpoint_interval`, `mining_policy` (optional; `disabled` default or `propose_all`), `max_revise_attempts` (optional; `0` default, `1` or `2` to enable in-run verify/revise), `checkpoint_policy` (optional; `manual` default, `auto_advisory` or `auto_strict` to self-clear checkpoints), `replan_policy` (optional; `disabled` default or `propose_all` to replan future plans after each chapter summary) | `run_id`, `status` |
| `authoring_status` | Retrieves status and next actions of the active run. | `project_id`, `run_id` (optional) | `status`, `next_action`, `blocked_reason`, `chapters` |
| `authoring_execute_next` | Advances exactly one bounded drafting/commit/checkpoint action. Default mode is interactive/hybrid: non-explicit draft steps return host-draft instructions instead of calling the draft route. | `project_id`, `run_id`, `mode` (optional; use `"agent"` only for intentional full offload) | `run_id`, `executed_action`, `next_action`, `status` |
| `authoring_save_scene_draft` | Saves host-drafted prose plus its required structured continuity package. | `project_id`, `run_id`, scene placement, `full_text`, `summary`, `character_states`, `canonical_facts`, `relationship_updates`, `beats`, `continuity_notes` | `run_id`, `scene_id`, `scene_artifact_path`, `structured_update_count` |
| `authoring_record_checkpoint_audit` | Attaches the deep consistency result returned by a separate `check_consistency(deep_check=true)` call to a pending checkpoint. | `project_id`, `run_id`, `start_chapter`, `end_chapter`, `deep_consistency` | `run_id`, `status` |
| `authoring_review_checkpoint` | Marks a checkpoint reviewed and appends directives to resume. | `project_id`, `run_id`, `start_chapter`, `end_chapter`, `directives` | `run_id`, `status` |
| `authoring_resolve_block` | Clears a blocked scene after operator inspection and advances it to the next safe phase, or resets a poisoned scene to pending-draft with `target_phase: "redraft"`. | `project_id`, `run_id`, `chapter_number`, `scene_order`, `target_phase` | `run_id`, `status` |
| `authoring_cancel_run` | Pauses or cancels the active run without deleting progress. | `project_id`, `run_id` | `run_id`, `status` |

Chapter plans are the source of truth for pre-draft scene requirements. Each
`plan_chapter` scene should carry first-class `character_ids`, `location_id`,
and `content_rating` fields. `authoring_prepare_run` still accepts old plans
that encoded `location:` / `rating:` in summary text as a legacy fallback, but
new supervisor flows should not depend on that parsing convention.

`mining_policy` on `authoring_start_run` is optional and defaults to `disabled`
(a run that never opts in behaves exactly as before). Set it to `propose_all`
to insert an automatic `mine canon` step between `commit scene changes` and
`annotate beats`: each committed scene is mined into proposed canon deltas the
operator ratifies via the canon-delta flow (`list_canon_deltas` /
`decide_canon_deltas`). The mining outcome is recorded honestly per scene in
`authoring_status` as `mine_status` (`staged` | `skipped` |
`model_output_rejected` | `error`) plus a `mine_detail` — a skip or error never
reads as a clean mine, and mining never blocks the run. When `propose_all` is
in play, `authoring_prepare_run` (called with the same `mining_policy`) also
verifies the mine-or-review route ladder covers every planned rating and reports
`missing_requirements` when it cannot, so an offload gap fails at prepare rather
than mid-run. `authoring_prepare_run`'s optional `mining_policy` input drives
this extra preflight and defaults to skipping it.

`max_revise_attempts` on `authoring_start_run` is optional and defaults to `0`
(disabled — a run that never opts in behaves exactly as before: a saved draft
goes straight to `commit scene changes`). Set it to `1` or `2` (a bound; higher
values are rejected as an input error) to insert a deterministic `verify scene`
step between `draft` and `commit`. Verify runs the scene-scoped check subset
(`SCENE_VERIFY_CHECKS`, no model calls); a finding at or above `warning` sends
the scene back to the same draft route as a bounded `revise scene (attempt N)`
(in hybrid mode `authoring_execute_next` instead returns a "Host revision
required" instruction listing the findings, and the host re-saves via
`authoring_save_scene_draft`, which resets the verify state and counts the
attempt). The outcome is recorded honestly per scene in `authoring_status` as
`verify_status` (`clean` | `findings` | `parked_findings` | `error`),
`verify_detail`, and `revise_attempts`. The loop converges or stops: a re-verify
with an unchanged finding set parks the scene (`parked_findings`, "unchanged
after revision") rather than re-revising the same findings, and any parked
findings inherit to the checkpoint. Verify is deterministic and revision reuses
the already-preflighted draft route, so `authoring_prepare_run` adds **no** extra
coverage check for this policy; verify never blocks the run.

`checkpoint_policy` on `authoring_start_run` is optional and defaults to
`manual` (a run that never opts in runs the classic 4-step operator checkpoint
flow, byte-identical to before). The two auto policies let the supervisor
self-clear a checkpoint in-process instead of surfacing `await_checkpoint_review`:

| Policy | Checkpoint behavior |
|---|---|
| `manual` (default) | The classic flow: run `check_consistency(deep_check=true)`, `authoring_record_checkpoint_audit`, sampled `run_dual_persona_review` (rounds 2), then `authoring_review_checkpoint`. |
| `auto_advisory` | The harness runs the deep consistency check, records the audit, runs the sampled dual-persona reviews via the `review` route, then **auto-approves iff no finding is `warning`-or-worse** (`info` is allowed). Otherwise it blocks with the full report exactly like `manual`. |
| `auto_strict` | Same automation, but auto-approves **only on zero findings of any severity**. An `info`-only finding set that `auto_advisory` approves, `auto_strict` blocks. |

An auto policy requires — enforced at `authoring_start_run` via
`authoring_prepare_run`'s preflight — that the `review` route resolves
rating-cleared for **every** distinct content rating in the run's range;
otherwise start is blocked with a `missing_requirements` entry naming the policy,
the `review` route, and the uncovered rating (fail at prepare, not mid-run). The
deep-check-capable review-route requirement collapses into this same check today
because the same `review` route serves the checkpoint's deep dual-persona pass.

**Explicit-manual-fallback:** if a sampled scene's dual-persona review dispatch
is rejected at the offload chokepoint because the `review` agent is not cleared
for that scene's rating (e.g. a mid-run `configure_agents` change dropped
explicit coverage), that scene is marked **pending-manual** and is dispatched
nowhere else — its prose never reaches an uncleared model. The checkpoint then
blocks listing exactly which scenes await manual review; the cleared scenes'
reviews and the deep audit still completed and are recorded. `authoring_status`
surfaces the outcome per checkpoint as `checkpoint_policy`, `auto_outcome`
(`approved` | `blocked` | `manual`), and `pending_manual_scene_ids`. On approval
the run journal emits `checkpoint_auto_approved` (start/end chapter, policy,
finding counts); on a block it emits `checkpoint_blocked` (reason). A blocked
auto-checkpoint stays `pending_review`, so the manual escape hatch
(`authoring_review_checkpoint`) still clears it. There is no per-checkpoint
model-cost ceiling in v1. Canon-steward ratification (`list_canon_deltas` /
`decide_canon_deltas`) still happens between checkpoints regardless of policy.

### Living outline (replan)

`replan_policy` on `authoring_start_run` is optional and defaults to `disabled`
(a run that never opts in behaves exactly as before). Set it to `propose_all` to
insert an automatic `replan future plans` step immediately after each chapter
summary and before that chapter's checkpoint. The replan pass (`replan_chapter`)
audits the just-summarized chapter's **realized reality** (summary, key events,
promise/arc states, beat annotations) against every **not-yet-drafted** future
chapter's plan and stages plan-amendment proposals — the outline chases the
story, never the reverse. The differ is **non-prose-bearing** (summaries +
metadata only, no scene prose), so no rating clearance applies: on a missing
route it falls to `review` then **skips honestly**, and it never blocks the run.
Because there is no rating perimeter to protect, `authoring_prepare_run` adds
**no** extra coverage check for this policy. The outcome is recorded honestly per
chapter in `authoring_status` as `replan_status` (`staged` | `skipped` |
`no_targets` | `no_summary` | `error`) plus `replan_detail`; a staged pass emits
a `replan_proposed` run-journal event (chapter + amendment count), a skip emits
`pass_skipped`. The pass runs **at most once per chapter**, so it never delays
the checkpoint.

Staged amendments are **never auto-applied** — every applied amendment is a human
decision. Review the queue with `list_plan_amendments` (filter by status and by
`book_number` + `source_chapter` provenance; read each `rationale`) and ratify a
batch with `decide_plan_amendments`, exactly as canon deltas are decided. Each
decision is `apply` or `reject`, with an optional `edit` (a corrected payload
applied AND recorded) and an operator `note`. All decisions are pre-flighted
before any write: a single failure aborts the whole call with **zero writes**.
The pre-flight enforces the **immutability guard** — an amendment whose target
chapter has *any persisted scene on the active branch at apply time* is rejected
(drafted reality is never rewritten by the outline; the guard is checked at
decision time, not at staging, because drafting may advance between). On apply,
the amendment replays through the existing plan write path (`plan_chapter`, or
`create_narrative_promise` for a `promise_followup`), the affected plan slice is
snapshotted into the amendment's `prior_state` before the write, and
`plan_revision` increments — so the outline gains recoverable **history**
(rollback is a new operator edit informed by `prior_state`, not an automated
revert). A per-decision outcome (`applied` | `rejected` | `failed` |
`not_reached`) is returned; a real write failure mid-batch stops honestly (earlier
applies stay applied, the failing row stays staged).

### Reader simulation

Under an auto policy (only — v1 does not run reader-sim on the manual flow), the
auto-checkpoint automation runs a **cumulative reader simulation** after the
sampled dual-persona reviews and before the verdict. It reads the checkpoint
range's chapters **in order, with memory**: a persona derived from the project's
reader contract (promise + style notes + boundaries — "you are the reader this
book promises…") reads each chapter's committed prose in spine order, reports its
engagement (`high` | `steady` | `dipping`) and any craft concerns, and writes a
self-contained set of cumulative notes that carry forward to the next chapter. So
engagement drift ("chapter 13 retreads chapter 12") becomes visible.

The reader's rolling memory lives in one `reader-sim-notes.json` per run (the run
artifacts dir), carrying `updated_through_chapter`, the current `notes`, and a
per-range `history`. The notes we feed into the next chapter's prompt are capped
char-safe at 4000 (a tail truncation keeping the newest content). Each chapter's
result lands additively in the checkpoint report's `reader_sim` section
(`{chapter, engagement, concerns[], skipped_reason?}` + `notes_artifact_path`),
and `authoring_status` surfaces a compact per-chapter engagement summary
(`reader_sim_engagement`).

Reader simulation is **enrichment, not a gate**: it rides the `reader_sim` route,
falling back to `review` when no `reader_sim` route is configured, and is fully
rating-gated at the dispatch chokepoint. A chapter whose rating the route cannot
serve, or a transport failure, is **skipped honestly** — an entry that names the
route + rating (never prose) — and the pass continues; a reader-sim skip never
marks scenes pending-manual. Its concerns are **report-only** and never fold into
the approve/block verdict (which is computed from the deep-consistency severity
counts alone, exactly as the sampled-review outcomes are report-only). A dipping
engagement or a warning concern is a craft signal for the supervisor to weigh at
triage, not an automatic block.

## Interactive Drafting Workflow

An agent driving the drafting run executes the following loop:

```mermaid
graph TD
    A[Start: User Request] --> B[authoring_prepare_run]
    B --> C{Ready to Draft?}
    C -- No --> D[Report missing resources to user]
    C -- Yes --> E[authoring_start_run]
    E --> F[authoring_execute_next]
    F --> G{Next Action?}
    G -- Host Draft Required --> L[Active assistant drafts and authoring_save_scene_draft]
    L --> F
    G -- Active Action --> F
    G -- Await Checkpoint Review --> P[Run deep consistency and record audit]
    P --> H[Run sampled dual-persona review]
    H --> M{Needs operator decision?}
    M -- No, local fix/directive --> N[Revise or carry directive autonomously]
    N --> I[authoring_review_checkpoint]
    M -- Yes --> O[Ask focused operator question]
    O --> I
    I --> F
    G -- Blocked by Error --> J[Report errors to user / authoring_resolve_block]
    G -- Complete --> K[Drafting finished]
```

### Checkpoint Review Policy

When a run reaches `await_checkpoint_review`, the supervisor should inspect the
checkpoint report. If deep consistency is pending, run `check_consistency` for
that chapter range with `deep_check: true`, then call
`authoring_record_checkpoint_audit` with the returned structured payload. Then
run any missing sampled dual-persona reviews with `rounds: 2`. The checkpoint
cannot be closed until the deep audit is recorded and sampled reviews are
current in the local database.

After the review:

- For local craft issues such as missing sensory grounding, repetition,
  weak line phrasing, pacing trim, filter words, scene-specific UI/prose
  punch-up, missing expected LitRPG/system markup, or required reward/result UI
  blocks, the assistant should revise the scene itself and rerun the relevant
  review/check steps before approving the checkpoint.
- For feedback that should shape later chapters but does not require changing
  completed prose, approve the checkpoint with clear directives.
- For plot, canon, content-boundary, relationship-direction, or author-intent
  choices, ask the operator a focused question instead of guessing.

The supervisor should not ask "revise or approve?" for reviewer findings it can
address without changing author intent. It should choose the quality path and
continue the run.

### Resuming an Interrupted Run
If the session or service restarts mid-run:
1. Call `authoring_status` to fetch the active run.
2. If the status is `"blocked"` with reason `"await_checkpoint_review"`, inspect the checkpoint report, run/record any pending deep consistency audit, run any missing sampled dual-persona reviews, resolve local fixes/directives, then call `authoring_review_checkpoint`.
3. Call `authoring_execute_next` to continue driving the drafting loop.

Run statuses are `"active"`, `"blocked"`, `"completed"`, or `"paused"`. `authoring_cancel_run` sets `"paused"` and `authoring_execute_next` will not advance a paused run.

### Recovering an Unparseable / Poisoned Draft
If an automated draft comes back as unparseable model output (for example an
agent that narrates around the JSON), `authoring_execute_next` reports the parse
error. The harness now **discards the poisoned completion automatically**, so the
next `authoring_execute_next` re-dispatches a fresh draft rather than re-parsing
the cached output forever. If you want to force a clean re-draft explicitly —
e.g. after fixing agent config — call `authoring_resolve_block` with
`target_phase: "redraft"`. That resets the scene to pending-draft (clearing the
stored generation, deleting the stale scene artifact, and clearing verify state)
so the next execute re-drafts it from scratch. The forward-advance phases
(`draft_saved`, `changes_committed`, `beats_annotated`) are unchanged.

### Draft Routing Semantics

By default, `authoring_execute_next` is intended for the natural MCP/chat
workflow: the active assistant writes General, Teen, and Mature prose using
Spindle context, then saves it with `authoring_save_scene_draft`. This keeps
the primary writing voice in the current conversation while requiring the same
structured continuity package as offloaded drafting. Explicit scenes may still
route through the explicit-capable backend configured for the project.

`authoring_save_scene_draft` requires at least one structured continuity entry:
`character_states`, `canonical_facts`, `relationship_updates`, `beats`, or
`continuity_notes`. If the scene introduces no durable canon changes, record
that explicitly in `continuity_notes`.

`mode: "agent"` is an explicit opt-in for fully automated tests or batch runs.
In that mode, Spindle routes non-explicit scenes through the configured `draft`
route too. Do not use it for normal interactive writing unless the operator
asks for full offload.

### Explicit-content offload guarantee

Every automated pass that carries scene prose — drafting, in-run revision,
canon mining, and (once enabled) review/reader-sim automation — resolves its
route through the single rating-gated chokepoint. A scene's prose is never
dispatched to a model whose agent does not declare the scene's content rating.
When a pass cannot serve a rating it **skips honestly** rather than downgrading:
the mine step records `mine_status: "skipped"` naming the rating and emits a
`pass_skipped` journal event; the run advances anyway (skips never block). The
in-run verify step is deterministic (no model call), so it carries no leakage
risk, and the journal carries ids/counts only — never prose. The checkpoint
samples the last scene in the range, so an interleaved explicit scene is not
routed to review by the default flow. This whole guarantee is pinned by an
integration test that drives a `general` + `explicit` run whose `mine` route
resolves only to a non-explicit agent and asserts, against a post-gate dispatch
recorder, that no request containing the explicit scene's prose or brief ever
reached the uncleared agent (evolution design §4).

The same rating-gated chokepoint now also covers the model-backed
`check_consistency` deep tiers that carry scene prose — the intra-scene temporal
check and the semantic world-rule check — which previously dispatched with
`rating: None` and bypassed clearance; each now stamps the scene's rating and
skips honestly (one `info` finding naming the route and rating) when the review
agent does not declare it.

## Run journal (observability)

Each authoring run appends an **append-only event journal** as it advances —
one row per observable transition (draft, verify, revise, commit, mine, annotate,
chapter summary, checkpoint create/block/review, and run status changes). The
journal is the *timeline view*; `authoring_status` (the run tables) remains the
source of truth. A journaling error never fails a run step — it logs at `warn`
and the run proceeds — so the journal is an honest-but-not-guaranteed-complete
record.

The kind vocabulary and payload shapes are a one-way door, fixed by
[ADR 0002](adr/0002-authoring-run-event-journal.md) (§D2 lists every kind and its
payload keys). Payloads carry **ids, artifact paths, counts, and enums only —
never prose, fact text, evidence, or model output** (ADR §D3.1), so the stream
is safe to leave open on a shared screen. Consumers must ignore unknown kinds and
unknown payload keys (additive-evolution rule, ADR §D3.2); the P3/P4 kinds in the
ADR table are reserved and do not occur until those phases land.

### Streaming over SSE

The journal streams over the existing HTTP surface at
`GET /events?topic=run:<authoring_run_id>`:

- Each SSE frame carries `id` = the event `seq`, `event` = the kind, and `data`
  = the payload JSON.
- Replay resumes exactly from `Last-Event-ID`: the server replays rows with
  `seq > Last-Event-ID`, then follows live appends (polled on the snapshot
  cadence). `seq` is dense and 1-based per run, so resume is exact.
- `GET /events` with **no** `topic` param is unchanged: it streams the existing
  model-routes snapshot. A malformed topic (unknown scheme or an id that is not a
  well-formed `authoring_run:` id) returns `400`. A well-formed run id with no
  events is a valid empty stream (no existence check).
