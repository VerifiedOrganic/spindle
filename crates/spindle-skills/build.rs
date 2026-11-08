use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    name: String,
    tools: Vec<String>,
    profiles: Vec<String>,
    requires_skills: Vec<String>,
    #[serde(default)]
    references: Vec<Reference>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    name: String,
    path: String,
    tools: Vec<String>,
    profiles: Vec<String>,
    trigger: String,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-_".contains(&c))
}

fn unique(values: &[String], context: &str) {
    let mut seen = BTreeSet::new();
    for value in values {
        assert!(identifier(value), "invalid {context}: {value}");
        assert!(seen.insert(value), "duplicate {context}: {value}");
    }
}

fn visit<'a>(
    name: &'a str,
    manifests: &'a BTreeMap<String, Manifest>,
    active: &mut BTreeSet<&'a str>,
    done: &mut BTreeSet<&'a str>,
) {
    if done.contains(name) {
        return;
    }
    assert!(active.insert(name), "skill dependency cycle at {name}");
    let manifest = manifests
        .get(name)
        .unwrap_or_else(|| panic!("unknown skill dependency {name}"));
    for dependency in &manifest.requires_skills {
        visit(dependency, manifests, active, done);
    }
    active.remove(name);
    done.insert(name);
}

fn reference_path(path: &Path, relative: &str) -> PathBuf {
    let relative = Path::new(relative);
    assert!(
        relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
            && relative.starts_with("references")
            && relative.extension().is_some_and(|s| s == "md"),
        "unsafe reference path"
    );
    let reference_path = path
        .join(relative)
        .canonicalize()
        .expect("reference exists");
    assert!(
        reference_path.starts_with(path.canonicalize().unwrap()),
        "reference escapes skill directory"
    );
    reference_path
}

fn markdown_files(path: &Path, files: &mut BTreeSet<PathBuf>) {
    if !path.exists() {
        return;
    }
    for entry in fs::read_dir(path).expect("read references") {
        let path = entry.expect("reference entry").path();
        if path.is_dir() {
            assert!(
                !path.is_symlink(),
                "reference directory must not be a symlink"
            );
            markdown_files(&path, files);
        } else if path.extension().is_some_and(|e| e == "md") {
            files.insert(path.canonicalize().expect("reference exists"));
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let skills_dir = repo_root.join("skills");
    println!("cargo:rerun-if-changed={}", skills_dir.display());
    let mut manifests = BTreeMap::new();
    for entry in fs::read_dir(&skills_dir).expect("read skills dir") {
        let path = entry.expect("skill entry").path();
        if !path.join("SKILL.md").is_file() {
            continue;
        }
        let text = fs::read_to_string(path.join("manifest.toml"))
            .expect("every skill needs manifest.toml");
        let manifest: Manifest = toml::from_str(&text).expect("valid skill manifest");
        assert_eq!(manifest.version, 1, "unsupported manifest version");
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            manifest.name,
            "manifest name must match directory"
        );
        assert!(identifier(&manifest.name));
        unique(&manifest.tools, "tool");
        unique(&manifest.profiles, "profile");
        assert!(
            !manifest.profiles.is_empty(),
            "skill must declare supported profiles"
        );
        unique(&manifest.requires_skills, "dependency");
        assert!(
            manifests.insert(manifest.name.clone(), manifest).is_none(),
            "duplicate skill"
        );
    }
    let mut done = BTreeSet::new();
    for name in manifests.keys() {
        visit(name, &manifests, &mut BTreeSet::new(), &mut done);
    }
    let mut generated = String::from("pub const EMBEDDED_SKILLS: &[EmbeddedSkill] = &[\n");
    for manifest in manifests.values() {
        let path = skills_dir.join(&manifest.name);
        let mut references = String::from("&[");
        let mut names = BTreeSet::new();
        let mut declared_paths = BTreeSet::new();
        for reference in &manifest.references {
            assert!(
                identifier(&reference.name) && names.insert(&reference.name),
                "invalid/duplicate reference"
            );
            unique(&reference.tools, "reference tool");
            unique(&reference.profiles, "reference profile");
            assert!(!reference.profiles.is_empty() && !reference.trigger.trim().is_empty());
            let reference_path = reference_path(&path, &reference.path);
            assert!(
                declared_paths.insert(reference_path.clone()),
                "duplicate reference path"
            );
            references.push_str(&format!("EmbeddedReference {{ name: {:?}, markdown: include_str!({:?}), tools: &{:?}, profiles: &{:?}, trigger: {:?} }},", format!("{}/{}", manifest.name, reference.name), reference_path, reference.tools, reference.profiles, reference.trigger));
        }
        let mut actual_paths = BTreeSet::new();
        markdown_files(&path.join("references"), &mut actual_paths);
        assert_eq!(
            declared_paths, actual_paths,
            "every nested Markdown reference must be declared"
        );
        references.push(']');
        generated.push_str(&format!("EmbeddedSkill {{ name: {:?}, markdown: include_str!({:?}), manifest: SkillManifest {{ version: 1, name: {:?}, tools: &{:?}, profiles: &{:?}, requires_skills: &{:?}, references: {} }} }},\n", manifest.name, path.join("SKILL.md"), manifest.name, manifest.tools, manifest.profiles, manifest.requires_skills, references));
    }
    generated.push_str("];\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR").expect("out dir")).join("embedded_skills.rs"),
        generated,
    )
    .expect("write embedded skills");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(dependency: &str) -> Manifest {
        Manifest {
            version: 1,
            name: "test".into(),
            tools: vec![],
            profiles: vec!["default".into()],
            requires_skills: vec![dependency.into()],
            references: vec![],
        }
    }

    #[test]
    fn rejects_dependency_cycles_and_unknown_dependencies() {
        for manifests in [
            BTreeMap::from([("a".into(), manifest("b")), ("b".into(), manifest("a"))]),
            BTreeMap::from([("a".into(), manifest("missing"))]),
        ] {
            assert!(
                std::panic::catch_unwind(|| visit(
                    "a",
                    &manifests,
                    &mut BTreeSet::new(),
                    &mut BTreeSet::new()
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn rejects_duplicate_and_malformed_names() {
        for names in [
            vec!["tool".into(), "tool".into()],
            vec!["../tool".into()],
            vec!["".into()],
        ] {
            assert!(std::panic::catch_unwind(|| unique(&names, "test")).is_err());
        }
    }

    #[test]
    fn rejects_unknown_manifest_fields() {
        let text = "version = 1\nname = 'test'\ntools = []\nprofiles = ['default']\nrequires_skills = []\ntoolz = []";
        assert!(toml::from_str::<Manifest>(text).is_err());
    }

    #[test]
    fn reference_paths_are_confined() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("references")).unwrap();
        fs::write(root.path().join("references/valid.md"), "valid").unwrap();
        assert!(reference_path(root.path(), "references/valid.md").is_file());
        for name in [
            "../outside.md",
            "/etc/passwd",
            "references/../SKILL.md",
            "references/missing.md",
            "SKILL.md",
        ] {
            assert!(
                std::panic::catch_unwind(|| reference_path(root.path(), name)).is_err(),
                "{name}"
            );
        }
    }
}
