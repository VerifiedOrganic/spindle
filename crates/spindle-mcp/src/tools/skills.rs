use super::*;

/// Public entry point for both the MCP tool and the CLI.
/// Writes the Grok skill adapter(s) for Spindle into the target directory
/// (or into ~/.grok/skills/ when `global` is true).
pub fn run_init_grok_skills(
    target_dir: Option<String>,
    global: bool,
) -> anyhow::Result<InitGrokSkillsOutput> {
    use std::fs;
    use std::path::PathBuf;

    let base_skills_dir: PathBuf = if global {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
        home.join(".grok").join("skills")
    } else {
        let target = match target_dir {
            Some(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => std::env::current_dir().map_err(|e| anyhow::anyhow!("could not get cwd: {e}"))?,
        };
        target.join(".grok").join("skills")
    };

    fs::create_dir_all(&base_skills_dir)
        .map_err(|e| anyhow::anyhow!("failed to create skills directory: {e}"))?;

    let mut files_written = Vec::new();

    // 1. Write the main meta skill
    let meta_dir = base_skills_dir.join("spindle");
    fs::create_dir_all(&meta_dir)?;
    let meta_path = meta_dir.join("SKILL.md");
    fs::write(&meta_path, SPINDLE_GROK_SKILL_MD)?;
    files_written.push(meta_path.display().to_string());

    // 2. Write thin adapters for every individual Spindle skill
    let spindle_skills = [
        "scene-writer",
        "character-creator",
        "worldbuilder",
        "revision-manager",
        "continuity-editor",
        "editor",
        "manuscript-importer",
        "bible-librarian",
        "plot-architect",
    ];

    for skill in spindle_skills {
        let adapter_name = format!("spindle-{}", skill);
        let adapter_dir = base_skills_dir.join(&adapter_name);
        fs::create_dir_all(&adapter_dir)?;

        let content = generate_spindle_skill_adapter(skill);
        let path = adapter_dir.join("SKILL.md");
        fs::write(&path, content)?;
        files_written.push(path.display().to_string());
    }

    let location = if global {
        "~/.grok/skills/ (global)"
    } else {
        "repo-scoped .grok/skills/"
    };

    Ok(InitGrokSkillsOutput {
        target_dir: if global {
            "~/.grok/skills".to_string()
        } else {
            // best effort
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        },
        files_written,
        message: format!(
            "Grok Spindle skills initialized ({location}). You now have the 'spindle' meta skill plus individual adapters (spindle-scene-writer, spindle-character-creator, etc.). They will be available globally in Grok."
        ),
    })
}

/// Generates a thin Grok adapter for a specific Spindle bible skill.
pub(super) fn generate_spindle_skill_adapter(skill_name: &str) -> String {
    let title = skill_name.replace('-', " ");
    let title = title
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        r#"---
name: spindle-{skill_name}
description: Use when doing {title} work inside Spindle book projects. This is the Grok adapter for the official Spindle {skill_name} skill.
---

# Spindle {title} (Grok Adapter)

This is a thin Grok-specific wrapper around Spindle's canonical skill.

**Authoritative instructions (always read these):** `bible://skills/{skill_name}`

## When to activate
- The user asks to write, plan, revise, or manage anything related to **{title}** in a Spindle project.

## Grok-specific notes
- Spindle already provides strong guardrails via the Bible, continuity, and voice profiles.
- Prefer natural, readable, webnovel-style prose.
- For explicit content, always route through the proper `continue_generation` + `rating: "explicit"` path.
- Re-anchor with `get_writer_state` + the relevant briefing tools before major work.

Use the live `bible://skills/{skill_name}` resource for the detailed step-by-step procedure.
"#
    )
}

impl ToolRouter {
    pub(super) fn handle_init_grok_skills(
        &self,
        input: InitGrokSkillsInput,
    ) -> anyhow::Result<InitGrokSkillsOutput> {
        run_init_grok_skills(input.target_dir, input.global)
    }
}
