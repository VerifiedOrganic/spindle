# ADR 0002 — Authoring-run event journal: kinds, payloads, delivery

**Status:** Accepted (owner standing approval "Continue", 2026-07-19) · **Supersede, never edit.**
**Context docs:** `docs/spindle-evolution-design.md` §3.4 (journal + SSE), §4.6 (observability
carries ids, never prose); ADR 0001 (the staging vocabulary some payloads reference).

## Context

P2 makes authoring runs observable and replayable: an append-only per-run event journal, streamed
over the existing SSE surface, consumed by `authoring_status` renderers, the future console, and
resume hardening. The **kind vocabulary and payload shapes are a one-way door**: journal rows
persist in operator databases, SSE consumers key on them, and replay tooling will parse them.
Renaming kinds or reshaping payloads after ship breaks every consumer and orphans recorded
history.

## Decision

### D1 — Storage (fixed shape)

```sql
CREATE TABLE authoring_run_event (
  id                TEXT    PRIMARY KEY,            -- authoring_run_event:*
  authoring_run_id  TEXT    NOT NULL REFERENCES authoring_run(id) ON DELETE CASCADE,
  seq               INTEGER NOT NULL,               -- 1-based, dense per run
  kind              TEXT    NOT NULL,
  payload           TEXT    NOT NULL CHECK (json_valid(payload)),
  created_at        INTEGER NOT NULL,               -- unix micros, house convention
  UNIQUE (authoring_run_id, seq)
);
```

Append-only: no UPDATE or DELETE path exists in the repository API (cascade delete rides the
run's own lifecycle only).

### D2 — Kind vocabulary (v1)

| Kind | Emitted when | Payload keys (beyond none) |
| --- | --- | --- |
| `run_started` | start_run persists the run | `book_number, start_chapter, end_chapter, mode?, mining_policy?, revise_policy?` |
| `scene_drafted` | a draft is saved (host or agent) | `chapter, scene_order, scene_id, origin: "host"\|"agent"` |
| `scene_verify_completed` | in-run verify finishes | `chapter, scene_order, scene_id, finding_counts: {severity: n}, verdict: "clean"\|"findings"` |
| `scene_revised` | a bounded revision round completes | `chapter, scene_order, scene_id, attempt, directive_finding_count` |
| `scene_committed` | commit step succeeds | `chapter, scene_order, scene_id` |
| `scene_mined` | mining step records an outcome | `chapter, scene_order, scene_id, mine_status, staged_count?, skip_reason?` |
| `deltas_decided` | decide_canon_deltas completes for run-mined deltas | `scene_id?, applied, rejected, failed` |
| `beats_annotated` | annotation step succeeds | `chapter, scene_order, scene_id` |
| `chapter_summarized` | chapter summary saved | `chapter, summary_artifact_path?` |
| `checkpoint_created` | checkpoint enters pending_review | `start_chapter, end_chapter, save_point_id, sampled_scene_ids` |
| `checkpoint_auto_approved` | an auto policy approves (P3) | `start_chapter, end_chapter, policy, finding_counts` |
| `checkpoint_blocked` | checkpoint requires operator | `start_chapter, end_chapter, reason` |
| `checkpoint_reviewed` | operator review completes | `start_chapter, end_chapter, directive_count` |
| `replan_proposed` | replan pass stages amendments (P4) | `chapter, amendment_count` |
| `pass_skipped` | any pass skips (rating, route, budget) | `pass, chapter?, scene_order?, reason` |
| `run_blocked` / `run_resumed` / `run_paused` / `run_completed` | run status transitions | `reason?` |

### D3 — Payload discipline (normative)

1. **Ids, paths, counts, enums — never content.** No prose, no fact/secret text, no delta
   payloads, no evidence quotes, no model output. The journal must be safe to leave streaming on
   a shared screen (evolution §4.6); artifact paths point at content, they never carry it.
2. **Additive evolution only.** New kinds and new optional payload keys are allowed; existing
   kinds/keys are never renamed, retyped, or removed. **Consumers MUST ignore unknown kinds and
   unknown keys** — this forward-compatibility rule is part of the contract and ships in the
   consumer-facing docs.
3. **Journal writes never fail the step.** Emission happens after the state change commits; an
   emission error logs at warn and the run proceeds. Consequence: consumers treat the journal as
   an honest-but-not-guaranteed-complete record; `authoring_status` (DB state) remains the source
   of truth, the journal is the timeline view.
4. **`seq` is the resume token.** Dense per run, assigned at append under the same connection
   discipline the run tables use; SSE delivers it as the event id so `Last-Event-ID` resume is
   exact. Payload-vocabulary kinds for phases not yet built (P3/P4 rows above) are RESERVED now
   so consumers can pre-register handlers; they simply do not occur until those phases land.

### D4 — Delivery

`GET /events?topic=run:<run_id>` on the existing HTTP surface streams journal rows as SSE
(`id` = seq, `event` = kind, `data` = payload JSON), replaying from `Last-Event-ID`+1 then
following live appends. Absent topic param → existing model-routes behavior unchanged (I1).

## Consequences

- The console and any notification tooling build on a stable vocabulary from day one; P3/P4 land
  as pure emitters with no consumer changes.
- Because payloads carry no content, rating/secret gating never applies to the journal — one less
  surface to audit.
- Replay/resume tooling can trust `(run_id, seq)` ordering; it cannot trust completeness (D3.3)
  and must reconcile against run state — this is deliberate (a journaling outage must never halt
  drafting).

## Reversal cost

High for renames/reshapes (breaks SSE consumers, orphans recorded timelines); low for additions
(D3.2). Removing a kind requires a superseding ADR; rows of removed kinds remain valid history
that consumers ignore by rule.

## Alternatives considered

- **Full event-sourcing (journal as source of truth):** rejected for P2 — inverting the
  source-of-truth relationship mid-flight risks every pinned compatibility flow; the design keeps
  DB state authoritative and the journal observational (evolution §2.2 notes the run tables are
  already 80% of a journal; a future ADR may revisit).
- **Prose-bearing payloads for richer UIs:** rejected — violates §4.6; the console reads content
  through the same gated read tools as everyone else.
- **Per-event journaling transactional with the step:** rejected — couples drafting availability
  to observability plumbing (D3.3 chooses the opposite).
