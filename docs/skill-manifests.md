# Skill manifest alignment

`skills/<name>/manifest.toml` is the versioned dependency inventory for each of
the 13 embedded skill cores. A manifest declares its direct tool requirements,
required skill cores, and currently supported profiles. Dependencies contribute
their transitive tool requirements. Routing to a specialist skill in prose does
not automatically enable that specialist or union its tools into the entry skill.

`[[references]]` declares an independently loadable unit with a name, Markdown
path under `references/`, trigger, direct tools, and supported profiles. Loading a
reference requires both its tools and its owning core's dependency closure.
The nested units are `spindle/setup`, `editor/style`, and the merged P1 rollout
reference `scene-writer/drafting-packets`; load them with
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

Current long-form cores describe broad workflows, so most honestly declare only
the unset `default` profile. `default` is an internal manifest label for no startup
profile setting, not a newly registered profile. These declarations are coverage
assertions, not a profile compiler or access grant. Current named profiles and
`tools/list` bytes remain unchanged. Narrow cores, generated task profiles,
profile-aware loader messages, schema hygiene lint and target size ceilings follow
in later P4 PRs. Do not advertise a workflow in a narrow profile until its complete
dependency closure fits there.

`tools/tool-dispositions.toml` freezes every currently shipped registry name.
Retained entries need a skill/reference owner; compatibility entries have an
explicit owner and review release; operator entries cannot become skill
dependencies. The ledger does not claim replacement aliases or CLI coverage have
shipped. Operator calls remain in the compatibility catalog until the planned
CLI migration. Alias input/result adapters and R/R+1 removal metadata will be
added with their actual replacements; no alias is a mere string rename.

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
