# Tool surface and skill alignment (Wave 2 / P4)

Status: accepted; implementation in progress. Baseline: `origin/main` at
`419f40be7378d8919ac89d28139496d06b7db935`, measured 2026-11-08.
The measurements below are the original design baseline. Current implementation
changes are tracked in the implementation checkpoints at the end.

## Decision and scope

Make ordinary scene work **prepare → write/review locally → commit**: one bounded
`prepare_scene` read and one `commit_scene` write. Exceptional evidence lookups,
failed validation, editorial revisions, and chapter/run checkpoints may add calls;
this is not a promise that every scene takes exactly two requests.

Generate profiles from the same manifests that declare each skill's tools. Keep
branch/revision functionality discoverable in a specialist profile, reduce schema
and skill load, and move machine administration behind operator CLI commands.
Public branch and revision workflows must survive this work: that requirement is
explicit in [`CLAUDE.md:11`](../CLAUDE.md#L11).

This design owns interface composition, discovery, compatibility, and budgets.
P1 owns drafting-packet selection and critique/revise behavior; P2 owns extraction,
event/memory semantics; P3 owns planning. Their future payloads plug into the
contracts below; P4 does not prescribe their algorithms or claim they have shipped.
Implementation starts only after S's behavior-preserving `tools.rs` split lands.
The existing crate boundaries remain: public DTOs in core, assembly/business rules
in adapters, MCP transport in MCP, static skills in spindle-skills
([architecture:52](spindle-architecture.md#L52),
[implementation brief:77](spindle-implementation-brief.md#L77)).

## Measured baseline and corrected audit

### Method

Built this commit with `cargo build -p spindle-mcp`. Ran the resulting binary over
stdio once per profile, using a **different `/tmp/spindle-p4-*/<profile>`
`SPINDLE_DATA_DIR` and empty `SPINDLE_CONFIG`** for each process, with its working
directory there. Unset inherited `SPINDLE_*` settings first, including HTTP mode.
No real user database, config, or installed skill directory was used. Sent MCP
`initialize` (protocol `2024-11-05`), `notifications/initialized`, then `tools/list`;
closed stdin after the response. The override behavior is implemented in
[`main.rs:99`](../crates/spindle-mcp/src/main.rs#L99) and
[`workspace.rs:44`](../crates/spindle-adapters/src/workspace.rs#L44).

Sizes below serialize compact UTF-8 JSON (`ensure_ascii=False`, separators `,` and
`:`), preserving returned array order. Input size is the array projected onto
`name`, `description`, `inputSchema`; full size is the entire returned tools array,
excluding the JSON-RPC wrapper. Estimated tokens are **ceil(bytes / 4)**, a stable
budget proxy, not an actual host tokenizer count. They should not be compared as
exact tokenizer measurements to the earlier audit's ~32k/~19k estimates.

| Profile | Tools | Input bytes | Input token estimate | Full bytes | Full token estimate |
| --- | ---: | ---: | ---: | ---: | ---: |
| default (unset) | 170 | 137,800 | 34,450 | 557,658 | 139,415 |
| minimal | 19 | 15,683 | 3,921 | 135,091 | 33,773 |
| import | 39 | 19,492 | 4,873 | 206,761 | 51,691 |
| write | 121 | 105,491 | 26,373 | 389,109 | 97,278 |
| authoring | 76 | 81,082 | 20,271 | 278,295 | 69,574 |

Default output-schema values alone total 417,138 bytes, **74.8%** of full bytes.
`get_chapter_briefing` output schema is 36,203 bytes and `get_scene_context` is
30,733 bytes under this serialization (the older ~39KB/~33KB figures used a
different representation). `generate_alternatives` is also large at 32,323 bytes.
Every tool currently attaches an output schema, with inline input subschemas
([`tools.rs:7383`](../crates/spindle-mcp/src/tools.rs#L7383)).

To reproduce, use an absolute binary path and a small JSON-lines client. This
pseudocode is intentionally sufficient to avoid any default data/config lookup:

```python
root = tempfile.mkdtemp(prefix="spindle-p4-", dir="/tmp")
for profile in [None, "minimal", "import", "write", "authoring"]:
    d = Path(root) / (profile or "default")
    d.mkdir()
    (d / "config.toml").write_text("")
    env = {k: v for k, v in os.environ.items() if not k.startswith("SPINDLE_")}
    env.update(SPINDLE_DATA_DIR=str(d), SPINDLE_CONFIG=str(d / "config.toml"))
    if profile:
        env["SPINDLE_TOOL_PROFILE"] = profile
    # Popen([absolute_binary], cwd=d, env=env, stdin=PIPE, stdout=PIPE)
    # Write one JSON object + newline per message, flush, read matching id.
    # initialize id=1, notifications/initialized (no id), tools/list id=2.
    # Project response.result.tools, serialize as above, measure len(utf8).
    # Close stdin, wait with timeout; terminate the child on failure.
```

CI will implement the complete client rather than rely on retained `/tmp` files.
No model calls or populated manuscript are necessary to measure discovery size.

### What remains true, and what changed

- **Schema rebuilding is fixed.** `all_tool_schemas` uses `OnceLock<Vec<Tool>>` and
  `has_tool` uses cached names plus the profile allowlist
  ([`tools.rs:333`](../crates/spindle-mcp/src/tools.rs#L333),
  [`tools.rs:366`](../crates/spindle-mcp/src/tools.rs#L366)). Dispatch calls
  `has_tool`, not schema construction
  ([`tools.rs:1626`](../crates/spindle-mcp/src/tools.rs#L1626)). Retain this seam;
  caching reduces CPU, not advertised context bytes.
- **Profiles still contradict instructions.** `set_active_project` is missing from
  import, write, and minimal; authoring includes it. Initialize instructions and
  scene-writer both require it first
  ([`tools.rs:384`](../crates/spindle-mcp/src/tools.rs#L384),
  [`server.rs:109`](../crates/spindle-mcp/src/server.rs#L109),
  [`scene-writer:61`](../skills/scene-writer/SKILL.md#L61)).
- **Eight scene-writer names are absent from authoring:** `continue_generation`,
  `revise_generation`, `find_scenes_referencing`, `pull_chapter_from_file`,
  `push_chapter_to_file`, `research_pack_for_scene`, `update_relationship`, and
  `update_world_rule`. This is literal tool-name matching against live discovery,
  including conditional workflows, not a claim that all eight are needed on
  every scene. The existing test checks skill-loading tools and a hand-picked
  authoring subset, not all skill references
  ([`tools.rs:8388`](../crates/spindle-mcp/src/tools.rs#L8388)).
- **33 names occur in no named profile** (they are still in default). This includes
  branch creation/switch/diff/merge, alternatives, revision markers, scene versions,
  `revise_scene`, and `set_narrator_voice`. Save-point tools are an exception:
  write/authoring already include them. The gap is not accurately described as
  *all* branch/revision-related tools being absent. Profile filtering is also
  invocation filtering, so hidden tools cannot simply be called by name
  ([`tools.rs:348`](../crates/spindle-mcp/src/tools.rs#L348)).
- **Import contains 18 style tools out of 39.** The wider catalog has 19 style
  tools; these include preview/apply/rollback families, but not every style tool
  is a triplet ([`tools.rs:408`](../crates/spindle-mcp/src/tools.rs#L408),
  [`tools.rs:764`](../crates/spindle-mcp/src/tools.rs#L764)).
- **Many prose entry points share insufficiently visible semantics.** Examples
  are draft save, revision, alternative selection, scene-version restore, source
  pull, style-patch apply, and authoring save
  ([`service.rs:23200`](../crates/spindle-adapters/src/sqlite/service.rs#L23200),
  [`service.rs:13617`](../crates/spindle-adapters/src/sqlite/service.rs#L13617),
  [`service.rs:14473`](../crates/spindle-adapters/src/sqlite/service.rs#L14473),
  [`service.rs:4176`](../crates/spindle-adapters/src/sqlite/service.rs#L4176),
  [`service.rs:17047`](../crates/spindle-adapters/src/sqlite/service.rs#L17047),
  [`style_service.rs:2815`](../crates/spindle-adapters/src/sqlite/style_service.rs#L2815),
  [`tools.rs:4431`](../crates/spindle-mcp/src/tools.rs#L4431)). These are seven
  entry points, not seven proven independent repository implementations.
- **Canon has multiple workflows:** direct fact registration, scene extraction,
  and proposed mining/ratification
  ([`service.rs:3987`](../crates/spindle-adapters/src/sqlite/service.rs#L3987),
  [`service.rs:14546`](../crates/spindle-adapters/src/sqlite/service.rs#L14546),
  [`tools.rs:1133`](../crates/spindle-mcp/src/tools.rs#L1133)). Preserve the
  distinction between explicit author decisions and unratified model proposals.
- **Undo scopes differ:** branch save-point restore, scene-version restore, and
  style application/patch rollback
  ([`service.rs:16805`](../crates/spindle-adapters/src/sqlite/service.rs#L16805),
  [`service.rs:4176`](../crates/spindle-adapters/src/sqlite/service.rs#L4176),
  [`style_service.rs:1622`](../crates/spindle-adapters/src/sqlite/style_service.rs#L1622),
  [`style_service.rs:3006`](../crates/spindle-adapters/src/sqlite/style_service.rs#L3006)).
- **Naming duplicates decisions.** `update_entity` coexists with specific updates
  and four selected batch variants
  ([`tools.rs:449`](../crates/spindle-mcp/src/tools.rs#L449),
  [`tools.rs:498`](../crates/spindle-mcp/src/tools.rs#L498)). `operator_delete_scene`
  currently delegates to `delete_scene` in SQLite; its name is not an authorization
  boundary ([`service.rs:3166`](../crates/spindle-adapters/src/sqlite/service.rs#L3166)).
- **Administration is model-visible in default:** `init_grok_skills`,
  `configure_agents`, `rebuild_search_index`, and `backfill_scene_source_offsets`
  appear in the live catalog; backfill is also in write
  ([`tools.rs:505`](../crates/spindle-mcp/src/tools.rs#L505)).
- **Description coverage is worse under a precise nested-property count:** 970 of
  1,208 input-schema property occurrences (80.3%) lack a nonempty description.
  Count recursively under `properties`, including repeated inline objects;
  this is not a count of unique Rust fields. The earlier 78% is directional.
  Closed policies are strings despite runtime validation, and their docs contain
  “evolution §”, “ADR”, and “(I1)”
  ([`models.rs:6733`](../crates/spindle-core/src/models.rs#L6733),
  [`models.rs:7142`](../crates/spindle-core/src/models.rs#L7142)).
  `research_query` says Gemini in discovery, though implementation uses configured
  route machinery ([`tools.rs:1078`](../crates/spindle-mcp/src/tools.rs#L1078),
  [`service.rs:15186`](../crates/spindle-adapters/src/sqlite/service.rs#L15186)).
- **Skill load is large and disconnected from discovery.** Thirteen `SKILL.md`
  files total 278,318 UTF-8 bytes (~69,580 proxy tokens); scene-writer is 58,487
  bytes (~14,622). Literal matching finds 27 catalog tools unmentioned by any
  skill, including `create_project` and `set_narrator_voice`. This measures
  mentions, not successful guidance. Entry text is explicitly Codex/GPT-6-specific
  and gives reconnect instructions without an explicit new-book setup path
  ([`spindle:6`](../skills/spindle/SKILL.md#L6),
  [`spindle:32`](../skills/spindle/SKILL.md#L32)).
- **The normal instructions require three overlapping context reads:** writer
  state, chapter briefing, scene context, followed by separate save, validation,
  canon, beat, summary, and cursor steps
  ([`scene-writer:57`](../skills/scene-writer/SKILL.md#L57),
  [`scene-writer:138`](../skills/scene-writer/SKILL.md#L138)). The audit's 10–12
  calls/scene is a workflow estimate, not a measured runtime distribution.

## Target surface

### Scene contracts

`prepare_scene` accepts project/branch defaults, a typed scene ID or new-scene
placement, intent (`draft | revise | continue`), and `budget_tokens`. It returns:

- Resolved project/branch, placement, writer cursor, and scene version/context
  revision token; a new scene receives a stable preparation ID.
- P1's bounded drafting packet: applicable plan/scene purpose, hard constraints,
  reader/style/narrator contract, character knowledge and state, continuity,
  preceding prose/voice anchors, and relevant research. Reuse current assemblers;
  do not concatenate their three full outputs or repeat JSON as Markdown.
- Coverage, omitted/truncated sections, focused evidence references, and honest
  unavailable-pass status. Hard constraints must not silently disappear to fit a
  budget; return a budget error with required minimum if they cannot fit.

It is read-only: preparing a new scene does not advance the cursor, persist a
placeholder, start generation, or approve canon. P2/P3 provide future memory and
planning contributions via adapter interfaces. Session re-anchoring uses the
resolved state in this packet; `get_writer_state` remains a resume/dashboard read.

`commit_scene` accepts the preparation ID/token, expected scene version, full
prose, authorship/provenance and generation receipt, typed continuity package,
beats, optional chapter-completion summary, `dry_run` (default false), and a
client-generated idempotency key. It returns a compact receipt: operation ID,
scene/version/branch, validation findings and coverage, applied deltas, cursor,
follow-ups, and references to large diffs/artifacts. No model-generated prose is
returned unless explicitly requested through a separate read.

All scene text persistence converges on one adapter service boundary. It checks
branch ownership, stale packet/scene version, provenance/rating rules, and
existing validation gates before writing prose + approved continuity + beats +
cursor atomically in SQLite. Required deterministic checks run inside this path;
model critique belongs to P1 before commit and is never an unbounded transaction.
A failed gate returns findings with no partial writes; advisory findings may
accompany a successful commit, labeled as such. A summary can be omitted on a
non-final scene; chapter-final follow-ups remain explicit if unavailable.

`dry_run=true` performs the same preflight and reports the proposed effects,
without a version, receipt for mutation, journal event, cursor movement, external
model call, or write. A subsequent live call revalidates; a preview grants no
lease. Replaying the same idempotency key/payload returns the original receipt;
reusing it with different content fails. Packet validity includes dependencies
on canon/style/plan state, not just the scene text hash.

For an authoring run, the same request may carry a run ID and expected phase;
the harness uses this boundary and advances its existing phase machine only on
success. Do not bypass checkpoints or ratification. Legacy save-only calls keep
their save-only semantics during migration rather than unexpectedly finalizing
canon. `revise_scene`'s current non-main guard
([`service.rs:13636`](../crates/spindle-adapters/src/sqlite/service.rs#L13636))
is preserved for revision intent and aliases until separately changed.

### Registry disposition (canonical names are proposed, not shipped)

Avoid a single giant `action: string` dispatcher. Keep small domain operations,
typed DTOs, and dedicated specialist profiles. Every old registry entry must have
exactly one retained, alias, or operator disposition in the implementation ledger.
The following families define the target, including public advanced workflows.

| Family | Target model surface | Disposition of existing names |
| --- | --- | --- |
| Entry/setup | `list_skills`, `get_skill`, `get_reference`, `list_projects`, `set_active_project`, `get_writer_state`, `create_project`, `create_book`, `create_chapter` | Retain; put project selection in every profile. |
| Ordinary scene | `prepare_scene`, `commit_scene`, `check_consistency` | Context reads become compatibility adapters; save/revise/authoring-save and accepted alternative/style/source writes converge on the commit service. Keep focused `get_entity`, `find_entity`, `search_bible`, `find_scenes_referencing`, `get_character_snapshot`, `list_chapter_scenes`, `list_book_chapters` for missing evidence. |
| Scene organization | `move_scene`, `delete_scene` with `dry_run` impact report | Fold delete impact into dry run; operator bypass moves to CLI. Keep `save_summary`, `annotate_scene_beats`, `record_note`, `update_writer_position` for targeted repair, outside the normal loop. |
| Typed world building | Existing typed `create_*`, `update_world_rule`, `update_relationship`, `update_promise_status`, `update_arc_milestone`, voice/calendar/clock/quantity setters and `archive_entity` | Retain in relevant world/character/planning skills. Replace open `update_entity` with typed `update_character`, `update_location`, `update_conflict`, `update_character_arc`, etc. only for its supported cases; inventory fields before migration. No arbitrary table/column edits. |
| Canon | `propose_canon_changes`, `list_canon_deltas`, `decide_canon_deltas`; explicit `register_canonical_fact`, `record_knowledge`, `commit_character_state`, `commit_quantity_state`, `bind_canonical_fact_to_scene`, `migrate_canonical_fact` | Extraction/mining map to proposal generation owned by P2. Routine author-declared changes travel in commit; independent author canon edits remain explicit. Do not auto-ratify mined changes. |
| Branch/revision | `create_branch`, `switch_branch`, `diff_branches`, `merge_branch`, `create_save_point`, `generate_alternatives`, `compare_alternatives`, `select_alternative`, `list_revision_markers`, `resolve_revision_marker` | Retain in revision profile. Selection accepts through shared commit boundary. Restore/list-version names map to the history/undo contract below with original scope preserved. |
| Style | `create_style_profile_from_markdown`, `list_style_profiles`, `get_style_profile`, `check_style_profile_sources`, `refresh_style_profile`, `apply_style_profile`, `check_style_against_profile`, `plan_style_revision`, `compare_style_profiles`, `archive_style_profile`, `create_style_revision_patch`, `evaluate_style_revision_patch`, `apply_style_revision_patch` | Preview twins become `dry_run` on refresh/apply. Patch creation produces a non-mutating artifact; acceptance writes via commit. Application/audit listings and both rollbacks map to history/undo. Style has its own skill/profile, no unconditional import inclusion. |
| Runs/plans | Existing `authoring_prepare_run`, `authoring_start_run`, `authoring_status`, `authoring_execute_next`, checkpoint audit/review, resolve/cancel; `plan_chapter`, outlines, pacing/arc tools; replan/list/decide amendments | Retain orchestration and P3 plan seams; authoring-save aliases commit. Plans and proposals never silently expand commit authority. |
| Research/generation | Existing research source/note/claim/search/pack/usage/report/plan tools and `research_query`; `continue_generation`, `revise_generation`, read-only `list_agents` | Retain in researcher or offload reference closure. Describe configured routes without a provider claim. Generation receipt validation survives consolidation. |
| Import/source | Existing import lifecycle and review tools; `pull_chapter_from_file`, `push_chapter_to_file` | Retain in import/source-sync specialist closure; imported prose uses the shared commit machinery with import provenance, not fabricated assistant receipts. Backfill moves to CLI. |
| Release/export | Existing compile/preflight/export, episode read/release/status, editorial queue/decision, and usage tools | Retain in release/editor skills; do not inject into minimal scene work. |
| Administration | None in model profiles | `init_grok_skills`, `configure_agents`, `test_agent`, `rebuild_search_index`, `backfill_scene_source_offsets`, and operator-delete bypass become CLI commands. Existing `init-grok-skills` CLI is reused, not duplicated. |
| History | `list_changes`, `undo_change` | One discoverable recovery vocabulary, scoped as below. |

Batch policy: typed mutations accept `items` with 1–50 entries, `dry_run`, and
idempotency key where batching is useful. Prefer one canonical schema per operation
over permanent single/batch twins. Preflight every item; commit all-or-nothing in
one branch, return indexed errors and receipts. Cross-branch batches are rejected.
The four existing `batch_*` names alias the matching typed operation for one
release. Do not implement an arbitrary heterogeneous `batch_call_tool`; it would
hide authorization and transaction boundaries. Existing batch semantics must be
captured in compatibility tests before adapters are written.

### One undo interface, explicit scopes

Every supported mutation returns an operation ID and `undo_support`:
`reversible | checkpoint_only | irreversible`, with a reason for the latter two.
`list_changes` filters project/branch/scene/kind and paginates compact receipts.
`undo_change(operation_id, expected_head, dry_run)` reports affected objects and
creates a compensating operation after checking dependencies. It never deletes
audit history or silently overwrites intervening edits. If later changes depend
on an operation, refuse with affected IDs and a recovery path; no generic force.

Scene commits restore prose **and their approved derived state** coherently;
style undo restores only the captured style effects; branch checkpoint restore
restores the branch snapshot. Undo is branch-local and uses optimistic revision
checks. Restoring an old scene version must not rewind unrelated branch changes.
`restore_scene_version`, `restore_save_point`, and style rollback aliases keep
their original inputs and scope, resolving to explicit history operations, not
pretending that a scene version ID is a branch checkpoint ID. Old snapshots
without sufficient prior state are labeled checkpoint-only/manual recovery.

This receipt index is a P4 interface requirement, not P2's canon event-store
schema and not the existing authoring event journal. Implementation must establish
which mutations can be compensated transactionally before marking them reversible.
Exports, model dispatch, and external file pushes cannot be magically undone:
record their effect and limits; file sync requires conflict checking and its own
backup policy. One recovery vocabulary does not imply universal reversibility.

## Profiles generated from skill manifests

Add `skills/<name>/manifest.toml` beside each skill. It declares versioned tool
names, skill dependencies, separately loadable references, and profile membership.
Example target manifest (illustrative; these names are not available today):

```toml
version = 1
name = "scene-writer"
profiles = ["write", "authoring"]
requires_skills = ["spindle"]
tools = ["prepare_scene", "commit_scene", "check_consistency"]

[[references]]
name = "offloaded-generation"
path = "references/offloaded-generation.md"
tools = ["continue_generation", "revise_generation", "list_agents"]
profiles = ["authoring"]
```

The build generates sorted, deduplicated profile allowlists from these manifests
and a small profile composition file listing enabled skill/reference units. Tools
must resolve against one typed registry; unknown tool names, duplicate definitions,
dependency cycles, or operator tools in manifests fail the build. Keep static
embedding in spindle-skills; registry validation belongs to MCP tests to avoid a
skills → MCP dependency. The current build embeds only each `SKILL.md`
([`build.rs:23`](../crates/spindle-skills/build.rs#L23)); extend packaging to embed
manifest metadata and named nested references deterministically. No runtime
filesystem or network access is needed to load them.

For every profile, take the transitive union of all enabled core skills and their
enabled references. The shared `spindle` entry dependency always includes project
listing/selection, writer-state, and skill/reference discovery. Core skill files
must not prescribe optional tool calls: move conditional workflows into named
references with their own declarations. In the example, write loads only the core;
authoring can also load the offload reference. Dependencies cannot be marked
“optional” in prose while silently omitted from the profile.

`get_skill`/`get_reference` report `required_profile`, available units, and missing
capabilities. They return actionable workflow instructions only when the current
profile covers the requested unit; otherwise return a concise profile-selection
message and metadata. Merely reading a skill must not grant access or mutate the
profile. Use startup profile selection and reconnect for v1, with no new dynamic
`enable_tools` control plane. Skill delegation (entry points at specialist skills)
is routing metadata, not a requirement to union every specialist into every profile.

Define profiles as task entry points:

| Profile | Enabled work |
| --- | --- |
| minimal | Entry/reconnect and bounded scene core; no import, release, or model-route administration. |
| write (new default after compatibility release) | Scene core, focused lookups, new-book setup, ordinary world/character setup, targeted repairs. |
| authoring | Write spine plus run/checkpoint orchestration and declared offload references; no release/export unless explicitly composed. |
| import | Entry + importer + review + source-sync references; style is an explicit add-on, not half the profile. |
| revision | Entry + branch/alternatives/history/revision tools. |
| worldbuilding / planning | Entry + worldbuilder/character-creator or plot-architect; P3 tools as they land. |
| style / research / release | Entry + corresponding specialist skill. |
| legacy-all | Frozen compatibility catalog for one release; never a normal recommendation. |

Composed profiles must satisfy their own budget; unioning skills is not an escape
from limits. List profiles and required capabilities in CLI help and skill metadata.
Reject unknown profiles at startup instead of the current fallback to all tools
([`tools.rs:338`](../crates/spindle-mcp/src/tools.rs#L338)). Profile filtering stays
consistent across `tools/list` and `tools/call` on stdio and HTTP. It is a discovery
boundary, not a replacement for operator authentication.

### CI alignment contract

Add `skill_profile_alignment` to enumerate the embedded core/reference units and
actual profile routers. For every enabled unit, assert every declared tool is
listed **and callable by name** in that profile; dispatch availability can be
checked without executing mutations. Recursively scan code spans and fenced call
examples for registry tool names, fail if a named tool lacks a declaration, and
require machine-readable `tool:` markers for normative instructions. Check markers
for unknown names too, so typos cannot vanish from a registry-only scanner. Negative
examples/alias migration tables require explicit non-normative markers.

Run the check against core and on-demand references, not just `SKILL.md`. Also
assert every model tool has at least one owning skill or an explicit compatibility
classification; operator tools have CLI coverage instead. This prevents a new set
of 27 undocumented tools. Fixtures must prove failures for an unavailable
`set_active_project`, missing offload dependency, misspelled name, operator tool,
and newly added unowned tool. Snapshot generated metadata and reject hand edits.

## Schema hygiene and discovery cost

1. Require a useful description on every tool, input property (nested and optional
   included), and exposed output property. Describe meaning, units/ranges, defaults,
   branch scope, and consequences where relevant; a copy of the field name fails
   review. IDs explain what creates/resolves them. Describe conditional requirements.
2. Closed vocabularies use Rust enums with stable serde names and JSON-Schema enums;
   free text remains a string. Start with authoring mode and mining/checkpoint/replan
   policies. Preserve currently accepted normalized strings and omission semantics
   in compatibility parsing; do not infer valid values from stale documentation.
   At this baseline the validators accept `disabled | propose_all` for mining and
   replan and `manual | auto_advisory | auto_strict` for checkpoints
   ([`models.rs:7142`](../crates/spindle-core/src/models.rs#L7142)).
3. Remove implementation-history shorthand from model-visible descriptions:
   `evolution §...`, ADR numbers, invariant labels such as `(I1)`, SQL column names
   without user meaning. Keep developer rationale in Rust comments that are not
   schema documentation. Replace provider promises with “configured research
   route”; provider adapters remain documented for operators.
4. Keep input schemas small and typed; use reusable definitions where client
   compatibility allows. Never shrink a schema by erasing validation with an
   unbounded object. Preserve current numeric-format sanitation and strict-client
   tests ([`tools.rs:7397`](../crates/spindle-mcp/src/tools.rs#L7397),
   [`tools.rs:8657`](../crates/spindle-mcp/src/tools.rs#L8657)).
5. Prefer compact typed receipt output schemas. For large context/analysis results,
   omit `outputSchema` from discovery and publish a versioned full response contract
   as an optional reference/resource. Continue Rust DTO validation and stable JSON
   results; omission must not silently change return shapes. Where an output schema
   is advertised, it must accurately cover the actual structured result, not a
   misleading stub. Clients that depend on full output schemas can use legacy-all
   during migration; changing result envelopes requires versioned compatibility.
6. Return one requested representation (structured default or rendered text), not
   prose duplicated in JSON and Markdown. Bound collections and diffs; offer
   pagination/artifact references for details. Hard constraints and skipped-check
   status are never removed to improve a budget number.

Add `tool_schema_lint` at the finalized public-schema seam, after sanitization, on
all profiles/transports. Recursively resolve local `$ref`s with cycle protection;
check descriptions on distinct nodes and report JSON pointers/tool names. A
registry annotation identifies closed-set fields; lint requires a matching schema
enum and a parser round-trip test for every serialized value. Heuristics flag new
`*_policy`, `mode`, `status`, and `action` strings for review; heuristics alone
cannot prove a field is a closed set. Ban the jargon patterns above in emitted
text, validate schemas with a JSON-Schema validator, and enforce size limits.
Use a temporary baseline allowlist only for legacy aliases, with removal release
and owner; canonical tools may not add lint exemptions.

## Skills as small host-neutral workflows

Replace the entry skill with a <=1,000 estimated-token host-neutral guide. It
states that the connected host writes by default, how to choose a task skill, and
two explicit branches:

- **Existing book:** list/select project, confirm branch/cursor from writer state,
  then load scene/revision/planning workflow. Do not create a replacement project.
- **New book:** establish title, premise/genre, reader/style contract and boundaries
  from the author's supplied direction; ask only for missing creative choices.
  Create project, select it, create book/chapter, then load planning/world/character
  setup as needed and prepare the first scene. New setup tools belong to the setup
  unit enabled in write; minimal directs to that profile rather than suggesting
  unavailable calls.

Move Codex model selection and Grok installation to short CLI-installed host
adapters/operator docs. Neither initialize instructions nor the general entry
skill assumes Codex, GPT-6, a particular model vendor, or MCP resource support.
Tools and resource URIs remain equivalent loading routes.

Split scene-writer into a <=2,500 estimated-token core covering prepare, authoring
provenance, voice/canon constraints, focused self-review, commit, validation,
recovery, and honest completion. Put craft examples, scene/sequel/MRU discussion,
research, source-sync, offload/rating receipts, and chapter/run details in named
`references/*.md`. Each reference has a trigger, explicit tool requirements, and
<=3,000 estimated tokens; split larger references. Core explains when to load each,
never “read everything before drafting.” Explicit-content routing and receipts
remain mandatory where applicable; shortening instructions does not relax them.

The reference loader must resolve nested skill assets (e.g.
`get_reference(name="scene-writer/offloaded-generation")` with equivalent resource
URI) without path traversal or collision with existing craft references. Preserve
old skill names/URIs; a request for scene-writer returns the new core and reference
index. The legacy complete text can remain in compatibility packaging for one
release, never auto-loaded. Other skill cores target <=3,000 tokens each; remove
repeated schema argument catalogs and link live schemas instead. Runtime context
packets carry project-specific facts; skills teach workflow, not duplicate canon.

## Budgets and tests

Enforce the same compact serialization and `ceil(UTF-8 bytes / 4)` proxy used in
the baseline, with exact byte caps as the authoritative host-independent gate.
Track real tokenizer counts optionally, with tokenizer/version named, but do not
mix providers' counts in CI. Both input-only and full catalog budgets must pass;
omitting output schemas cannot hide bloated inputs.

| Target profile | Max tools | Input estimate / bytes | Full estimate / bytes |
| --- | ---: | ---: | ---: |
| minimal | 16 | 3,000 / 12,000 | 4,000 / 16,000 |
| write / default | 45 | 8,000 / 32,000 | 10,000 / 40,000 |
| authoring | 55 | 10,000 / 40,000 | 12,000 / 48,000 |
| import | 30 | 5,000 / 20,000 | 6,000 / 24,000 |
| revision | 30 | 5,000 / 20,000 | 6,000 / 24,000 |
| worldbuilding / planning (each) | 45 | 8,000 / 32,000 | 10,000 / 40,000 |
| style / research / release (each) | 25 | 4,000 / 16,000 | 5,000 / 20,000 |

These are target acceptance ceilings, not claims that a target catalog has been
implemented/measured. Default full discovery must fall by >=92% from 557,658 bytes
to <=40,000; authoring by >=82% to <=48,000. No individual canonical tool may exceed
4,096 discovery bytes without an explicit reviewed budget reallocation. Preserve
schema accuracy before increasing budgets. Legacy-all is separately capped at the
measured 170 tools / 557,658 bytes plus an explicit reviewed allowance for new
canonical wrappers during its single release; it cannot be counted as progress.

`profile_token_budgets` runs both registry serialization and an isolated stdio
probe with the measurement protocol above, asserting identical tool names and
payload sizes, caps, and valid profile selection. A corresponding HTTP router test
checks parity. CI uses a fresh `/tmp` data/config directory, no host credentials,
no user data, and tears down children on all exits. Store per-tool deltas in CI
output so a failure says what grew. Test additions of oversized output schemas and
oversized descriptions actually fail. `skill_load_budgets` enforces entry/core/
reference caps and <=4,000 tokens for entry + scene core together.

A scene-loop contract fixture traces requests: after one session selection, an
ordinary scene with complete supplied continuity uses prepare + commit only.
Assert equivalent saved prose, state, beats, cursor, warning/coverage behavior,
branch isolation, and provenance gates to the current workflow. Measure actual
calls and loaded skill/schema bytes; do not turn “two calls” into fewer checks.
P1/P2 evaluations remain quality gates for recall/revision; P4 reports their
results when available and does not claim consolidation improves the audit's
57% recall or fixes `dead_character_acting` by itself.

## Compatibility and operator separation

Define R as the first release shipping canonical replacements, not a calendar
promise. R supports old names/inputs/results for **one full release**, with a
machine-readable alias ledger (replacement, scope, deprecation release, removal
release). Legacy profiles and unset default preserve their old visible names in
R; `legacy-all` supplies the old full catalog, while new profiles are explicitly
opted into. Freeze current names before S moves their source locations.

New canonical tools live in the generated profiles; compatibility aliases are
listed only in compatibility profiles and accepted there. Alias adapters retain
old DTOs and side effects, translating into shared service operations where
semantically equivalent. They are not simple string renames: save-only versus
final commit, branch-only revise, full snapshot restore, and style rollback need
distinct adapters. Add old JSON fixtures and harness resume tests for each family;
do not remove branch/revision APIs because ordinary scene work no longer uses them.
Deprecation notices use metadata or text without breaking legacy structured DTOs.

At R+1, switch unset default to generated write, remove deprecated aliases and
legacy profiles, and retain the documented canonical branch/revision profile.
Publish a migration table, profile-selection instructions, CLI equivalents, and
release notes before removal. Unknown/removed names return a concise migration
error, never route to a different mutation. Update CLAUDE.md, public docs, skills,
harness calls, and client fixtures together. Stored history/IDs need migration or
continued read support independently of alias removal; never discard old history.

Administrative aliases are a deliberate exception to model compatibility: ship
CLI equivalents first, then remove them from normal model profiles. If R must
support an old operator client, expose aliases only through an **explicitly enabled
operator endpoint with an operator credential/capability**, never by choosing a
model profile or naming a tool `operator_*`. Do not introduce that endpoint solely
for convenience: default is CLI-only. Config mutation, filesystem installation,
index repair, and deletion bypass must not remain callable through default MCP,
HTTP `/mcp`, or hidden by-name dispatch. Read-only sanitized route state stays
available. Test denied model calls as well as absent listings. CLI uses the same
service/locking rules and reports impacts; ordinary deletion retains its gates.

## Phased PR-sized implementation plan

All implementation below follows **S merged on main**, especially its tools
registry/dispatch/schema separation. Rebase then locate extracted modules instead
of editing the obsolete god-file layout. Do not combine semantic consolidation
with S's move-only diff.

1. **Inventory and baseline guards.** Port the isolated probe into a test/support
   utility; freeze old names, profiles, DTO fixtures, byte counts, and alias ledger.
   Add missing project-selection coverage as a focused behavior fix with docs.
   Exit: live profile snapshots, strict-client tests, existing harness flows pass.
2. **Manifests and profile compiler.** Add manifests to all 13 skills and optional
   units, generate allowlists, expose revision/style/research/setup units, and add
   alignment tests. Initially preserve compatibility profiles unchanged; annotate
   every old tool disposition. Exit: zero unowned tools or normative undeclared
   calls; import no longer drags style into new profiles.
3. **Schema hygiene and slim discovery.** Add descriptions/enums, correct research
   copy, remove emitted jargon, trim/omit large output schemas without changing
   legacy result DTOs. Add lint and profile budget tests; turn them on for canonical
   profiles while legacy snapshots retain explicit caps. Exit: valid schemas and
   target caps, no lossy type-erasure shortcuts.
4. **Skill packaging and entry rewrite.** Embed nested references/manifests, shorten
   entry and scene core, add new-book path, move host setup copy to adapters.
   Update initialize instructions. Exit: load-budget and all reference/profile
   alignment tests pass, including no-resource clients and reconnect/new-book cases.
5. **Prepare facade.** Introduce prepare DTOs and adapter composition over existing
   context assemblers; integrate P1's packet contract when ready. Keep legacy reads.
   Exit: branch/secret visibility, hard-constraint preservation, bounded output,
   coverage, stale dependency tokens, and ordinary-scene read count tests pass.
6. **Commit facade and harness adapter.** Introduce atomic approved scene effects,
   dry run, stale-version checks, idempotency, generation gates, and run-phase
   integration. Migrate callers one at a time; keep alias fixtures. Exit: no partial
   writes, retries safe, warnings honest, child-branch canon and relationship delta
   replacement correct, authoring checkpoints/resume unchanged. P1/P2 semantic
   changes are separate dependencies, not smuggled into this PR.
7. **Domain consolidation and recovery, in separate small PRs.** First style
   preview/dry-run + receipt/undo adapters; then typed update/batch replacements;
   then scene/canon aliases and other history scopes. Each PR owns its affected
   ledger rows and round-trip/conflict tests. Exit: all promised reversibility is
   demonstrated; unsupported cases explicitly marked, public revision flows work.
8. **Operator migration, then R release.** Add CLI operations, remove model admin
   access, migrate harness/skills/docs and publish the compatibility guide. Run all
   contract/budget tests plus normal workspace checks. Exit: operator calls denied
   via model transports, CLI repair tested in temp data, one-release aliases usable.
9. **R+1 removal PR.** After the full compatibility release, delete aliases/legacy
   profiles, flip default, keep stored history readable. Exit: no repository caller
   uses old names; scene loop and profile ceilings pass; public branch/revision
   and operator migration docs remain complete.

For each implementation PR run focused affected contracts, then required workspace
fmt/test/clippy checks. This design-only PR validates source citations, measured
stdio discovery, and Markdown/diff scope; it does not add the proposed tests or
claim a workspace test run. Any later main changes require refreshing the measured
baseline before budget enforcement, not silently treating this snapshot as current.

## Open questions for the author

Proceed with these recommended defaults unless the author chooses otherwise:

1. **Default discovery:** use generated write after R, not the full catalog. Keep
   explicit specialist profiles and a one-release legacy-all escape hatch.
2. **Operator compatibility:** CLI-only by default; build a separately authenticated
   operator endpoint only for a demonstrated existing client requirement.
3. **Commit authority:** ordinary commit applies the author's declared continuity
   package, while mined proposals still require explicit ratification. Consolidation
   does not grant auto-accept authority or alter creative boundaries.
4. **Budget tradeoff:** enforce byte caps above and split specialist workflows when
   a profile grows; raise a cap only with measured workflow evidence and review.

## Implementation checkpoints

The inventory guard in `tools/profile_snapshot_probe.py` checks live stdio
`tools/list` against the registry snapshots under
`crates/spindle-mcp/src/tools/snapshots/`. Run `cargo build -p spindle-mcp`
then `python3 tools/profile_snapshot_probe.py`. Each child uses an empty config
and a separate temporary data directory with inherited Spindle settings removed.
The probe reports compact UTF-8 input/full bytes and ceil(bytes / 4) estimates;
snapshot equality is the initial compatibility budget. Target canonical-profile
ceilings above remain pending the manifest/compiler and schema phases.

The first intentional surface change adds `set_active_project` to minimal,
write, and import. Existing-book selection is now listed and callable in every
named profile. Default and authoring discovery, legacy DTOs, unknown-profile
fallback, and branch/revision behavior remain unchanged. The snapshots include
this addition; a profile-call regression test exercises valid selection.

The manifest infrastructure adds versioned manifests to all 13 skills, static
nested setup/style references, an exhaustive current-name disposition ledger,
and registry/profile/dispatch alignment tests with negative fixtures. See
[skill manifest contracts](skill-manifests.md). Runtime profile generation,
profile-aware loading, schema lint and target-budget enforcement remain pending;
no current discovery profile or schema changes in this checkpoint.
