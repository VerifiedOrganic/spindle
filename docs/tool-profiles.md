# Startup tool profiles

Select a profile before starting Spindle, then reconnect the MCP client:

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

Unset, `minimal`, `write`, `import`, and `authoring` retain their exact existing
catalogs for compatibility. Explicit `legacy-all` gives the same complete catalog
as unset. New generated profiles are opt-in:

| Profile | Enabled work | Tools | Input bytes | Full bytes |
| --- | --- | ---: | ---: | ---: |
| workflow-write | Scene-writer core + setup | 45 | 50,064 | 103,725 |
| workflow-authoring | Scene-writer + authoring-supervisor + setup | 70 | 74,469 | 160,027 |
| workflow-import | Manuscript importer and source sync | 27 | 26,748 | 62,888 |
| revision | Branch/revision/history workflow | 38 | 32,735 | 83,514 |
| worldbuilding | Worldbuilder + character creator | 48 | 51,723 | 90,560 |
| planning | Plot architect | 39 | 39,800 | 80,892 |
| research | Researcher | 22 | 21,797 | 51,963 |
| style | Editor core + style reference | 48 | 41,648 | 106,579 |
| release | Editor + bible librarian | 65 | 57,699 | 118,742 |
| setup | New-book setup reference | 16 | 18,393 | 40,986 |

Every generated profile includes the shared entry core. These are **transitional
closures over current long-form skills and schemas**, not the final canonical
catalogs. The broad entry/core dependencies still contribute run, offload or
release capabilities in some profiles. Large context/analysis output schemas now load optionally through versioned
[response contracts](tool-response-contracts.md); actual results are unchanged.
Their measured byte ceilings are explicit
in `tools/profile-budgets.json`; canonical target caps are unchanged and remain
unmet. Lower tool counts alone do not establish lower full-schema bytes. Skill
splitting, profile-aware loading metadata and canonical schema cleanup follow.
Generated import has no style dependency; style is a separately selected profile.
No generated profile includes the six currently classified operator tools. This
is discovery/invocation filtering, not operator authentication; operator CLI
migration and removal from compatibility MCP catalogs are separate work.

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
Review new profile `tools/list` snapshots and measured caps in the same PR.
Existing five catalog snapshots stay unchanged; the direct unknown-router
snapshot intentionally becomes empty because startup typos no longer fall back.

StdIO, HTTP and secondary proxies use the same profile gate. A secondary filters
the primary's discovered catalog and denies out-of-profile calls before forwarding
(the primary may further restrict availability). If a shared primary uses a
narrower profile, reconnect to a primary with the needed capabilities; selecting a
secondary profile cannot expand its primary. Tests exercise real HTTP sessions
and a secondary over a full primary, including a denied mutation with no database
change. The isolated live probe checks every profile, CLI metadata/help, and
unknown-name rejection before data access. The inherited stalled-child test
observes the real child/workspace from the parent, avoiding child-marker races.
