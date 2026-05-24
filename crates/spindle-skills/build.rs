use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let skills_dir = repo_root.join("skills");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("out dir"));

    println!("cargo:rerun-if-changed={}", skills_dir.display());

    let mut skill_dirs: Vec<_> = fs::read_dir(&skills_dir)
        .expect("read skills dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    skill_dirs.sort_by_key(|entry| entry.file_name());

    let mut generated = String::from("pub const EMBEDDED_SKILLS: &[EmbeddedSkill] = &[\n");

    for entry in skill_dirs {
        let name = entry.file_name().to_string_lossy().into_owned();
        let skill_path = entry.path().join("SKILL.md");
        if !skill_path.is_file() {
            continue;
        }

        println!("cargo:rerun-if-changed={}", skill_path.display());

        let include_path = skill_path.to_string_lossy().replace('\\', "\\\\");
        generated.push_str("    EmbeddedSkill {\n");
        generated.push_str(&format!("        name: \"{name}\",\n"));
        generated.push_str(&format!(
            "        markdown: include_str!(\"{include_path}\"),\n"
        ));
        generated.push_str("    },\n");
    }

    generated.push_str("];\n");

    fs::write(out_dir.join("embedded_skills.rs"), generated).expect("write embedded skills");
}
