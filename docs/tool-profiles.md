# Startup tool profiles

Select an initial profile before starting Spindle, or switch the session at runtime with `use_profile`:

```sh
SPINDLE_TOOL_PROFILE=research spindle-mcp
spindle-mcp profiles
spindle-mcp profiles --json
```

CLI help lists every startup name. The profile catalog includes enabled units
and their complete required tool names without opening a project database.
Unknown names now fail before workspace/database access in both stdio and HTTP
modes. Directly constructed unknown-profile routers list nothing and deny calls.
The internal manifest label `default` means an unset environment variable; it is
not a startup profile name.

Unset starts the generated `workflow-write` profile. The legacy `minimal`,
`write`, `import`, `authoring`, and `legacy-all` names are removed in R+1 and
fail closed. Select a generated workflow or specialist profile with
`SPINDLE_TOOL_PROFILE`:

| Profile | Enabled work | Tools | Input bytes | Input tokens | Full bytes | Full tokens |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| workflow-write | Scene-writer core + craft/packet/research refs + setup | 46 | 63,414 | 15,854 | 133,598 | 33,400 |
| workflow-authoring | Scene-writer (all refs) + authoring-supervisor + setup | 78 | 96,711 | 24,178 | 208,854 | 52,214 |
| workflow-import | Manuscript importer and source sync | 21 | 15,052 | 3,763 | 33,983 | 8,496 |
| revision | Branch/revision/history workflow | 35 | 26,864 | 6,716 | 76,463 | 19,116 |
| worldbuilding | Worldbuilder + character creator | 52 | 54,983 | 13,746 | 103,237 | 25,810 |
| planning | Plot architect | 40 | 37,260 | 9,315 | 81,223 | 20,306 |
| new-book | New-book entry: planning sessions, human gates and handoff | 15 | 22,528 | 5,632 | 29,348 | 7,337 |
| research | Researcher | 16 | 8,118 | 2,030 | 21,075 | 5,269 |
| style | Editor core + style reference | 42 | 37,184 | 9,296 | 101,028 | 25,257 |
| release | Editor + bible librarian | 64 | 50,668 | 12,667 | 108,687 | 27,172 |
| setup | New-book setup reference | 10 | 4,696 | 1,174 | 10,080 | 2,520 |
| full | Every model-facing skill and reference unit | 177 | 179,504 | 44,876 | 381,794 | 95,449 |

Token counts are the same stable proxy used by the profile probe:
`ceil(bytes / 4)`, not a host tokenizer measurement.

## One server, switchable profiles

Start one server with any profile, then switch the session without reconnecting:

```json
{"profile": "full"}
```

`use_profile` validates the name against the generated catalog and returns
`{profile, tool_count, tools}` for the new active profile. The server declares
`tools.listChanged` and emits `notifications/tools/list_changed` after a
successful switch, so clients that support the capability refresh `tools/list`.
Out-of-profile calls remain denied under the new active profile. Unknown names
return an error result with `structuredContent.error.code = "unknown_profile"`.

The switch is session-scoped and in-memory. `SPINDLE_TOOL_PROFILE` remains the
initial profile, other sessions are unaffected, and a restart returns to the
startup profile. Use `full` when one session must reach every skill-declared
workflow without reconnecting.

### When to use `full`

`full` composes every model-facing skill and reference unit, so one MCP server
exposes every tool those units declare: the union of every generated workflow and
specialist profile plus the `continuity-editor` and `canon-steward` units that no
smaller profile enables. Use it when one session must expose every workflow at
once, without switching profiles during the session. It is the
largest catalog: 177 tools, 179,504 input bytes (about 44,876 proxy tokens) and
381,794 full bytes (about 95,449 proxy tokens). Prefer the smallest profile that
covers the current work; unset still starts `workflow-write`, and `full` is
opt-in. Operator-only commands stay excluded exactly as in every model profile.

Every generated profile includes the shared entry core (`spindle`, 7 entry tools).
Large context/analysis output schemas load optionally through versioned
[response contracts](tool-response-contracts.md); actual results are unchanged.
Their measured byte ceilings are explicit in `tools/profile-budgets.json`;
canonical target caps remain unmet until domain consolidation and compatibility
cleanup complete; `commit_scene` is the first
receipts-inline canonical write in these profiles. `get_skill` and `get_reference` check active profile coverage and
return structured coverage metadata (`covered`, `required_profile`,
`available_units`, `missing_capabilities`) without mutating the active profile.
Generated import has no style dependency; style is a separately selected profile.
Operator-only commands (`configure_agents`, `init_grok_skills`, `test_agent`,
`rebuild_search_index`, `backfill_scene_source_offsets`, and
`operator_delete_scene`) are omitted from every model profile, including unset,
and denied by name on stdio and HTTP model transports. Use the
CLI commands in the [release R migration guide](tool-migration-r.md). The
authoring harness reaches `test_agent` only through a server-issued internal
token; an external caller cannot. This is discovery/invocation filtering, not
operator authentication.

`skills/profiles.toml` composes skill and named-reference units. The build resolves
transitive skill dependencies, includes each reference's owning core, rejects
unknown units, duplicate definitions/units, cycles and reserved profile names,
and emits sorted/deduplicated static allowlists. Registry/operator checks remain
in MCP alignment tests, preserving the packaging crate's boundaries. Generated
skill/reference metadata reports enabled profile membership. Reading a skill
never adds tools to a profile; only the enabled units listed by the catalog are
supported workflows until loader coverage messages ship.

`tools/generated-profiles.json` is a checked compiler-output snapshot, not a
second editable composition file. Regenerate it and embedded skill metadata with
`SPINDLE_UPDATE_SKILL_SNAPSHOT=1 cargo test -p spindle-mcp metadata_matches_snapshot`.
Review every profile's `tools/list` snapshot and measured caps in the same PR.
The direct unknown-router snapshot intentionally stays empty because startup
typos never fall back.

StdIO, HTTP and secondary proxies use the same profile gate. A secondary filters
the primary's discovered catalog and denies out-of-profile calls before forwarding
(the primary may further restrict availability). A secondary `use_profile` may
narrow within the primary's discovered catalog, but a target that needs tools the
primary does not expose returns `profile_not_available`; reconnecting to a
broader primary is still the way to expand. Tests exercise real HTTP sessions,
per-session profile isolation, and a secondary over a primary, including denied
mutations with no database change. The isolated live probe checks every profile,
CLI metadata/help, and unknown-name rejection before data access. The inherited
stalled-child test observes the real child/workspace from the parent, avoiding
child-marker races.

## Semantic continuity evaluation

`check_consistency` accepts `semantic_mode: off | shadow | on` (default off).
`shadow` builds and judges candidates but emits no findings; `on` emits
policy-approved semantic findings. `prepare_semantic_continuity` is the
read-only host-request surface, and `check_consistency.semantic_host_reports`
accepts validated host reports. The evaluator is:

```sh
ai run OLLAMA_API_KEY -- python3 evals/continuity_recall.py run \
  --semantic shadow --agent-config /path/to/agents.toml --binary target/debug/spindle-mcp
```

Use `run`, `held-out`, `held-out-2`, or `held-out-3` as the set subcommand.
The command reports deterministic-only vs semantic-on recall, semantic warning
precision, control warnings by check type, evidence validity, repair/cache/call
rates, p95 prompt/output tokens and latency, and token totals per scene/chapter.
