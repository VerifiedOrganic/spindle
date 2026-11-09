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
| 3 | Verified-tokenizer prerequisite: pinned `tokenizer.json` encoders, provider calibration certificates, Ollama count probes and cache, final-message verification before dispatch. | Token-count parity + C; legacy remains default. |
| 4 | Eval-to-Rust generation bridge (`spindle-tokenizer draft-packet` / `draft-messages`), generation seed plumbing, cluster-bootstrap CIs by brief across seeds, generated-accepted-prose continuity fixture, and full live 90-pair G experiment (`30 briefs × 3 seeds`). | Honest G report (`0.4444 [0.3667, 0.5167]`) + C (`20/28 = 71.43%`, `5` FPs, `90` verified artifacts); lower bound `<= 0.50` keeps `EditorialPolicy::Legacy` default. |
| 5 | Critic DTO, anchor validation and feedback ingestion. Test invalid offsets/quotes, stale hashes, duplicate findings, malformed repair, author-lock conflicts and rating denial. Shadow mode cannot mutate text. | Shadow parity + C; blinded directive usefulness sample; eventual activated loop must pass G. |
| 6 | Prior-draft revision, deterministic candidate checks and double-order acceptance. Shared Python/Rust verdict fixtures; test tie/error retains incumbent, new blocker rejects, attempt/fingerprint stops, CAS conflict, crash at every boundary. | G for loop vs baseline and packet-only + C; report rejection and edit-span distributions. |
| 7 | Hybrid host task and agent parity; CLI resume/status, durable budget ledger and attribution. Test no-route, unknown usage, concurrency reservations, restart caps, explicit receipt preservation and legacy state. | Output parity + C; run G separately per execution mode before enabling either. |
| 8 | Optional line-edit route and bounded chapter pass, repetition memory, checkpoint feedback adapters and book advisory queue. Test loser excluded from memory, chapter coverage, invalidation/re-extraction and chapter-level rejection. | G vs baseline and scene-loop-only, chapter-coherence blinded comparison CI lower bound >0.50; C. Keep each optional pass off if inconclusive. |

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

## Implementation status: verified-tokenizer prerequisite (PR 3)

The orchestrator inserted this prerequisite after PR 2 and before the critic
(previous PR 3). Subsequent sequence numbers advance by one; public scheduling
integration remains deferred until its designated integration PR. No run policy
is activated by this prerequisite.

Pinned `tokenizer.json` encoders now implement assembly-time counting, with
provider-bound calibration certificates and explicit bounded tolerance. Ollama
Cloud calibration uses authorized one-token count probes; local zero-output
probes require zero-generation proof. Counts are cached and attributed to the
existing model-usage ledger. The internal packet route resolves the configured
encoder when no caller-supplied counter is provided and verifies final-message
counts before prose dispatch by default. See `spindle-agent-config.md` for the
verified Gemma profile, operator CLI, recovery behavior and billing details.
Legacy remains default; calibration is evidence of token counting, not prose
quality or completion of release gate G.

## Implementation status: prose-judge generation bridge and full G experiment (PR 4)

PR 4 connects `evals/prose_judge.py` to the internal Rust packet generation and
calibrated native Ollama transport, adds the generated-accepted-prose continuity
integration fixture to `evals/continuity_recall.py`, and records the full 90-pair
(`30 briefs × 3 seeds`) live release-gate **G** experiment.

### Generation bridge and seed / hash provenance

- **Generation seed plumbing**: `PacketAssembly` (`spindle-core`), `PacketBudget`
  (`spindle-adapters`), `RoutingRule`, and `ModelRoute` now carry an optional
  `generation_seed: Option<i64>`. Native Ollama `/api/chat` requests pass
  `options.seed` when present, while one-token count probes explicitly clear
  `seed` so probe cache keys remain seed-independent. Cached
  `PlainTextDraftArtifactV1` reloads verify that the recorded
  `prepared_packet.packet.assembly.generation_seed` matches the requested
  `PacketBudget::generation_seed`.
- **Same effective transport and context policy across arms**: Both the packet
  candidate arm (`drafting_packet_v1`, dispatched through
  `SqliteSpindleService::draft_packet_to_artifact` and accepted via
  `accept_artifact`) and the frozen legacy baseline arm (`spindle_full_context`,
  dispatched through `ModelRouter::draft_eval_messages`) use the same configured
  `draft` route (`gemma4:31b`, revision
  `842da3794eaa0b77d5f08bae87a17459d91ff475`), the same calibrated Hugging Face
  `tokenizer.json` (`cc8d3a0ce36466ccc1278bf987df5f71db1719b9ca6b4118264f45cb627bfe0f`,
  `tolerance_tokens = 0`, `verify_each_prompt = true`), and the same native
  Ollama `/api/chat` window and sampling policy (`options.num_ctx = 32768`,
  `options.num_predict = 1200`, `options.temperature = 0.4`, `truncate = false`,
  `shift = false`).
- **Multi-seed cluster-bootstrap CIs**: `evals/prose_judge.py` supports
  `--generation-seed` and `--generation-seeds 101,202,303`, pairs saved outputs
  by `(brief_id, generation_seed)` (`P001-S101` .. `P030-S303`), records
  `packet_hash`, `source_packet_sha256`, `prompt_hash`, `tokenizer_id`,
  `tokenizer_version`, and `settings` per pair, and computes 95% bootstrap
  confidence intervals clustered by `brief_id` (`n_clusters = 30`,
  `n_resamples = 2000`) when multiple seeds per brief are evaluated.

### Continuity gate C and generated-accepted-prose integration fixture

`evals/continuity_recall.py` now includes `evaluate_generated_accepted_prose`
(reported under `generated_accepted_prose_fixture` and runnable standalone via
`verify-accepted-prose` / `--accepted-artifacts-dir`):

- **Planted 2-manuscript benchmark (`C`)**: `20 / 28` (`71.43%`) overall recall,
  `5` control false positives (`world_rule_compliance: 2`,
  `world_rule_semantic_drift: 3`), and unchanged per-type recall across all 7
  categories (`dead_character_acting: 4/4 = 1.0`,
  `timeline_chronology_rewind: 4/4 = 1.0`, `quantity_affordability: 4/4 = 1.0`,
  `character_attribute: 2/4 = 0.5`, `knowledge_leak_before_learned: 2/4 = 0.5`,
  `world_rule_cost_violation: 2/4 = 0.5`, `secret_leak: 2/4 = 0.5`).
- **Generated-accepted-prose continuity fixture**: Verifies
  `PlainTextDraftArtifactV1` schema/receipt/SHA-256/non-truncation invariants
  (rejecting truncated or tampered artifacts), ingests accepted prose through
  MCP `save_scene_draft` -> `check_consistency` with `0` clean-prose blocker
  issues and `4 / 4` (`1.0`) recall across planted accepted-prose contradictions
  (`character_attribute`, `dead_character_acting`, `world_rule_cost_violation`,
  `timeline_chronology_rewind`), and verified all `90 / 90` live
  `PlainTextDraftArtifactV1` artifacts produced during G.

### Full live G experiment results (30 briefs × 3 seeds = 90 pairs)

Checked-in summary artifact: `evals/results/prose_judge_packet_v1_g_live.json`.

- **Generation summary (`gemma4:31b`, seeds `101, 202, 303`)**:
  - `drafting_packet_v1` (candidate): `90` accepted artifacts, `0` omissions on
    all `30` briefs, mean `3279.1` input tokens, mean `441.0` words.
  - `spindle_full_context` (frozen legacy baseline): `90` completions, mean
    `935.1` input tokens, mean `418.0` words.
- **Judge panel (swapped-order double judging, `540` live judge calls across 3
  independent model families)**:
  - `ollama-glm-5.2` (`zhipu-glm` family)
  - `ollama-deepseek-v4.1-flash` (`deepseek` family)
  - `ollama-gemma4-31b` (`google-gemma` family)
- **Overall panel consensus (`n = 90` pairs, `n_clusters = 30` briefs)**:
  - Candidate (`drafting_packet_v1`) outright wins: `14 / 90` (`0.1556`, 95%
    cluster-bootstrap CI `[0.0778, 0.2444]`)
  - Baseline (`spindle_full_context`) outright wins: `24 / 90` (`0.2667`, 95%
    cluster-bootstrap CI `[0.1667, 0.3667]`)
  - Ties: `52 / 90` (`0.5778`)
  - **Candidate tie-half win rate**: **`0.4444` (`44.44%`), 95% cluster-bootstrap
    CI `[0.3667, 0.5167]`**
- **Per-scene-type tie-half win rates (`18` pairs / `6` briefs per type)**:
  - `chapter-closer`: `0.5556` (`[0.4167, 0.7222]`, `4` candidate wins, `2`
    baseline wins, `12` ties)
  - `dialogue-heavy`: `0.4444` (`[0.3889, 0.5000]`, `2` candidate wins, `4`
    baseline wins, `12` ties)
  - `quiet-interior`: `0.4444` (`[0.2778, 0.6111]`, `2` candidate wins, `4`
    baseline wins, `12` ties)
  - `reveal`: `0.4167` (`[0.2222, 0.5833]`, `3` candidate wins, `6` baseline
    wins, `9` ties)
  - `action`: `0.3611` (`[0.2222, 0.5556]`, `3` candidate wins, `8` baseline
    wins, `7` ties)
- **Per-seed tie-half win rates (`30` pairs per seed)**:
  - Seed `101`: `0.4833` (`[0.3833, 0.6000]`, `5` candidate, `6` baseline, `19` ties)
  - Seed `202`: `0.3833` (`[0.2667, 0.4833]`, `3` candidate, `10` baseline, `17` ties)
  - Seed `303`: `0.4667` (`[0.3500, 0.5833]`, `6` candidate, `8` baseline, `16` ties)
- **Per-judge tie-half win rates and position bias (`90` double-judged pairs each)**:
  - `ollama-glm-5.2`: tie-half `0.4500` (`[0.3722, 0.5278]`, `22` candidate, `31`
    baseline, `37` ties), order consistency `0.5889`, position-bias rate `0.4111`.
  - `ollama-deepseek-v4.1-flash`: tie-half `0.5000` (`[0.4333, 0.5667]`, `16`
    candidate, `16` baseline, `58` ties), order consistency `0.3556`,
    position-bias rate `0.6444`.
  - `ollama-gemma4-31b`: tie-half `0.3611` (`[0.2833, 0.4389]`, `11` candidate,
    `36` baseline, `43` ties), order consistency `0.5222`, position-bias rate
    `0.4778`.
  - Overall position-bias rate across `270` double judgments: `0.5111` (`126`
    first-position, `9` second-position, `3` tie-flip), all resolved to ties by
    swapped-order double judging.
  - Inter-judge agreement: Fleiss' $\kappa = 0.2912$, unanimous rate `0.3778`,
    pairwise non-tie directional agreement `0.7917`–`0.9091` (Cohen's $\kappa$:
    `0.2421`, `0.3361`, `0.3325`).
- **Rubric dimension diagnosis (`270` double-judged pairs)**:
  - `continuity_with_brief`: candidate `4.5000` vs baseline `4.4296` (`+0.0704`,
    tie-half `0.5259`, `78` candidate wins vs `64` baseline wins) — structured
    `DraftingPacketV1` improves canon/brief adherence and `chapter-closer`
    payoff.
  - `dialogue` (`-0.0092`, tie-half `0.4815`), `pov_discipline` (`-0.0223`,
    tie-half `0.4759`), and `scene_purpose_fulfilled` (`-0.0500`, tie-half
    `0.4537`) are near parity.
  - One-shot `drafting_packet_v1` (without the critic/revision loop) trails the
    hand-tuned legacy markdown prompt on `tension_turn` (`4.0593` vs `4.2574`,
    `-0.1981`, tie-half `0.3963`), `voice_distinctiveness` (`4.0370` vs `4.1870`,
    `-0.1500`, tie-half `0.4111`), and `prose_freshness` (`3.4333` vs `3.5778`,
    `-0.1445`, tie-half `0.4222`).
- **Release gate decision**: Because the 95% cluster-bootstrap CI lower bound is
  `0.3667 <= 0.50`, one-shot packet generation does **not** clear gate **G** for
  default activation. `EditorialPolicy::Legacy` remains the default, and
  `DraftingPacketV1` / `PlainTextDraftArtifactV1` stays opt-in while the stream
  proceeds to PR 5 (`CritiqueV1` DTO + span/hash/quote anchors) and PR 6
  (prior-draft revision + double-order acceptance).

## Implementation status: critic DTO, anchor validation, feedback ingestion, and shadow mode (PR 5)

PR 5 implements the structured scene critic contract (`CritiqueV1`), UTF-8
byte-span/hash/quote anchor validation, multi-source feedback ingestion and
deduplication, author-lock conflict quarantine, bounded 1-shot schema/anchor
repair, and shadow-only execution (`ShadowCritiqueArtifactV1`).

### G-informed scene-critic rubric (`scene-critic-v1`)

The critic rubric (`CRITIC_RUBRIC`, `rubric_version = "scene-critic-v1"`) incorporates
the empirical failure modes measured in the 90-pair live **G** experiment:

- **Action scenes (`0.3611` tie-half in G — weakest scene type)** and **reveal
  scenes (`0.4167` tie-half in G — second weakest)**: The rubric explicitly checks
  whether structured packet constraints caused the draft to front-load or flatten
  its dramatic turn (`tension_turn`, `-0.1981` in G), over-explain the reveal instead
  of letting concrete action/subtext carry the discovery, or lose choreographic
  escalation across physical beats.
- **Chapter-closer payoff (`0.5556` tie-half in G — strongest scene type)** and
  **brief continuity (`+0.0704` in G)**: The critic preserves the packet's
  structural advantages on `continuity`, `chapter_role`, and `entry_exit_emotion`
  while targeting `tension_turn`, `voice` (`-0.1500` in G), and `freshness`
  (`-0.1445` in G) with span-anchored directives rather than generic rewrite
  advice.

### `CritiqueV1` DTO, anchor validation, and feedback ingestion

- **Typed critic contract (`spindle_core::drafting::critique`)**:
  - `CritiqueV1` (`schema_version = "critique_v1"`) binds `base_text_hash`
    (SHA-256 of the exact UTF-8 draft bytes), `packet_hash`, `rubric_version`,
    all 10 `CriticDimension` scores (`scene_purpose`, `tension_turn`, `pov`,
    `voice`, `dialogue`, `freshness`, `continuity`, `length`,
    `entry_exit_emotion`, `chapter_role`, with serde aliases for the
    `prose_judge.py` rubric keys), up to 5 ordered `CritiqueDirective` items,
    `strengths_to_preserve`, `requires_author_decision: Vec<AuthorDecisionItem>`,
    and optional `no_edit_reason`.
- **UTF-8 byte-span, quote, and zero-width insertion validation
  (`validate_anchor`)**:
  - Every `CritiqueAnchor` is validated against `base_text` as a half-open UTF-8
    byte interval `[start_byte, end_byte)` on valid char boundaries with
    `start_byte <= end_byte <= base_text.len()` and matching `base_text_hash` /
    `scene_id`.
  - Non-empty spans (`start_byte < end_byte`) must have a non-empty
    `exact_quote` matching `&base_text[start_byte..end_byte]` byte-for-byte.
  - Zero-width insertion spans (`start_byte == end_byte`) must have
    `exact_quote == ""` and at least one non-empty surrounding context
    (`context_before` matching the bytes immediately preceding `start_byte`, or
    `context_after` matching the bytes immediately following `end_byte`).
- **Deterministic and reader-persona feedback ingestion
  (`ingest_and_dedup_findings`)**:
  - Normalizes `CriticFindingInput` items from `continuity`, `anti_slop`,
    `reader_persona`, and `dual_persona` sources, validates any attached
    `CritiqueAnchor` against `base_text`, and deduplicates equivalent or
    overlapping findings (merging `merged_finding_ids` and `sources` while
    retaining the highest `DirectiveSeverity`: `blocker > major > minor > polish`).
- **Author-lock quarantine and priority ordering
  (`validate_and_normalize_critique`)**:
  - Directives that overlap a `MustKeepLine` quote, contradict a `LockedFact`,
    `ProtectedBeat`, or `StylePreference` in `packet.author_locks`, request an
    outcome forbidden by `scene_card.prohibited_outcomes`, or use
    `DirectiveScope::Chapter` / canon-creation phrasing are automatically
    quarantined into `requires_author_decision` and excluded from actionable
    `directives`.
  - Remaining directives are deduplicated by overlapping span/quote, sorted by
    priority tier (`continuity` > `scene_purpose`/`tension_turn`/`pov`/`entry_exit_emotion`/`chapter_role` >
    `voice`/`dialogue`/`freshness`/`length`) and severity, and truncated to
    `MAX_ACTIONABLE_DIRECTIVES = 5`.

### Shadow execution, rating clearance, and bounded 1-shot repair

- **`ModelRouter::critique_draft_shadow` and
  `SqliteSpindleService::critique_artifact_shadow`**:
  - Rejects `EditorialPolicy::Legacy` (`shadow critique requires critique_v1 policy`).
  - Resolves the `review` route through `resolve_cleared_route("review", Some(rating))`
    so unrated or under-cleared agents fail closed with `RouteClearanceError`
    before any prompt is sent.
  - Budgets the critic prompt (`CriticBudget::scene_default`: `input_limit = 16_000`,
    `output_reserve = 1_500`, `framing_reserve = 512`) and verifies token counts
    via the configured calibrated tokenizer when present.
  - Executes at most 1 initial critic call plus at most 1 bounded repair call
    (`max_calls` in `1..=2`) that includes the exact schema/anchor validation error.
    If the repair output is still invalid or truncated, returns
    `ShadowCritiqueOutcome::CriticInvalid` and retains the original draft unchanged.
  - Persists `ShadowCritiqueArtifactV1` (`schema_version = "shadow_critique_artifact_v1"`)
    via atomic tempfile rename and verifies that the underlying
    `PlainTextDraftArtifactV1` bytes and `text_sha256` on disk remain byte-for-byte
    identical before and after shadow critique.
  - Exposed for offline/bridge evaluation via `spindle-tokenizer critique-artifact`
    and verified in `evals/prose_judge.py` (`verify_shadow_critique_parity` and
    `build_blinded_directive_usefulness_packet`).

## Implementation status: prior-draft revision, deterministic candidate checks, and double-order judge acceptance (PR 6)

PR 6 implements the bounded prior-draft revision loop (`RevisionLoopArtifactV1`),
deterministic candidate-check gating (`evaluate_deterministic_checks` and
`compare_candidate_checks`), paragraph-aligned edit-span diffing
(`compute_edit_span_summary`), blinded randomized + swapped-order pairwise judge
acceptance (`PairwiseJudgeOutcome`), shared Python/Rust verdict fixtures, and
stage-boundary crash recovery with compare-and-swap (CAS) promotion.

### Shared Python/Rust verdict fixtures (`evals/prose_judge/shared_verdict_fixtures.json`)

- `evals/prose_judge/shared_verdict_fixtures.json` defines 14 canonical cases
  covering single-pass preference normalization (`passage_1`, `passage_2`, `tie`,
  `1`, `2`, `A`, `B`, `equal`, `neither`), rubric score clamping/validation,
  missing rationale/rubric rejection, and swapped-order double-judging resolution
  (consistent candidate win, consistent incumbent win, consistent tie,
  first-position bias, second-position bias, and tie-flip).
- Enforced by both `cargo test -p spindle-adapters --lib -- shared_python_rust_verdict_fixtures`
  and `python3 evals/prose_judge.py self-test` (`verify_shared_verdict_fixtures`)
  so the online Rust acceptance gate and the offline Python evaluation harness
  share identical verdict semantics.

### Prior-draft revision, deterministic candidate gating, and edit-span discipline

- **Targeted revision prompt assembly (`assemble_revision_messages`)**:
  - Budgets `RevisionBudget::scene_default` (`input_limit = 20_000`,
    `output_reserve = 4_000`, `framing_reserve = 512`) and includes the full
    incumbent draft text (`base_text` with `base_text_hash`), ordered actionable
    directives (`directives`, capped at `MAX_ACTIONABLE_DIRECTIVES = 5`) with
    their validated UTF-8 byte spans and exact quotes, `strengths_to_preserve`,
    protected `author_locks` (`must_keep_line`, `locked_fact`, `protected_beat`,
    `style_preference`), and mandatory packet context (`scene_card`,
    `hard_constraints`, `continuity`, `voice`, `editorial_memory`).
  - Instructs the drafter (`draft` route, cleared via
    `resolve_cleared_route("draft", Some(rating))`) to make targeted surgical
    edits addressing only the directives while preserving unflagged paragraphs
    and locked lines verbatim, returning plain story prose extracted through
    `AcceptedTextExtractor`.
- **Paragraph-aligned LCS edit-span summary (`compute_edit_span_summary`)**:
  - Splits incumbent and candidate prose on `\n\n+` paragraphs, computes a
    longest common subsequence (LCS) over normalized paragraphs, and maps every
    deleted/inserted/modified paragraph span back to exact UTF-8 byte ranges on
    both the incumbent and candidate texts.
  - Records `changed_paragraph_count`, `changed_paragraph_ratio_bps`,
    `changed_byte_ratio_bps`, and `has_scene_scope_directive`, setting
    `broad_change_flagged = true` (and `broad_edit_flagged_for_review = true` on
    the loop artifact) whenever more than `40%` (`> 4000` bps) of incumbent
    paragraphs change without a scene-scoped directive.
- **Deterministic candidate check comparison (`evaluate_deterministic_checks` and
  `compare_candidate_checks`)**:
  - Evaluates hard continuity blockers (`prohibited_outcomes` substring matches
    and violated `MustKeepLine` quotes), hard rating violations (explicit marker
    terms when `packet.rating` is `General` or `Teen`), hard and soft anti-slop
    blockers (`not X, but Y` contrast hinges, `a mix/cocktail/blend of X and Y`,
    `didn't know what tomorrow would bring`, `hours passed`, said-bookism
    clusters, physical-cliché clusters), and ingested findings (projecting
    span-anchored external findings onto candidate prose so rewritten spans are
    counted in `resolved_finding_count` while untouched spans remain active).
  - Immediately disqualifies a candidate before any pairwise judge call
    (`CandidateDisqualifiedByDeterministicCheck`) if the candidate is truncated,
    introduces any new hard continuity/rating/anti-slop blocker, increases total
    hard blockers, or fails to preserve every `MustKeepLine` quote.

### Blinded double-order pairwise judge and stage-boundary crash recovery

- **Blinded randomized + swapped-order pairwise judge
  (`assemble_pairwise_judge_messages` and `judge_revision_pair_double_order`)**:
  - Uses `JudgeBudget::scene_default` (`input_limit = 16_000`,
    `output_reserve = 1_500`, `framing_reserve = 512`) on the rating-cleared
    `review` route.
  - Selects the Pass 1 presentation order (`IncumbentFirst` vs `CandidateFirst`)
    deterministically from `SHA-256(packet_hash : incumbent_hash : candidate_hash : round : seed)`
    and flips the order for Pass 2 (`passage_1` and `passage_2` swap).
  - Prompts are strictly blinded (`Passage 1` vs `Passage 2` only — never
    exposing `incumbent`, `candidate`, `original`, `revised`, or directive notes)
    and include the scene contract, hard constraints, continuity/voice packet
    context, and the exact `prose_judge.py` 7-dimension rubric JSON template.
  - Promotes `DeblindedArm::Candidate` **only** when both Pass 1 and Pass 2
    independently select the candidate (`AcceptedRevision`). Any order
    disagreement (`JudgeOrderDisagreementTie`), explicit tie (`JudgeTie`),
    incumbent win (`JudgePreferredIncumbent`), or invalid/unavailable judge
    output (`JudgeInvalidOrUnavailable`, which also sets `parked_for_review = true`)
    retains the incumbent (`AcceptedOriginal`).
- **Stage-transition ledger, convergence stops, and CAS promotion
  (`SqliteSpindleService::revise_artifact_loop`)**:
  - Persists `RevisionLoopArtifactV1` (`schema_version = "revision_loop_artifact_v1"`)
    atomically at every stage boundary (`Drafted` -> `DeterministicChecked` ->
    `Critiqued` -> `CandidateReady` -> `CandidateChecked` -> `JudgePass1` ->
    `PairwiseJudged` -> `AcceptedRevision` / `AcceptedOriginal`). Resuming after
    a crash at any boundary reloads completed stage outputs from disk without
    replaying prior paid model calls.
  - Enforces per-scene call and token caps (`max_scene_calls = 8`,
    `max_scene_tokens = 65_000`, `max_rounds = 2`) and convergence stops
    (`NoActionableDirectives`, `CriticInvalid`, `IdenticalCandidateHash`,
    `CandidateTruncated`, `CandidateDisqualifiedByDeterministicCheck`,
    `NoDirectiveProgress`, `RepeatedFindingFingerprint`, `BudgetExhausted`).
  - Before promoting a winning candidate into `PlainTextDraftArtifactV1`,
    re-reads the draft artifact under `lock_artifact` and verifies its
    compare-and-swap expectation (`packet_hash`, `initial_text_sha256`, and
    `source_versions`). On any mismatch, fails closed with `StaleCasConflict`
    and leaves the on-disk draft artifact untouched.
- **Verification and live evaluation status**:
  - Continuity recall gate **C** (`python3 evals/continuity_recall.py run --no-build`)
    remains at `20 / 28 = 0.7143` overall recall (`5` control false positives,
    `4 / 4 = 1.0000` accepted-prose violating probe recall, `0` clean blocker
    issues across `90` live artifacts).
  - Live `spindle-tokenizer revise-artifact` execution on `gemma4:31b` across a
    6-scene stratified sample (`PJ01`, `PJ02`, `PJ07`, `PJ13`, `PJ19`, `PJ25`,
    seed `101`) exercises end-to-end deterministic checking, critic anchor
    repair, surgical revision (`5.88%` paragraph / `6.37%` byte change on
    `PJ02`), and swapped-order judge retention (`mean = 2.5` calls/scene, `0`
    broad-edit flags). `EditorialPolicy::Legacy` remains the default policy.

## Implementation status: hybrid host tasks, CLI, and durable budget ledger (PR 7)

PR 7 adds the durable editorial-controller contract, hybrid host tasks, agent
parity service methods, prose-free attribution, and the harness configuration
and status/dry-run seams. `EditorialPolicy::Legacy` remains the default; nothing
prose-affecting is activated by this step.

### Core contracts and durable budget ledger

- `spindle_core::drafting::editorial` defines `EditorialActionKind`,
  `EditorialBudgetCaps`/`Usage`/`EditorialBudgetReservation`,
  `EditorialBudgetLedgerV1`, `HostEditorialTaskV1`,
  `HostEditorialSubmissionV1`, `HostSubmissionOutcomeV1`,
  `ResolvedEditorialSettingsV1`, `EditorialControllerStateV1`,
  `EditorialSceneStateV1`, `EditorialNextActionV1`, and
  `EditorialAttributionV1`.
- Default ceilings are the design values: per-scene `12` calls, `210k` tokens,
  `720s`, `$2.00`; per-run `200` calls, `3M` tokens, `90` minutes, `$20`.
  Reservations are exact proposed calls (`draft` 1, `critique` 1, `revision` 1,
  two-order judge 2, line edit 1, extraction 1) and are admitted only when every
  scene and run ceiling still holds. Re-admitting the same
  `(run, branch, scene, packet, stage, attempt, action)` key is idempotent.
- `usage_known = false` (host completion or lost provider usage) keeps the full
  reservation charged; it never fabricates zero usage. A paid dispatch with no
  configured model price is denied with `missing_price` instead of pretending
  the monetary cap was verified.
- The controller is a single JSON artifact saved with a temp-file rename and a
  per-path lock. Stage changes and reservations are written in the same locked
  transaction, so a restart cannot reset counters and concurrent owners cannot
  double-admit. Each paid reservation has durable per-pass dispatch claims: an
  in-flight pass cannot be reused, a failed paid attempt consumes its attempt,
  and a retry must pass the caps again as a new admission. A two-call judge
  reservation claims `judge_pass_1` and `judge_pass_2` separately, so a crash
  between the swapped-order passes can dispatch the remaining reserved pass
  without replaying pass 1. Pass 2 is claimed on uninterrupted runs too, so a
  concurrent resume cannot duplicate the in-flight request. Host judge
  admission also runs inside the same locked transaction.
- `EditorialAttributionV1` is prose-free: it records stage/attempt, stable
  prompt template id and version, prompt/context/packet/source hashes,
  tokenizer identity, parent/candidate hashes, judge order, outcome, durations,
  actual and reserved tokens/cost, and linked `model_call` ids. Prose stays in
  the existing access-controlled draft artifacts.

### Hybrid host tasks, agent parity, and judgment

- `prepare_host_editorial_task` snapshots the exact packet and, for revision,
  the incumbent text, selected directives, and strengths. The host task carries
  the packet/hash and a stable `task_id`; retrieval through
  `retrieve_host_task_packet` refuses a submitted task, a mismatched packet
  hash, a stale branch, or a missing reservation, so an MCP-pull agent cannot
  bypass the budget or branch snapshot.
- Host submissions are validated against task id, packet hash, exact bytes,
  emptiness, truncation, and receipt. Host prose is stored with an explicit
  `host:<task_id>` receipt (`agent_id = "host"`, exact and normalized hashes)
  and unknown usage keeps the reservation.
- Revision submissions run the existing deterministic candidate checks before
  any judge call. A disqualified candidate retains the incumbent. A missing,
  uncleared, tokenizer-less, or invalid judge route returns
  `pending_manual_judgment` and retains the incumbent; it is never treated as a
  candidate preference. A valid swapped-order judge still requires both passes
  to prefer the candidate before promotion.
- Agent mode exposes the same first-draft and bounded revision-loop entry points
  (`execute_editorial_agent_draft`, `execute_editorial_agent_revision_loop`) over
  the same prepared packet and artifacts, recording prose-free attribution for
  each persisted stage transition. The draft reservation is persisted before
  dispatch and reconciled afterwards (a failed dispatch keeps the maximum
  charged). The revision loop admits each model pass (critic with its bounded
  repair, revision, both judge orders) through a durable controller gate before
  dispatch and reconciles it afterwards; a failed pass keeps its maximum
  charged, and exhausted scene/run caps or missing pricing block dispatch.
- Successful judge reconciliation records measured elapsed time and cost from
  the configured per-token prices. Unknown usage is never replaced with zero
  tokens/cost/time; the conservative reservation remains charged.

### Harness CLI, status, and legacy compatibility

- `HarnessSeed` accepts an optional `editorial` object
  (`editorial_policy = legacy | critique_v1`, execution mode, packet/rubric
  versions, round cap, line-edit and chapter-pass flags, scene/run budgets,
  optional pricing, generation seed) plus an optional
  `editorial_controller_path`. `HarnessState` persists the resolved settings;
  absent or `legacy` configs resolve to `None` and emit no new output.
- `init` persists resolved settings. `status` reports incumbent/candidate,
  critic/judge outcome, parked issues, remaining scene/run caps, pending host
  task, pending manual judgment, and the next action with its reservation and
  any missing route/tokenizer. `verify` validates the controller artifact
  without model calls. `resume` dry-run reports the next action, reservation,
  and missing route/tokenizer without dispatching.
- `resume --execute-one` advances one persisted editorial action. The CLI can
  accept a hybrid draft submission (`--host-submission <json>`) and preserves
  its exact bytes plus receipt. A revision submission or an agent model pass is
  reported as requiring server-side checks/judging/dispatch; the CLI never
  silently skips those gates. Agent dispatch remains an internal service seam;
  MCP tool naming and wiring are deferred to P4 as the design specifies.
- Legacy runs continue to resume with exact output parity. The feature remains
  opt-in; release gate **G** has not been re-run for this step and the prior
  one-shot packet result (`0.4444`, CI `[0.3667, 0.5167]`) still leaves
  `EditorialPolicy::Legacy` as the default. Continuity gate **C** remains
  `20 / 28 = 0.7143` overall recall with `5` control false positives and
  `4 / 4` accepted-prose violating probes across `90` verified live artifacts.

## Implementation status: optional line edit and bounded chapter pass (PR 8)

PR 8 adds the optional line-edit pass, the bounded chapter advisory critique with
author-selected edits, repetition/editorial memory, checkpoint-feedback adapters,
and the book advisory queue. Both optional model passes remain **off by default**
(`line_edit_enabled = false`, `chapter_pass_enabled = false`); nothing here
activates a prose-affecting default.

### Optional line edit

- `crates/spindle-adapters/src/drafting/line_edit.rs` defines the `line_edit`
  system prompt, `LineEditBudget` (18,000 input / 3,000 output / 512 framing),
  and token-verified prompt assembly. The prompt permits diction, syntax,
  rhythm, and dialogue polish only and requires preserving every event, the
  scene turn, POV, rating, and author locks.
- `ModelRouter::line_edit_candidate` resolves the rating-cleared `line_edit`
  route with **no fallback to `draft`**; a missing route is an honest skip. The
  candidate still runs the existing deterministic checks and the blinded
  swapped-order judge. A new candidate is promoted only when both passes prefer
  it; any tie, incumbent win, order disagreement, truncation, new blocker, or
  unavailable judge retains the pre-line-edit accepted prose. The pass is
  marked attempted either way and its ledger/attribution rows are recorded.
- The harness already exposes `line_edit_enabled` in resolved settings; the
  controller's next-action report returns `line_edit` only when the terminal
  scene has not attempted it. Tests cover the disabled/no-route skip (accepted
  prose byte-identical) and judge-tie retention (both judge passes charged).

### Bounded chapter pass

- `plan_chapter_windows` splits a chapter into deterministic windows (default 4
  scenes, hard cap 8) with an explicit index and coverage report; it never
  silently truncates. The full chapter index and coverage are mandatory in the
  chapter-critic prompt, and an over-budget chapter fails closed instead of
  claiming a full review.
- `ChapterCritiqueV1` covers openings/endings, repetition, beat/intensity
  progression, scene lengths, reader engagement, continuity, and chapter
  coherence. Concerns carry exact scene/hash/UTF-8 anchors; a missing or
  unverifiable anchor becomes `advisory = true` and never drives an edit.
  Invalid proposal anchors are dropped, and at most two scene-edit proposals
  survive validation.
- `run_editorial_chapter_critic` admits one `ChapterCritique` pass through the
  durable controller ledger, persists the validated critique as a local
  artifact, and returns it. A missing route/tokenizer/price returns an honest
  skip; no prose or plan is mutated.
- Applying edits requires an explicit `ChapterEditSelectionV1`. Each selected
  proposal is converted to a scene `CritiqueDirective` and flows through the
  existing deterministic-check + doubled-order judge loop with
  `pre_selected_directives` (the scene critic is skipped) and bounded chapter
  context included in both judge orders. Unselected proposals are never
  applied; stale base hashes/scene mismatches are skipped. Acceptance returns a
  `ChapterInvalidationPlanV1` covering the chapter summary, scene extraction,
  beat annotations, reader notes, re-extraction, and downstream revalidation.
  The chapter-level rejection test verifies a judge tie keeps the accepted
  prose and returns no invalidation.
- Editor/reader feedback adapters map retained reader-simulation and dual-persona
  notes into unanchored `CriticFindingInput`s for the critic.

### Repetition memory and book advisory queue

- `build_editorial_memory` selects accepted scenes only, excludes every rejected
  candidate hash, and combines the current chapter's earlier scenes with the
  previous eight accepted scenes. It builds bounded recent openings/endings,
  repeated phrases, recurring images, and pacing history with scene/hash
  provenance. Tests verify rejected candidates never enter memory.
- `build_book_advisory_queue` reports chapter-summary gaps, large pacing drops,
  repeated opening/closing devices, and unresolved promises whose planned
  payoff has passed. Every item is `advisory = true`; there is no automatic book
  rewrite, plot redesign, or plan mutation. The queue can be persisted beside
  the run artifacts with `save_book_advisory_queue`.

### Gates and defaults

- Legacy remains the default. Line edit and the chapter pass stay opt-in and
  off; bounded chapter edits additionally require an explicit author selection.
- Release gate **G** has not been run for line edit or the chapter pass. The
  design's chapter-coherence blinded comparison lower-bound `> 0.50` gate is
  therefore not cleared, so neither optional pass is activated by default.
- Continuity gate **C** remains `20 / 28 = 0.7143` overall recall with `5`
  control false positives, `4 / 4` accepted-prose violating probes, `90`
  verified live artifacts, and `0` clean blocker issues.
- No SQLite migration, MCP tool, skill/reference file, or `tools/list` surface
  change is introduced by this step.
