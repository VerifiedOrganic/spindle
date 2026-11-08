# Style-profile workflow

Load when the author asks to create, inspect, apply, or revise a style profile.
Use `create_style_profile_from_markdown` for supplied style direction,
`list_style_profiles` and `get_style_profile` to inspect profiles, and
`check_style_profile_sources` before deciding whether to refresh.
Use `preview_refresh_style_profile` before `refresh_style_profile`, and
`preview_apply_style_profile` before an approved `apply_style_profile`.
These are the current preview contracts; live schemas define their arguments.

Use `check_style_against_profile`, `plan_style_revision`, and
`compare_style_profiles` to assess changes. `archive_style_profile` retires a
profile. Inspect `list_style_profile_applications` before choosing a scoped
`rollback_style_profile_application`.

For prose patches, use `preview_style_revision_patch`, then
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
<!-- tool: evaluate_style_revision_patch -->
<!-- tool: get_style_profile -->
<!-- tool: list_style_profile_applications -->
<!-- tool: list_style_profiles -->
<!-- tool: list_style_revision_patch_audits -->
<!-- tool: plan_style_revision -->
<!-- tool: preview_apply_style_profile -->
<!-- tool: preview_refresh_style_profile -->
<!-- tool: preview_style_revision_patch -->
<!-- tool: refresh_style_profile -->
<!-- tool: rollback_style_profile_application -->
<!-- tool: rollback_style_revision_patch -->
