# Drafting packets and critique-driven revision (P1)

Status: staged implementation. Legacy remains the public default; release
gates have not passed. See the implementation status sections below.

## Evidence baseline and scope

Source baseline: `origin/main` at `419f40be7378d8919ac89d28139496d06b7db935`.
All file:line citations below refer to that immutable main commit, not this
feature branch. Proposed contracts, limits, and acceptance thresholds are design
decisions, not claims that they already exist. Parallel source splitting may
move these symbols; implementations should find the symbols, not retain line
numbers as API boundaries.

P1 owns prompt selection/rendering and the editorial controller. P2 owns canon
extraction semantics, event persistence, retrieval and embeddings; P3 owns plan
creation; P4 owns final tool naming/consolidation. P1 consumes their versioned
contracts without introducing another planner, event store, or retrieval engine.
Keep public DTOs in core, orchestration in adapters, MCP thin, and harness a
public-MCP client, consistent with the current write path
(`crates/spindle-harness/src/execution.rs:288`).

## What the code actually does

| Finding | Evidence on the baseline and implication |
| --- | --- |
| Host prompt combines output schema, rules, packet hooks, directives, ID manifest, research, constraints, briefing, JSON context, and full skill | `crates/spindle-harness/src/execution.rs:1928`–`2027`; skill fetched at `:1856`. The assembly has no final tokenizer budget. The reported 40–70k tokens is an audit observation, not a universal measured size. The checked-in skill is 58,487 bytes; its client workflow includes tool calls (`skills/scene-writer/SKILL.md:61`–`80`, `:183`). A stateless completion cannot execute those instructions. |
| Scene direction lacks a typed scene card | Manifest at `crates/spindle-harness/src/execution.rs:1943` carries IDs, chapter synopsis, tone and rating. `SceneSeed` (`crates/spindle-harness/src/state.rs:51`) lacks the goal/turn/emotion/length contract proposed below. |
| Voice hooks are not necessarily prose exemplars | `crates/spindle-core/src/style/antislop/packet.rs:225`–`250` selects do-rules, prompt snippet, and narrator notes; don't interpret all `voice_samples` as approved prose. |
| Context truncation is not narrative selection | Tail is at most 1,200 characters, approximately 200 English words, not a word contract (`crates/spindle-adapters/src/format.rs:1537`); story-so-far total is 2,000 bytes (`:3871`–`3880`). Token estimation is chars/4 (`:2875`–`2876`). |
| Prose and extraction share one completion | JSON `full_text`, facts, states, relationships and beats at `crates/spindle-harness/src/execution.rs:1984`–`2000`; package parsing at `:1608`. |
| Revision is a redispatch, not a prior-draft editing contract | Deterministic findings drive directives, old artifact is removed, phase reset and draft route invoked (`crates/spindle-harness/src/execution.rs:881`–`945`). There is already an unchanged-fingerprint convergence guard (`:814`) and an attempt cap (`:824`); retain both ideas. |
| Auto reviews are retained now, but not editorial feedback | `crates/spindle-mcp/src/tools.rs:5401`–`5494` collects and records sampled reviews. `:5496`–`5508` explicitly makes reader simulation and review content report-only, outside finding counts. “Discarded reviews” is stale for this main revision. |
| Route declaration is not a line-edit controller | `line_edit` is prose-bearing (`crates/spindle-adapters/src/ai.rs:131`); the in-run revision path above uses `draft`. The current scheduler's verify/revise section (`crates/spindle-harness/src/plan.rs:863`) does not supply the proposed chapter/book editorial stages. This is a scoped flow observation, not a claim that no manual revision tools exist. |
| Agent and host context paths differ | `caller_should_send_brief` selects an MCP-pull prompt instead of embedded context (`crates/spindle-harness/src/execution.rs:1799`–`1838`). Preserve capability differences while sharing the packet semantics. |

There is no opening/ending repetition-memory field in the cited scene manifest
or seed. Add a bounded editorial memory, rather than treating a longer tail or a
larger global summary as equivalent. The evaluation baseline records overall
recall 0.5714 and dead-character recall 0.0
(`evals/results/continuity_recall_baseline.json:14`–`29`); P1 must not describe
these as guarantees for another model or improve them by changing the fixture.

## DraftingPacketV1

Use a typed, serializable public core DTO, with a separate deterministic renderer.
The following is a proposed schema notation, not executable Rust:

```text
DraftingPacketV1 {
  schema_version, packet_id,
  project_id, branch_id, run_id?, scene_id?, scene_revision?,
  canon_snapshot_hash, plan_revision, source_versions[],
  scene_card: {
    pov: { character_id, display_name, person: first|third, tense: past|present },
    goal, opposition, turn, intended_outcome,
    value_shift: { axis, before, after },
    emotion: { entry, exit },
    chapter: { book_number, number, scene_index, planned_scene_count?, is_closer },
    target_words: { min, preferred, max },
    location: { id, name }, story_clock, rating, tone?,
    participants: [{ id, display_name, role }],
    required_beats[], promises_due[], prohibited_outcomes[]
  },
  hard_constraints: [{ id, kind, statement, source_ref, scope }],
  continuity: { character_states[], knowledge_boundaries[], facts[],
                prior_scene_tail?, story_so_far[], open_threads[] },
  voice: { rules[], exemplars: [{ text, speaker_id?, source_ref,
                                 approved_revision_hash }] },
  editorial_memory: { recent_openings[], recent_endings[], recurring_images[],
                      repeated_phrases[], pacing_history[] },
  craft_rubric: { version, text }, research_evidence[],
  assembly: { tokenizer_id, tokenizer_version, section_counts[], omissions[],
              input_tokens, output_reserve, context_window }
}
```

Each selected fact, excerpt, and memory item has a branch-aware source ID and
revision/hash; author-approved examples are labeled examples, never canon.
Narrative requirements distinguish author locks from suggested choices. Missing
POV, rating, goal, turn, or length makes packet preparation incomplete; the host
can fill a persisted provisional scene card from existing plans. Do not invent
canon or silently resolve contradictory author locks. P3 can later provide the
same card. `is_closer` is explicit even when final chapter length is unknown.
Person/tense may inherit an author-approved project default, with provenance.

Only approved prose excerpts enter `exemplars`; existing do-rules remain rules.
If none exist, say `exemplars=[]`. Never manufacture a “voice sample.” Default
memory covers the previous eight accepted scenes plus the current chapter:
first/last paragraphs, opening/ending device tags, repeated imagery and selected
phrase counts, with scene/hash provenance. P1 can compute this locally from
accepted text; P2 may later supply retrieval. It is advisory, never a ban on
intentional motifs. Rejected candidates never update it.

### Priority and tokenizer budget

Proposed default: 12,000 input tokens and 3,000 output tokens for an 800–1,500
word scene; reserve another 512 tokens for model framing. Admit only if
`input + output_reserve + framing <= configured model context window`.
Use the resolved model's tokenizer, counting the final rendered messages,
including system text and delimiters. Pin tokenizer identity/version. No chars/4
fallback for automatic dispatch: unsupported tokenizer yields `budget_unknown`
and requires a configured verified tokenizer or host handling.

Render in this order, with a short scene-task reminder last. Allocations below
are maxima, not padding; unused capacity may flow down the priority list.

| Priority | Section | Default token ceiling | Overflow policy |
| --- | --- | ---: | --- |
| 0 | Output contract, rating, scene card and reminder | 1,500 | Required; never truncate |
| 1 | Hard canon/knowledge/time/location constraints, author locks | 2,500 | Required; overflow blocks preparation |
| 2 | Current states, due promises, prior-scene closing excerpt | 2,500 | Select relevant whole records; tail up to 1,000 tokens |
| 3 | Approved voice prose and voice rules | 1,500 | Prefer POV and dialogue participants; whole excerpts |
| 4 | Craft rubric | 1,500 | Versioned static maximum, usually smaller |
| 5 | Story-so-far, repetition memory, research | 2,500 | Rank by scene relevance, then recency and stable ID |

The ceilings total 12,000. Priority 2 facts necessary to obey a hard constraint
are promoted to priority 1. Deduplicate by source and proposition; don't send
both JSON and markdown copies. Never cut a fact, quote or UTF-8 code point in
half. Record excluded items, reason and counts in the artifact, not the prompt.
Recount after rendering. Overfull mandatory context stops rather than guessing.

Replace embedded SKILL.md with a reviewed rubric of at most 1,500 tokens:
scene purpose and turn; POV knowledge and sensory selection; distinct speech;
concrete action and consequence; emotional change; fresh, purposeful language;
opening/ending variation; author intent over generic advice. Include applicable
compact anti-slop rules without repeating their surrounding manual. Full skills,
tool instructions, ID lookup catalogs, extraction schemas, briefing formatting,
and unused world records stay in resources/controller code. Tool-capable agents
may consult resources, but skill loading is not part of the prose task.

## Plain-text draft and separate extraction

Draft output is UTF-8 scene prose only: no JSON envelope, fences, analysis or
canon fields. Store exact returned bytes and hash before any other pass. A
truncated response is `incomplete`, never accepted; bounded continuation counts
against the same token/time/call ledger. Do not silently strip uncertain text.
The legacy combined-package parser remains available only for legacy runs
(current parser: `crates/spindle-harness/src/execution.rs:1608`).

After editorial acceptance, run a separate structured extraction on the accepted
text hash, selected canon and ID map. P1 requires a summary/beat/continuity package
sufficient for existing save/commit steps; P2 owns extractor implementation and
fact semantics. In a staged rollout a compatibility extractor produces the
existing package without requiring a drafting model to emit it. Extraction failure
keeps accepted prose staged and commit pending; it never triggers a prose re-roll.
Extraction output is proposed metadata, validated before canon application.
Never extract canon from a losing candidate. Preserve source spans, rating
clearance and explicit-generation receipt provenance through the split; the
existing save path passes generation identity (`crates/spindle-harness/src/execution.rs:307`).
A receipt must identify the exact accepted draft or revision, not its predecessor.

## Editorial controller

Proposed state progression:

```text
packet_ready -> drafted -> deterministic_checked -> critiqued
  -> candidate_ready -> candidate_checked -> pairwise_judged
  -> accepted_original | accepted_revision
  -> optional_line_edit -> extracted -> saved -> committed
```

Persist immutable versioned artifacts and append transitions. Each transition
uses `(run, branch, scene, base_text_hash, packet_hash, stage, attempt)` as an
idempotency key; restart must not replay paid calls or commit twice. Use compare
and swap on branch/text/canon versions before promotion. A concurrent edit or
canon change makes anchors and judgment stale; reprepare instead of overwriting.
Do not delete the incumbent artifact as today's redispatch does
(`crates/spindle-harness/src/execution.rs:909`).

### Critic and directives

Call a rating-cleared `review` model with the packet, exact draft, deterministic
findings and relevant retained reader/dual-persona feedback. A critic assesses
the seven existing prose-judge dimensions: scene purpose, tension/turn, POV,
voice, dialogue, freshness and continuity
(`evals/prose_judge.py:56`–`64`), plus length, entry/exit emotion and chapter role.
Scores are diagnostic, not an automatic optimization target.

```text
CritiqueV1 {
  base_text_hash, packet_hash, rubric_version,
  dimensions: [{ dimension, score_1_to_5, evidence_spans[], rationale }],
  directives: [{ id, dimension, severity, priority,
    anchor: { scene_id, base_text_hash, start_byte, end_byte, exact_quote },
    diagnosis, requested_change, preserve[], success_test,
    source_finding_ids[], scope: span|scene|chapter }],
  strengths_to_preserve[], requires_author_decision[], no_edit_reason?
}
```

Offsets are half-open UTF-8 byte ranges validated against exact quoted text.
Insertions use zero-width anchors and surrounding context; scene-wide concerns
anchor their evidence and declare scope explicitly. Reject malformed, stale or
unverifiable anchors. One bounded schema repair is allowed inside the call cap;
otherwise retain draft with `critic_invalid`. Select at most five actionable
directives, prioritize continuity and author intent, deduplicate deterministic
and reader concerns. Model suggestions cannot create new canon or override locks.

The revision prompt contains the full prior draft, compact packet, selected
directives and strengths to preserve. Ask for targeted edits preserving unaffected
passages, not a new scene from beats. Default output is full plain text to avoid
fragile patch syntax; record changed spans and flag unexpectedly broad changes
(over 40% changed paragraphs) for manual review. Exceptional structural rewrites
require an explicit directive and remain subject to the same acceptance gate.

### Acceptance and convergence

Run deterministic checks on each candidate before the judge. Any new hard
continuity/rating violation disqualifies it; unresolved blockers park the scene
for review rather than labeling it clean. Preserve existing hard anti-slop
semantics; soft findings are critic evidence, not a blanket rejection.

**Keep the revision only if a pairwise judge prefers it.** Adapt
`evals/prose_judge.py:924`–`1005` (`judge_pair_double`) into a tested runtime
contract: anonymize incumbent/candidate, randomize order, judge both orders,
deblind; order disagreement is a tie. Give the judge the same card, constraints
and exemplars, not critic scores, revision labels or model identity. Both passes
must prefer candidate; tie, original win, malformed output, timeout or unavailable
rating-cleared judge retains incumbent with an honest status. Do not import or
shell out to the Python eval from the Rust server; share fixtures and parity
tests for rubric, parsing and verdict semantics. Runtime may use one configured
judge family; promotion evaluations use an independent multi-family panel.

Default one critique/revision round, hard maximum two. Stop on no actionable
directives, repeated normalized finding fingerprint, identical text hash,
rejected candidate, no directive-resolution progress, or any budget exhaustion.
Round two is permitted only after a preferred revision with remaining actionable
issues and available reservation. Never revise the same unchanged issue set twice.
Unresolved craft concerns are parked at checkpoint; mandatory continuity blockers
require manual resolution. Disabled editorial policy preserves legacy semantics.

Optional `line_edit` runs once after structural acceptance, default off. Restrict
to diction, syntax and dialogue polish, preserving events and scene turn. Its
candidate must pass the same deterministic checks and two-order pairwise gate;
rejection retains pre-line-edit text. Resolve every prose-bearing dispatch at the
actual content rating (route set: `crates/spindle-adapters/src/ai.rs:131`).

## Chapter and book scope

After a chapter's scenes are provisionally accepted, run one chapter critic over
openings/endings, repetition memory, beat/intensity progression and scene lengths.
Use retained reader engagement/confusion notes and dual-persona evidence as
inputs, not orders. Resolve concerns to scene/hash spans before editing; an
unanchored concern stays advisory. Propose at most two scene edits per chapter,
then apply each via the scene candidate/check/judge gate, with chapter context
included in both judge orders. A local win must also clear a chapter-context
comparison for the affected scene sequence; reserve those calls before editing.
Invalidate chapter summary, extraction, beat annotations and dependent reader
notes after acceptance; recompute before checkpoint and revalidate downstream
continuity if an already committed scene changes. Use staged branch-local edits
and existing revision semantics, not direct database updates.

Long chapters use deterministic scene windows plus a chapter index; never
silently truncate the chapter and claim full review. Explicitly report coverage
and aggregate cross-window opening/ending/repetition checks. Book-level pass at
run end is advisory by default: chapter summaries, pacing trajectory, repeated
open/close devices and unresolved promises produce a revision queue. No automatic
book rewrite or plot redesign in P1; author selects queued chapter work, which
uses the same bounded controller. P3 owns structural replanning decisions.

## Hybrid, agent and CLI behavior

Both modes consume the same canonical packet and persisted editorial states.
Hybrid returns a bounded host task with packet/hash and incumbent/directives for
revision. Host submits plain text plus task ID/hash; controller validates,
critiques/judges via cleared routes or returns an explicit host-review task.
A missing automated judge never counts as preference: keep incumbent and mark
`pending_manual_judgment`. Host completion token usage may be unknown; record it
as unknown and reserve configured maxima rather than fabricating API usage.

Agent mode dispatches the same rendered packet to `draft`, critic/judge to
`review`, optional polish to `line_edit`. A tool-capable MCP-pull agent may fetch
the immutable packet by task ID; retrieved additions must enter a new counted,
hashed packet before prose generation. A stateless agent receives inline data
and no impossible tool instructions. Never permit free extra retrieval to bypass
budget or branch snapshot constraints. Route gaps use supported host fallback
or honest skip; explicit provenance and rating gates still apply.

Harness remains an MCP client. Extend seed/run configuration with
`editorial_policy=legacy|critique_v1`, packet/rubric versions, round cap,
line-edit flag, chapter-pass flag and budget object; persist resolved settings at
prepare. Existing `init`, `status`, `verify`, `resume --execute-one` are the CLI
seams (`crates/spindle-harness/src/main.rs:20`–`72`). Each execute-one advances
one persisted editorial action. Dry-run reports the next action, reservation and
missing route/tokenizer without calling a model. Status shows incumbent/candidate,
critic/judge outcome, parked issues, remaining caps and pending host task.
Older state without new fields resumes legacy behavior. Proposed examples:

```text
spindle-harness init --state run.json --seed seed.json
spindle-harness resume --state run.json --execute-one --writeback
spindle-harness status --state run.json --verbose
```

These commands exist; new seed fields and stages are proposals. P4 determines
final public MCP names. Do not duplicate a separate CLI-only editor.

## Cost and latency envelope

Proposed reservations below are ceilings for the default scene, not measured
provider performance or current prices. Input includes system framing. Larger
scenes require an explicit profile; never cut prior draft to fit a revision.

| Pass | Calls | Input/output token reservation per call | Wall deadline |
| --- | ---: | ---: | ---: |
| Draft | 1 | 12,512 / 3,000 | 90 s |
| Critic | 1 | 16,000 / 1,500 | 45 s |
| Revision | 1 | 18,000 / 3,000 | 90 s |
| Two-order judge | 2 | 16,000 / 1,000 | 45 s each |
| Accepted-text extraction | 1 | 8,000 / 2,000 | 45 s |

Default worst reservation: six calls, 86,512 input + 11,500 output = 98,012
tokens, 360 seconds serial deadlines. A clean/no-directive scene skips revision
and judges: three calls, at most 43,012 total tokens and 180 seconds. These are
hard timeouts, not p50 promises; first live pilot must report p50/p95 and tune.
Retries, repair and continuations consume the same ledger; no implicit retries.
Reserve extraction capacity before optional editing.

Per-scene hard cap: 12 calls, 210k total tokens, 720 seconds, configured USD
ceiling default $2.00. Second round or line edit each needs four calls (critic or
line-editor, candidate generation where applicable, and two judges); admission
uses exact proposed calls, not permission to exceed the cap. For line edit alone
reserve three calls. Chapter critic defaults to one call (24k input/2k output,
90 seconds), with a chapter pool of 20 calls/320k tokens/20 minutes/$4 including
affected-scene work and chapter comparisons. Book advisory pool: four calls/100k
tokens/five minutes/$1. Per-run hard caps: 200 calls, 3M total tokens, 90 minutes,
$20, shared by scenes, chapter, book, reader simulation and checkpoint review.
These may stop a long run early; prepare estimates feasibility and reports it.

Use configured per-model input/output prices to reserve worst-case USD before
dispatch; reconcile with actual usage, retain conservative reservation when
usage is unknown. Missing price makes the monetary cap unverifiable and blocks
automatic paid execution unless an explicit local/free profile applies. Persist
reservations atomically with stage IDs; restart cannot reset counters. All
limits apply together, earliest exhaustion wins. Keep incumbent, mark budget
exhausted and pause required work; never call it a successful quality review.

## Attribution and evaluation

Reuse V0047 fields, not a second model-call ledger:
`crates/spindle-adapters/migrations/V0047__model_call_attribution.sql:1`
(the migration filename is verified during doc checks). `RequestContext` already
carries run/scene/template/version/hash
(`crates/spindle-adapters/src/ai.rs:252`–`268`), and
`compute_context_packet_hash` SHA-256 hashes exact text (`:293`).

Assign distinct versioned templates for packet draft, critic, revision, judge
and line-edit; send `prompt_version` and exact assembled-message hash on every
call. Preserve current exact-prompt hash semantics; additionally record canonical
packet-content hash, tokenizer/version, source snapshot, variant assignment,
parent/candidate hashes, stage/attempt, judge order, outcome, durations, actual
and reserved tokens/cost in a prose-free editorial event linked to model-call
IDs. Hash system plus user messages with an unambiguous serialization. Persist
prose only in existing access-controlled local artifacts, not telemetry.

Randomize variant once per run (avoid adjacent-scene contamination), record seed,
model/version and sampling settings. Export saved baseline/candidate text to the
existing evaluation schema, preserving hashes. Compare frozen current-main
baseline, packet-only variant, and packet+loop variant to separate effects.
Existing judging supports saved files and live panel flags
(`evals/prose_judge.py:1804`–`1822`), swapped orders (`:924`) and bootstrap CIs
(`:1008`, `:1087`). Use independent judge families for release evaluation; runtime
judge selection cannot serve as evidence that its own decisions generalize.

## PR-sized implementation sequence and gates

No implementation is included here. Each step lands independently behind the
new opt-in policy. The shared release gate **G** is: frozen 30-brief suite across
five scene types, at least three generation seeds, same model/settings across
conditions; report tie-half candidate win rate with 95% bootstrap CI and outright
wins/ties, position bias and judge-family agreement. Cluster resampling by brief
when combining seeds (extension to current item bootstrap), retain per-type
results. Enable a quality-changing step by default only when lower CI bound is
above 0.50 versus frozen baseline and its previous enabled stage. Inconclusive
means remain opt-in, not pass. Infrastructure-only steps require exact output
parity (tie-half 0.50, CI [0.50, 0.50]) until their behavior flag is enabled.

Every step also runs **C**: unchanged `continuity_recall.py` fixture/config on
baseline and candidate; overall and every per-type recall must not decrease,
including dead-character acting; no increase in clean-control false positives.
The 57% historical figure is context, not permission to regress another type.
Run the same checks on generated accepted prose in addition to the planted
fixture, which alone does not exercise draft quality. Eval tooling must add that
integration fixture before enabling the controller.

| PR | Scope and tests | Eval acceptance |
| --- | --- | --- |
| 1 | Typed packet/card DTO, branch/source provenance, tokenizer interface, rubric. Snapshot required-field failures, UTF-8 boundaries, dedup, deterministic ordering, mandatory overflow and final-message counts. Legacy output untouched. | Parity gate + C; packet-only experiment recorded under G, feature remains off until G clears. |
| 2 | Plain-text generation artifact and separate compatibility extraction. Test truncation, exact bytes/hash, receipt binding, extraction failure/restart, accepted-only extraction. Coordinate P2 schema without implementing its event store. | G on packet/plain-text vs baseline and packet-only; C. |
| 3 | Critic DTO, anchor validation and feedback ingestion. Test invalid offsets/quotes, stale hashes, duplicate findings, malformed repair, author-lock conflicts and rating denial. Shadow mode cannot mutate text. | Shadow parity + C; blinded directive usefulness sample; eventual activated loop must pass G. |
| 4 | Prior-draft revision, deterministic candidate checks and double-order acceptance. Shared Python/Rust verdict fixtures; test tie/error retains incumbent, new blocker rejects, attempt/fingerprint stops, CAS conflict, crash at every boundary. | G for loop vs baseline and packet-only + C; report rejection and edit-span distributions. |
| 5 | Hybrid host task and agent parity; CLI resume/status, durable budget ledger and attribution. Test no-route, unknown usage, concurrency reservations, restart caps, explicit receipt preservation and legacy state. | Output parity + C; run G separately per execution mode before enabling either. |
| 6 | Optional line-edit route and bounded chapter pass, repetition memory, checkpoint feedback adapters and book advisory queue. Test loser excluded from memory, chapter coverage, invalidation/re-extraction and chapter-level rejection. | G vs baseline and scene-loop-only, chapter-coherence blinded comparison CI lower bound >0.50; C. Keep each optional pass off if inconclusive. |

Required local tests follow touched crates; include architecture-boundary tests
when moving DTOs or responsibilities. Offline judge self-tests validate machinery,
not prose improvement (`evals/prose_judge.py:1539`). Live evaluation uses
`ai run OLLAMA_API_KEY -- python3 evals/prose_judge.py judge ... --live` with
explicit saved baseline/candidate files and `/tmp` result paths; a missing key
blocks live validation and must be reported. No live model calls or claimed win
rate are necessary to validate this design-only PR.

## Open questions for the author

1. Should stronger prose be allowed to change planned outcomes? Recommended
   default: no; outcome/value shift and author locks require explicit approval.
2. Is the cost of double-order judging acceptable for routine scenes?
   Recommended default: opt-in critique policy, one round, six-call reservation;
   preserve legacy mode until G clears and real cost/latency are measured.
3. Should chapter revision run unattended? Recommended default: enable advisory
   chapter critique first; author opts into bounded gated edits. Book-wide
   structural changes remain an author-approved queue.

## Implementation status: packet foundation (PR 1)

`spindle_core::drafting` now defines the versioned packet and scene-card contracts.
`spindle_adapters::drafting::assemble_packet` prepares counted, deterministic
messages using a caller-supplied tokenizer verified for the resolved model.
There is no approximate tokenizer fallback. Its result retains the complete
source artifact and omission report; only `messages` are eligible for dispatch.
The message hash covers an unambiguous JSON serialization of system and user
text. Supplied assembly counts and rubric text are replaced during preparation.
Inherited source branches must be explicitly present in `source_versions`;
preparation validates membership, not database snapshot freshness. Duplicate and
conflict checks use the full source (branch, ID, revision hash) plus its local
proposition ID; independent sources may reuse proposition IDs. The future
service integration must resolve branch visibility and perform snapshot CAS.

This foundation is not connected to generation or exposed as an MCP tool.
Existing runs and prompts retain legacy behavior. `EditorialPolicy` defaults to
`legacy`; its `critique_v1` variant is a contract for subsequent rollout, not an
active run setting. No model tokenizer is assumed supported by default. Packet
messages contain a compact reviewed rubric and plain-prose contract rather than
skills, standards or extraction instructions. Existing legacy combined-package
prompts remain until the separately gated plain-text/extraction integration.
No release quality gate has passed and no default is changed.

## Implementation status: packet draft and extraction boundary (PR 2)

Following the orchestrator's sequencing update, PR 2 connects the packet to the
real drafting route without waiting for P2's extractor implementation.
`ModelRouter::draft_packet` and
`SqliteSpindleService::draft_packet_to_artifact` are opt-in Rust APIs guarded by
`EditorialPolicy::CritiqueV1`. They do not activate a critic/controller, add an
MCP tool, or change existing authoring runs. Legacy combined-package generation
and continuity save/commit workflows remain available and unchanged.

The caller must supply a tokenizer verified for the actual resolved model.
Only HTTP adapter framing is supported initially; other adapters return
`budget_unknown`. Counts include the configured route system prompt, applicable
rating appendix, packet task framing and final user message. The actual HTTP
body uses those exact messages. Output reservation cannot be smaller than the
configured route output cap. Packet calls make one HTTP attempt, with no hidden
transport retries. Existing completion retry behavior is preserved for legacy
calls. Runtime per-run paid-call/budget admission remains a prerequisite for
public automated execution in PR 5.

The service persists a `PlainTextDraftArtifactV1` before returning, including
the prepared packet, assembly/omission report and exact dispatched messages. It preserves
exact UTF-8 bytes and their SHA-256, separately from the existing normalized
receipt hash (CRLF normalized and surrounding whitespace trimmed). A source-packet
fingerprint prevents reuse with changed inputs. Reopening the same captured
artifact verifies the snapshot and receipt instead of generating again. Expired
or mismatched receipts stop with the artifact retained. Truncated output stays
staged and cannot be accepted; no continuation or prose re-roll is implicit.

Local JSON snapshots use atomic replacement and a per-path lock. Concurrent
owners are rejected. Immediately before HTTP dispatch, the lock is marked unresolved and synced.
Cancellation, process crash or any unresolved post-dispatch error retains that
marker; only durable artifact capture clears it. Budget/tokenizer/clearance
failures before dispatch release the lock normally. Inspect the artifact, packet
hash and receipt before explicitly recovering an unresolved lock. This is not the future durable model-call reservation ledger.

`spindle_core::drafting::extraction` defines the consumer-side
`AcceptedTextExtractor` trait. Its versioned request carries accepted text/hash,
packet identity, branch, immutable `StoryPlacement` (including unsaved scenes),
rating, unchanged generation receipt, selected canon and known entity IDs. The
packet chapter carries a book number and one-based scene order; these positive
coordinates are frozen in the accepted artifact and retry request. Selected
canon is preceding ratified evidence only, excluding future plans/summaries and
drafter assertions. Provisional conflicts must be separately labeled by a future
P2 schema extension rather than mixed into this field. The response supplies only summary/beat/compatibility
continuity metadata bound to the accepted hash. P2 implements extraction,
rating clearance and task-id idempotency; no extraction semantics or canon/event
store are implemented here. `extract_accepted` requires a persisted accepted
artifact, freezes retry inputs, persists each attempt and failure, and caches
successful metadata. Failure never clears accepted prose or dispatches a draft.
`GeneratedScenePackage::from_accepted_packet` adapts accepted metadata into the
existing package shape while retaining the exact accepted prose. No canon is
automatically applied by these APIs.

No built-in production tokenizer is assumed verified. Without one, a live
packet-vs-baseline experiment cannot pass admission; G remains unpassed and no
quality win is claimed. Public host/agent scheduling and CLI integration remain
later steps, coordinated with P2 and P4.
