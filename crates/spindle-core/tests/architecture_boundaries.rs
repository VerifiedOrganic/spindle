use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("spindle-core should live under crates/spindle-core")
        .to_path_buf()
}

fn rust_files_under(relative_dir: &str) -> Vec<PathBuf> {
    let root = repo_root().join(relative_dir);
    let mut files = Vec::new();
    collect_rust_files(&root, &mut files);
    files.sort();
    files
}

fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|error| {
        panic!("failed to read architecture guard directory {dir:?}: {error}")
    }) {
        let path = entry.expect("failed to read directory entry").path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn assert_no_forbidden_patterns(relative_dir: &str, forbidden: &[&str]) {
    let mut violations = Vec::new();
    for file in rust_files_under(relative_dir) {
        let contents = fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("failed to read {file:?}: {error}"));
        for (line_number, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for pattern in forbidden {
                if line.contains(pattern) {
                    violations.push(format!(
                        "{}:{} contains `{}`",
                        file.strip_prefix(repo_root()).unwrap_or(&file).display(),
                        line_number + 1,
                        pattern
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "architecture boundary violations:\n{}",
        violations.join("\n")
    );
}

fn non_comment_entries_in_manifest_section(manifest: &str, section: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut in_section = false;
    let section_header = format!("[{section}]");

    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed == section_header;
            continue;
        }
        if !in_section {
            continue;
        }

        let entry = trimmed.split('#').next().unwrap_or_default().trim();
        if !entry.is_empty() {
            entries.push(entry.to_string());
        }
    }

    entries
}

#[test]
fn spindle_core_has_no_adapter_transport_or_asset_imports() {
    assert_no_forbidden_patterns(
        "crates/spindle-core/src",
        &[
            "spindle_adapters::",
            "spindle_mcp::",
            "spindle_skills::",
            "rmcp::",
            "rusqlite::",
            "tokio_rusqlite::",
        ],
    );
}

#[test]
fn spindle_skills_stays_static_asset_packaging_only() {
    let manifest_path = repo_root().join("crates/spindle-skills/Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("failed to read {manifest_path:?}: {error}"));
    let dependencies = non_comment_entries_in_manifest_section(&manifest, "dependencies");
    assert!(
        dependencies.is_empty(),
        "spindle-skills should not grow runtime dependencies:\n{}",
        dependencies.join("\n")
    );

    assert_no_forbidden_patterns(
        "crates/spindle-skills",
        &[
            "spindle_adapters::",
            "spindle_mcp::",
            "rmcp::",
            "rusqlite::",
            "tokio_rusqlite::",
            "reqwest::",
        ],
    );
}

#[test]
fn spindle_harness_does_not_import_sqlite_or_repository_internals() {
    assert_no_forbidden_patterns(
        "crates/spindle-harness/src",
        &[
            "spindle_adapters::sqlite",
            "SqlitePool",
            "SqliteSpindleService",
            "tokio_rusqlite::",
            "rusqlite::",
        ],
    );
}

#[test]
fn spindle_mcp_declares_no_public_contract_dtos() {
    let mut violations = Vec::new();
    for file in rust_files_under("crates/spindle-mcp/src") {
        let contents = fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("failed to read {file:?}: {error}"));
        for (line_number, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed.strip_prefix("pub struct ") else {
                continue;
            };
            let struct_name = rest
                .split(|ch: char| ch == '<' || ch == '{' || ch == '(' || ch.is_whitespace())
                .next()
                .unwrap_or_default();
            if struct_name.ends_with("Input")
                || struct_name.ends_with("Output")
                || struct_name.ends_with("Envelope")
            {
                violations.push(format!(
                    "{}:{} defines public DTO `{}`",
                    file.strip_prefix(repo_root()).unwrap_or(&file).display(),
                    line_number + 1,
                    struct_name
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "MCP crate should consume public DTOs from spindle-core:\n{}",
        violations.join("\n")
    );
}
