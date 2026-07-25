# Spindle skills

Root `skills/` is the source of truth for Spindle skill prompts. The
`spindle-skills` crate embeds those files at build time and exposes them through
`bible://skills/{skill-name}` resources. For MCP clients without resource
support, the same content is available through the `list_skills`, `get_skill`,
and `get_reference` tools.

Do not maintain duplicate skill prompt copies under `docs/`. Update the matching
`skills/<skill-name>/SKILL.md` file instead, then run the workspace validation
commands from `README.md`.
