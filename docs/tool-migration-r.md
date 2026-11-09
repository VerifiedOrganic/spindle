# Release R compatibility and migration guide

Status: release R is the first release shipping the canonical tool surface. It
keeps the old names, inputs, and results that this guide lists for one full
release; R+1 removes the deprecated compatibility aliases and legacy profiles.
The design contract is in [tool surface and skill alignment](tool-surface-design.md).
Stored history and record IDs remain readable independently of alias removal.

## Model surface changes

### Operator commands are CLI-only

Operator commands are no longer tool calls. They are absent from `tools/list`
for every model profile, including unset and `legacy-all`, and a `tools/call`
by name returns the same concise unknown-tool error as any other unavailable
name. There is no operator MCP endpoint in release R. The authoring harness may
reach `test_agent` only through a server-issued internal dispatch token; an
external stdio or HTTP caller cannot.

| Old MCP tool | CLI replacement | Notes |
| --- | --- | --- |
| `init_grok_skills` | `spindle-mcp init-grok-skills` | Existing command; unchanged. |
| `configure_agents` | `spindle-mcp configure-agents` | Validates and reports the resolved config. Point `SPINDLE_CONFIG` or the workspace `config.toml` at the file and restart the server to apply it. |
| `test_agent` | `spindle-mcp test-agent --agent <id>` | Optional `--prompt`, `--route`, `--rating`, `--project-id`, `--book-id`, `--chapter-id`, `--scene-id`, `--run-id`. |
| `rebuild_search_index` | `spindle-mcp rebuild-search-index --project <id>` | Rebuilds FTS and embedding rows for one project. |
| `backfill_scene_source_offsets` | `spindle-mcp backfill-scene-source-offsets --project <id> --branch <id>` | Backfills scene source offsets for one branch. |
| `operator_delete_scene` | `spindle-mcp operator-delete-scene --project <id> (--scene-id <id> \| --book-number <n> --chapter-number <n> --scene-order <n>)` | The operator bypass remains a deliberate deletion path; ordinary model deletion keeps its dependency gates. |

The operator commands accept `--json` for structured output. `--data-dir` and
`--config` override `SPINDLE_DATA_DIR` and `SPINDLE_CONFIG`. Commands that open the project
database acquire the workspace primary lock; stop a running server before using
them. `configure-agents` does not need the database.

### Ordinary scene work

The ordinary loop is `prepare_scene` then `commit_scene`. `commit_scene` is the
single atomic write for prose, author-declared continuity, beats, an optional
chapter-completion summary, and the writer cursor, with `dry_run`, stale-token
and stale-version checks, idempotent retries, and optional authoring-run phase
integration. `save_scene_draft`, `revise_scene`, `authoring_save_scene_draft`,
`commit_scene_changes`, and `annotate_scene_beats` keep their release-R
save-only and branch semantics. Branch and revision tools are not removed.

### Style profiles

`refresh_style_profile` and `apply_style_profile` accept `dry_run`. A dry run
reports the proposed candidate or prose diff without inserting a profile
version, consuming candidates, activating the profile, invalidating caches, or
writing an audit record. `preview_refresh_style_profile` and
`preview_apply_style_profile` remain available through release R and return the
full extended preview shapes. `create_style_revision_patch` is the canonical
non-mutating patch-creation step; `preview_style_revision_patch` remains its
release-R alias, and only `apply_style_revision_patch` writes prose.

### Typed entity updates

`update_character`, `update_location`, `update_conflict`, and
`update_character_arc` are the canonical typed updates for those records. They
reject unknown fields, leave omitted fields unchanged, and keep the existing
validation/cache behavior. `update_entity` remains available as a compatibility
path for entity types that do not have a typed replacement yet; do not use it
for the four typed records when a typed tool covers the field.

### Batch creates

`create_terms`, `create_motifs`, `create_narrative_promises`, and
`set_character_voice_profiles` are the canonical batch operations. They accept
1-50 `items`, preflight every item, support `dry_run`, and commit all-or-nothing
in one branch transaction; errors identify the failing item index. Dry-run
preflight rejects existing exact-branch and within-batch normalized term/motif
collisions; child branches may shadow inherited names.
Post-commit derived work is best-effort and is reported in `warnings`; the
rows remain committed. The
`batch_create_terms`, `batch_create_motifs`,
`batch_create_narrative_promises`, and `batch_set_character_voice_profiles`
names delegate to them as one-release compatibility aliases.

### Unified history and undo

`list_changes` is the discoverable history surface: it paginates scene saves,
branch checkpoints, style applications/patches, and prior undo records, and
marks each receipt `reversible`, `checkpoint_only`, or `irreversible` with a
reason. `undo_change(project_id, operation_id, expected_head, dry_run)` rolls back the
current applied style application/patch after the expected head matches and
records a pending compensating audit before applying the rollback and marks it
completed after; a retry reconciles an interrupted compensation. It never
deletes history. Scene snapshots are
`checkpoint_only` because they preserve prose but not approved continuity
deltas: review those deltas and use `restore_scene_version` explicitly. Branch
save points remain checkpoint-scoped: use `restore_save_point`, not
`undo_change`. `restore_scene_version`, `restore_save_point`, and the style
rollback tools keep their original inputs and scopes.

### Profiles

Unset and `legacy-all` keep the full compatibility catalog except the
operator-only commands above. The generated workflow/specialist profiles remain
opt-in. Select a profile with `SPINDLE_TOOL_PROFILE` and reconnect; reading a
skill never grants tools or changes the active profile. `spindle-mcp profiles
--json` prints the catalog, units, and required tools without opening a project
database.

## Compatibility aliases during R

The following families keep their old names and shapes for R. They are adapted
at the service boundary rather than treated as simple renames:

- Save/revise/authoring-save/commit-changes/annotate-beats: save-only and
  branch semantics are preserved; `commit_scene` is the canonical atomic write.
- Preview refresh/apply and preview revision patch: kept as preview/alias routes;
  dry-run and `create_style_revision_patch` are the canonical routes.
- Branch, save-point, scene-version restore, alternative selection, and style
  rollback: kept; history/undo consolidation continues after R where its
  scoped adapters are not yet universal.
- Legacy skill names and resource URIs: kept; the shortened manifest-aligned
  skills are the canonical loading routes.

## R+1 removal

After the full compatibility release, R+1 will:

- remove the deprecated preview twins and remaining compatibility aliases;
- switch the unset default profile to the generated `write` profile;
- remove `legacy-all` and the legacy compatibility profiles;
- keep stored history, record IDs, and scene/version data readable, with
  migration or continued read support rather than discarding history.

Unknown or removed names return a concise migration error and never route to a
different mutation.

## Migration checklist

1. Update MCP configuration to a supported profile (or leave unset for R).
2. Replace operator tool calls with the CLI commands above; stop the server
   before DB-mutating commands.
3. Replace ad-hoc save/revise/commit sequences with `prepare_scene` +
   `commit_scene` where the workflow is ordinary scene drafting; keep the
   legacy calls for save-only or targeted repair.
4. Replace style refresh/apply preview calls with `dry_run: true` on the
   canonical tools; keep the preview twins only for their extended shapes.
5. Run `spindle-mcp profiles --json`, the profile snapshot probe, and the
   workspace test suite in CI before upgrading clients.
