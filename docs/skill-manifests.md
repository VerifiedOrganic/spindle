# Skill manifest alignment

`skills/<name>/manifest.toml` is the versioned dependency inventory for each of
the 14 embedded skill cores. A manifest declares its direct tool requirements,
required skill cores, and currently supported profiles. Dependencies contribute
their transitive tool requirements. Routing to a specialist skill in prose does
not automatically enable that specialist or union its tools into the entry skill.

`[[references]]` declares an independently loadable unit with a name, Markdown
path under `references/`, trigger, direct tools, and supported profiles. Loading a
reference requires both its tools and its owning core's dependency closure.
The nested units are `spindle/setup`, `editor/style`, and the five `scene-writer`
references (`scene-writer/authoring-runs`, `scene-writer/craft-and-voice`,
`scene-writer/drafting-packets`, `scene-writer/offloaded-generation`, and
`scene-writer/research-and-sources`); load them with
`get_reference(name="spindle/setup")` or the equivalent
`bible://references/spindle/setup` URI. Existing craft names and skill names remain
valid. Lookups match exact embedded names, with no runtime filesystem access.

The build checks TOML shape/version, names, duplicates, dependency cycles and
reference confinement (including canonicalized paths). The skills crate retains
no runtime dependencies. MCP tests validate tool names against the actual
registry, forbid operator dependencies, and check both discovery and dispatch
availability for every declared profile. Every named tool mention in core or
reference prose needs a declaration (possibly inherited) and an HTML
`<!-- tool: name -->` marker. Unknown markers fail too. Markers are inventory
annotations, never alternate callable names. This deliberately conservative
scanner covers prose, code spans, and fenced examples; remove obsolete tool
mentions instead of concealing them behind negative examples.

`skills/profiles.toml` now composes generated workflow profiles from core and
reference units. The build includes transitive skill dependencies and appends
generated profile membership to embedded manifests. Authored `profiles` arrays
remain coverage assertions for compatibility profiles. The unset `default` label
means no startup setting. See [profile selection and measurements](tool-profiles.md)
for new names, enabled units, static compiler snapshots and transitional limits.
The `spindle` entry core is host-neutral (`<= 1,000` estimated tokens), `scene-writer`
core is split into core (`<= 2,500` estimated tokens) plus five on-demand references,
and every other skill core and reference is capped at `<= 3,000` estimated tokens
(`skill_load_budgets`). `get_skill` and `get_reference` report structured profile
coverage metadata (`covered`, `required_profile`, `available_units`,
`missing_capabilities`) and return a profile-selection message when the active
session profile does not cover the requested unit. Reading a skill never grants
new capabilities or mutates the active profile.

`tools/tool-dispositions.toml` freezes every currently shipped registry name.
Retained entries need a skill/reference owner; compatibility entries have an
explicit owner and review release; operator entries cannot become skill
dependencies. Operator entries are now CLI-only: their commands are shipped in
`spindle-mcp`, hidden from every model profile, and denied by name on model
transports. See [release R migration](tool-migration-r.md). The ledger still does
not claim replacement aliases; alias input/result adapters and R/R+1 removal
metadata are added with their actual replacements, and no alias is a mere
string rename.

Run `cargo test -p spindle-skills`, `cargo test -p spindle-mcp alignment`, and
`cargo test -p spindle-mcp nested_skill_references_load`. Metadata is generated
from embedded manifests and compared to
`crates/spindle-mcp/src/tools/alignment/metadata.json`. After an intentional
manifest change, regenerate it with
`SPINDLE_UPDATE_SKILL_SNAPSHOT=1 cargo test -p spindle-mcp skill_manifest_metadata_matches_snapshot`,
inspect the diff, and run the test without the update setting. Never hand edit
generated metadata. Normal workspace CI runs all these tests; negative fixtures
cover unavailable selection, missing offload dependency, typo, operator dependency,
missing marker, and a newly added unowned tool.
