# Style-profile workflow

Load when the author asks to create, inspect, apply, or revise a style profile.
Use `create_style_profile_from_markdown` for supplied style direction,
`list_style_profiles` and `get_style_profile` to inspect profiles, and
`check_style_profile_sources` before deciding whether to refresh.
Preflight with `refresh_style_profile` (`dry_run: true`) and `apply_style_profile`
(`dry_run: true`): a dry run reports the proposed candidate or prose diff without
inserting a version, consuming candidates, activating the profile, invalidating
caches, or writing an audit record. `create_style_revision_patch` creates a
non-mutating revision patch; only `apply_style_revision_patch` writes prose.
These are the current preview contracts; live schemas define their arguments.

Use `check_style_against_profile`, `plan_style_revision`, and
`compare_style_profiles` to assess changes. `archive_style_profile` retires a
profile. Inspect `list_style_profile_applications` before choosing a scoped
`rollback_style_profile_application`.

For prose patches, use `create_style_revision_patch`, then
`evaluate_style_revision_patch`; apply only an author-approved patch with
`apply_style_revision_patch`. Inspect `list_style_revision_patch_audits` and
use `rollback_style_revision_patch` for that patch's supported recovery scope.
An application rollback and a patch rollback have different scopes. Neither
implicitly restores unrelated scene or branch history.

<!-- tool: apply_style_profile -->
<!-- tool: apply_style_revision_patch -->
<!-- tool: archive_style_profile -->
<!-- tool: check_style_against_profile -->
<!-- tool: check_style_profile_sources -->
<!-- tool: compare_style_profiles -->
<!-- tool: create_style_profile_from_markdown -->
<!-- tool: create_style_revision_patch -->
<!-- tool: evaluate_style_revision_patch -->
<!-- tool: get_style_profile -->
<!-- tool: list_style_profile_applications -->
<!-- tool: list_style_profiles -->
<!-- tool: list_style_revision_patch_audits -->
<!-- tool: plan_style_revision -->
<!-- tool: refresh_style_profile -->
<!-- tool: rollback_style_profile_application -->
<!-- tool: rollback_style_revision_patch -->
