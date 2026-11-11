use std::path::{Component, Path};

use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use spindle_core::models::{CommitSceneProseReceipt, CommitSceneProseSource};

pub(super) const MAX_PROSE_BYTES: usize = 2 * 1024 * 1024;

pub(super) struct ResolvedProse {
    pub text: String,
    pub receipt: CommitSceneProseReceipt,
}

/// Resolve the book root from the repository data dir. Relative
/// `SPINDLE_DATA_DIR=.spindle` is joined to the primary's cwd before taking the
/// parent; canonicalizing avoids empty-parent and symlink surprises.
pub(super) fn workspace_root_from_data_dir(data_dir: &Path, cwd: &Path) -> Option<std::path::PathBuf> {
    let absolute = if data_dir.is_absolute() {
        data_dir.to_path_buf()
    } else {
        cwd.join(data_dir)
    };
    let canonical = absolute.canonicalize().ok()?;
    if canonical.file_name().and_then(|name| name.to_str()) != Some(".spindle") {
        return None;
    }
    canonical.parent().map(std::path::Path::to_path_buf)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn checked_hash(expected: Option<&str>, actual: &str) -> Result<()> {
    let Some(expected) = expected.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if expected.len() != 64 || !expected.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("commit_scene expected_sha256 must be a 64-character hex SHA-256 digest");
    }
    if !expected.eq_ignore_ascii_case(actual) {
        bail!(
            "commit_scene expected_sha256 does not match the prose source (expected {expected}, got {actual})"
        );
    }
    Ok(())
}

fn validate_relative_path(prose_path: &str) -> Result<&Path> {
    let path = Path::new(prose_path);
    if path.is_absolute() {
        bail!("commit_scene prose_path must be relative to the book workspace root, not absolute");
    }
    let mut components = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                if name.to_string_lossy().starts_with('.') {
                    bail!(
                        "commit_scene prose_path must not reference .spindle internals or dotfiles"
                    );
                }
                components += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                bail!("commit_scene prose_path must not contain '..'");
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("commit_scene prose_path must be relative to the book workspace root");
            }
        }
    }
    if components == 0 {
        bail!("commit_scene prose_path must name a file under the book workspace");
    }
    Ok(path)
}

fn receipt_for_inline(text: &str) -> CommitSceneProseReceipt {
    let bytes = text.as_bytes();
    CommitSceneProseReceipt {
        source: CommitSceneProseSource::Inline,
        path: None,
        byte_count: bytes.len(),
        sha256: sha256_hex(bytes),
    }
}

fn resolve_file(
    workspace_root: Option<&Path>,
    prose_path: &str,
    expected_sha256: Option<&str>,
) -> Result<ResolvedProse> {
    let root = workspace_root.ok_or_else(|| {
        anyhow::anyhow!(
            "commit_scene prose_path requires a project-local .spindle workspace so the primary can resolve it"
        )
    })?;
    let relative = validate_relative_path(prose_path)?;
    let root = root.canonicalize().map_err(|_| {
        anyhow::anyhow!("commit_scene prose_path workspace root is not readable")
    })?;
    let candidate = root.join(relative);
    let canonical = candidate.canonicalize().map_err(|_| {
        anyhow::anyhow!(
            "commit_scene prose_path '{prose_path}' does not exist or cannot be resolved"
        )
    })?;
    if !canonical.starts_with(&root) {
        bail!("commit_scene prose_path '{prose_path}' resolves outside the book workspace");
    }
    let spindle_root = root.join(".spindle");
    if canonical.starts_with(&spindle_root) {
        bail!("commit_scene prose_path must not reference .spindle internals");
    }
    let relative_canonical = canonical.strip_prefix(&root).map_err(|_| {
        anyhow::anyhow!("commit_scene prose_path '{prose_path}' resolves outside the book workspace")
    })?;
    if relative_canonical
        .components()
        .any(|component| match component {
            Component::Normal(name) => name.to_string_lossy().starts_with('.'),
            _ => true,
        })
    {
        bail!("commit_scene prose_path must not reference dotfiles");
    }
    let metadata = std::fs::metadata(&canonical).map_err(|_| {
        anyhow::anyhow!("commit_scene prose_path '{prose_path}' is not readable")
    })?;
    if !metadata.file_type().is_file() {
        bail!("commit_scene prose_path '{prose_path}' is not a regular file");
    }
    let len = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    if len > MAX_PROSE_BYTES {
        bail!(
            "commit_scene prose_path '{prose_path}' is {len} bytes; the limit is {MAX_PROSE_BYTES} bytes"
        );
    }
    let bytes = std::fs::read(&canonical)
        .map_err(|_| anyhow::anyhow!("commit_scene prose_path '{prose_path}' is not readable"))?;
    if bytes.len() > MAX_PROSE_BYTES {
        bail!(
            "commit_scene prose_path '{prose_path}' is {} bytes; the limit is {MAX_PROSE_BYTES} bytes",
            bytes.len()
        );
    }
    let text = String::from_utf8(bytes.clone()).map_err(|_| {
        anyhow::anyhow!("commit_scene prose_path '{prose_path}' must be UTF-8 text")
    })?;
    let original = sha256_hex(&bytes);
    checked_hash(expected_sha256, &original)?;
    let path = relative_canonical
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("commit_scene prose_path must be valid UTF-8"))?
        .replace('\\', "/");
    Ok(ResolvedProse {
        text: normalize_line_endings(&text),
        receipt: CommitSceneProseReceipt {
            source: CommitSceneProseSource::File,
            path: Some(path),
            byte_count: bytes.len(),
            sha256: original,
        },
    })
}

pub(super) fn resolve_prose(
    workspace_root: Option<&Path>,
    full_text: &str,
    prose_path: Option<&str>,
    expected_sha256: Option<&str>,
) -> Result<ResolvedProse> {
    let prose_path = prose_path.map(str::trim).filter(|path| !path.is_empty());
    let has_inline = !full_text.is_empty();
    match (has_inline, prose_path) {
        (true, Some(_)) => bail!(
            "commit_scene accepts exactly one of full_text or prose_path; pass prose_path for long prose"
        ),
        (false, None) => bail!(
            "commit_scene requires exactly one of full_text or prose_path; write long prose to a workspace file and pass prose_path"
        ),
        (false, Some(path)) => resolve_file(workspace_root, path, expected_sha256),
        (true, None) => {
            if full_text.len() > MAX_PROSE_BYTES {
                bail!(
                    "commit_scene inline full_text is {} bytes; the limit is {MAX_PROSE_BYTES} bytes. Write the prose to a workspace file and pass prose_path instead",
                    full_text.len()
                );
            }
            let receipt = receipt_for_inline(full_text);
            checked_hash(expected_sha256, &receipt.sha256)?;
            Ok(ResolvedProse {
                text: full_text.to_string(),
                receipt,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn root() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("book");
        fs::create_dir_all(root.join(".spindle")).expect("workspace");
        fs::create_dir_all(root.join("scenes")).expect("scenes");
        (temp, root)
    }

    fn err_text(result: Result<ResolvedProse>) -> String {
        result.err().expect("expected error").to_string()
    }

    #[test]
    fn resolves_relative_and_absolute_spindle_data_dirs() {
        let (temp, root) = root();
        let expected = root.canonicalize().unwrap();
        let relative = workspace_root_from_data_dir(Path::new(".spindle"), &root);
        assert_eq!(relative.as_deref(), Some(expected.as_path()));
        let absolute =
            workspace_root_from_data_dir(&root.join(".spindle"), &temp.path().to_path_buf());
        assert_eq!(absolute.as_deref(), Some(expected.as_path()));
        assert!(workspace_root_from_data_dir(Path::new("data"), &root).is_none());
    }

    #[test]
    fn requires_exactly_one_source() {
        let (_temp, root) = root();
        let both = err_text(resolve_prose(Some(&root), "inline", Some("scenes/a.md"), None));
        assert!(both.contains("exactly one of full_text or prose_path"));
        let neither = err_text(resolve_prose(Some(&root), "", None, None));
        assert!(neither.contains("exactly one of full_text or prose_path"));
        assert!(neither.contains("prose_path"));
    }

    #[test]
    fn rejects_escape_absolute_missing_and_nonregular_paths() {
        let (_temp, root) = root();
        fs::write(root.parent().unwrap().join("outside.md"), "outside").unwrap();
        for path in ["../outside.md", "scenes/../../outside.md"] {
            let error = err_text(resolve_prose(Some(&root), "", Some(path), None));
            assert!(error.contains("must not contain '..'"), "{path}: {error}");
        }
        let absolute = root.join("scenes/a.md");
        fs::write(&absolute, "prose").unwrap();
        let error = err_text(resolve_prose(
            Some(&root),
            "",
            Some(absolute.to_str().unwrap()),
            None,
        ));
        assert!(error.contains("must be relative"), "{error}");
        let error = err_text(resolve_prose(Some(&root), "", Some("scenes/missing.md"), None));
        assert!(error.contains("does not exist"), "{error}");
        let error = err_text(resolve_prose(Some(&root), "", Some("scenes"), None));
        assert!(error.contains("not a regular file"), "{error}");
    }

    #[test]
    fn rejects_dotfiles_and_spindle_internals() {
        let (_temp, root) = root();
        fs::write(root.join("scenes/.hidden.md"), "prose").unwrap();
        fs::write(root.join(".spindle/secret.md"), "prose").unwrap();
        for path in ["scenes/.hidden.md", ".spindle/secret.md", ".hidden.md"] {
            let error = err_text(resolve_prose(Some(&root), "", Some(path), None));
            assert!(
                error.contains("dotfile") || error.contains(".spindle internals"),
                "{path}: {error}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_outside_workspace() {
        let (_temp, root) = root();
        let outside = root.parent().unwrap().join("outside.md");
        fs::write(&outside, "outside").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("scenes/link.md")).unwrap();
        let error = err_text(resolve_prose(Some(&root), "", Some("scenes/link.md"), None));
        assert!(error.contains("outside the book workspace"), "{error}");
    }

    #[test]
    fn normalizes_file_line_endings_and_receipts_raw_bytes() {
        let (_temp, root) = root();
        let bytes = b"line one\r\nline two\rline three\n";
        fs::write(root.join("scenes/ch1.md"), bytes).unwrap();
        let resolved = resolve_prose(Some(&root), "", Some("scenes/ch1.md"), None).unwrap();
        assert_eq!(resolved.text, "line one\nline two\nline three\n");
        assert_eq!(resolved.receipt.source, CommitSceneProseSource::File);
        assert_eq!(resolved.receipt.path.as_deref(), Some("scenes/ch1.md"));
        assert_eq!(resolved.receipt.byte_count, bytes.len());
        assert_eq!(resolved.receipt.sha256, sha256_hex(bytes));
    }

    #[test]
    fn checks_expected_hash_for_file_and_inline() {
        let (_temp, root) = root();
        let bytes = b"hashed prose\n";
        fs::write(root.join("scenes/ch1.md"), bytes).unwrap();
        let hash = sha256_hex(bytes);
        resolve_prose(Some(&root), "", Some("scenes/ch1.md"), Some(&hash)).unwrap();
        let error = err_text(resolve_prose(
            Some(&root),
            "",
            Some("scenes/ch1.md"),
            Some(&"0".repeat(64)),
        ));
        assert!(error.contains("does not match"), "{error}");
        let inline = resolve_prose(Some(&root), "hashed prose\n", None, Some(&hash)).unwrap();
        assert_eq!(inline.receipt.source, CommitSceneProseSource::Inline);
        assert_eq!(inline.receipt.byte_count, bytes.len());
    }

    #[test]
    fn rejects_non_utf8_and_oversized_files() {
        let (_temp, root) = root();
        fs::write(root.join("scenes/bad.md"), [0xff, 0xfe, b'\n']).unwrap();
        let error = err_text(resolve_prose(Some(&root), "", Some("scenes/bad.md"), None));
        assert!(error.contains("UTF-8"), "{error}");
        fs::write(
            root.join("scenes/huge.md"),
            vec![b'a'; MAX_PROSE_BYTES + 1],
        )
        .unwrap();
        let error = err_text(resolve_prose(Some(&root), "", Some("scenes/huge.md"), None));
        assert!(error.contains("limit"), "{error}");
        let inline = "a".repeat(MAX_PROSE_BYTES + 1);
        let error = err_text(resolve_prose(Some(&root), &inline, None, None));
        assert!(error.contains("pass prose_path"), "{error}");
    }

    #[test]
    fn requires_workspace_root_for_file_mode() {
        let error = err_text(resolve_prose(None, "", Some("scenes/ch1.md"), None));
        assert!(error.contains("project-local .spindle workspace"), "{error}");
    }
}
