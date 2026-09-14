# ADR 0003 — Plan-amendment classes and the living-outline contract

**Status:** Accepted (owner standing approval "continue", 2026-07-20) · **Supersede, never edit.**
**Context docs:** `docs/spindle-evolution-design.md` §3.5 (living outline); ADR 0001 (the staging
lifecycle this mirrors); ADR 0002 D2 (the reserved `replan_proposed` kind this activates).

## Context

P4 closes the last structural cap from the evolution design: plans are static while stories
drift. After each chapter summary, a replan pass compares realized reality (summaries, promise
events, beat annotations, arc deltas) against the *not-yet-drafted* chapters' plans and stages
amendment proposals the operator ratifies. As with canon deltas, the **class vocabulary, payload
shapes, and apply semantics are a one-way door**: staged amendment rows persist, the decide tools
and console render them, and applied amendments become the recoverable history of the outline.

## Decision

### D1 — The class set (v1)

Eight classes. Each applies through **existing plan write paths only** (`plan_chapter` /
`set_chapter_outline` / `create_narrative_promise`) — the replanner never gains a novel write
path.

| Class | Meaning | Applies via |
| --- | --- | --- |
| `synopsis_update` | rewrite a future chapter's synopsis to match realized trajectory | `plan_chapter` (merged) |
| `scene_add` | insert a planned scene into a future chapter's spine | `plan_chapter` |
| `scene_drop` | remove a planned scene from a future chapter | `plan_chapter` |
| `scene_replace` | swap a planned scene's summary/purpose/cast/location | `plan_chapter` |
| `scene_reorder` | reorder a future chapter's planned spine | `plan_chapter` |
| `thread_promote` | add a theme/conflict/plot-line id to a future chapter's targets | `plan_chapter` |
| `thread_retire` | remove a target id from a future chapter (thread resolved/abandoned) | `plan_chapter` |
| `promise_followup` | plant a successor promise after a payoff (planted_at = future placement) | `create_narrative_promise` |

### D2 — Staging row

Table `plan_amendment`, mirroring `canon_delta`'s shape and lifecycle exactly (id prefix
`plan_amendment:`; project/branch; provenance = the triggering chapter summary's chapter;
authoring_run_id nullable; amendment_class; target chapter_number (NULL only for
`promise_followup`); payload typed JSON; rationale TEXT NOT NULL — the replanner's stated
reasoning, ids/summaries only, no prose quotes; confidence; status
`staged | applied | rejected | superseded`; decided_at/by; **`prior_state` TEXT NULL** — see D4).
Supersede-on-replan: a new replan pass for the same source chapter supersedes that chapter's
prior `staged` amendments; decided rows are history.

### D3 — The immutability guard (normative)

An amendment whose target chapter has **any persisted scene on the active branch at apply time**
is rejected at apply (structural and synopsis/thread classes alike; checked at apply, not stage,
because drafting may advance between staging and decision). `promise_followup` targets a future
placement, not a chapter row, and is exempt from the chapter guard but must name a placement at
or after the next undrafted chapter. Drafted reality is never rewritten by the outline — the
outline chases the story, never the reverse.

### D4 — History via prior-state capture

At apply time, the affected slice of the plan (the full chapter-plan row for chapter-targeting
classes; nothing for `promise_followup`) is snapshotted into the amendment's `prior_state`
before the write, and `chapter_plan.plan_revision` (new nullable INTEGER column, NULL = 0)
increments. Outline history = the ordered applied amendments with their prior states; no
separate history table. Rollback of an amendment is a *new* operator edit informed by
`prior_state`, not an automated revert (plans are forward-moving).

### D5 — Policy and dispatch

- Run integration is opt-in: `replan_policy` on the run (`disabled` default / `propose_all`),
  mirroring `mining_policy`; the pass runs after `SaveChapterSummary` and stages against
  chapters strictly after the summarized one. Journal: the reserved `replan_proposed` kind
  activates (chapter + amendment_count).
- The differ's inputs are summaries and metadata only — **no scene prose** — so the `replan`
  route is non-prose-bearing: no rating clearance applies (documented boundary, same reasoning
  class as the import exemption). Ladder: `replan` → `review` on NoRoute → skip with an honest
  run-status note (never blocks the run).
- **No auto-accept exists for amendments** — not even as policy (evolution §3.5): every applied
  amendment is a human decision. Two new tools (`list_plan_amendments`,
  `decide_plan_amendments`) complete the evolution's 8-tool budget alongside the future
  recap/series-bible pair.

## Consequences

- The outline becomes a living document with recoverable history at zero new-table cost beyond
  the staging row itself.
- Because apply = `plan_chapter` replay, amendments inherit plan validation (first-class
  character_ids/location_id/content_rating requirements) for free — a staged amendment cannot
  produce a plan the drafting loop can't consume.
- The apply-time immutability check means a stale staged amendment (its chapter got drafted)
  fails honestly at decision time rather than corrupting a drafted chapter.

## Reversal cost

High for class renames/reshapes (orphaned staged rows, broken outline history); low for
additions. Removing a class requires a superseding ADR plus terminalizing its staged rows.

## Alternatives considered

- **Auto-applied replanning:** rejected — violates I4 and the §3.5 explicit no-auto-accept rule;
  the outline is authorial intent, and intent changes are always human decisions.
- **A separate plan-history table:** rejected — prior-state capture on the amendment row gives
  the same recoverability with the history attached to *why* it changed.
- **Prose-bearing replan (feeding scene prose to the differ):** rejected — summaries suffice for
  trajectory comparison, and keeping the route non-prose-bearing keeps it outside the rating
  perimeter by construction rather than by exception.
