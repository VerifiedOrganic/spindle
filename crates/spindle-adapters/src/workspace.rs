use dirs::data_local_dir;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// The root of the workspace if a `.spindle/` directory was found.
    pub root: Option<PathBuf>,
    /// The directory where Spindle stores its data (e.g. `.spindle/` or the platform fallback).
    pub data_dir: PathBuf,
    /// The resolved path to the SQLite database file.
    pub db_path: PathBuf,
    /// The resolved path to the config file (if any).
    pub config_path: Option<PathBuf>,
}

pub fn find_workspace_root(cwd: &Path) -> Option<PathBuf> {
    for ancestor in cwd.ancestors() {
        let spindle_dir = ancestor.join(".spindle");
        if spindle_dir.is_dir() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

pub fn default_data_dir() -> PathBuf {
    resolve_workspace_data_dir(
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        std::env::var_os("SPINDLE_DATA_DIR").map(PathBuf::from),
    )
}

pub fn default_config_path() -> Option<PathBuf> {
    resolve_workspace_config_path(
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        std::env::var_os("SPINDLE_CONFIG").map(PathBuf::from),
    )
}

pub fn resolve_workspace_data_dir(cwd: &Path, env_data_dir: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = env_data_dir {
        dir
    } else if let Some(root) = find_workspace_root(cwd) {
        root.join(".spindle")
    } else {
        default_platform_data_dir_for_cwd(cwd)
    }
}

pub fn resolve_workspace_config_path(cwd: &Path, env_config: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = env_config {
        Some(path)
    } else if let Some(root) = find_workspace_root(cwd) {
        let local_config = root.join(".spindle").join("config.toml");
        if local_config.exists() {
            return Some(local_config);
        }
        check_other_configs(cwd)
    } else {
        check_other_configs(cwd)
    }
}

fn check_other_configs(cwd: &Path) -> Option<PathBuf> {
    let cwd_config = cwd.join("spindle.toml");
    if cwd_config.exists() {
        Some(cwd_config)
    } else if let Some(home_dir) = dirs::home_dir() {
        let home_config = home_dir.join(".spindle").join("config.toml");
        if home_config.exists() {
            Some(home_config)
        } else {
            None
        }
    } else {
        None
    }
}

pub fn default_platform_data_dir_for_cwd(cwd: &Path) -> PathBuf {
    data_local_dir()
        .map(|path| path.join("spindle"))
        .unwrap_or_else(|| cwd.join(".spindle-data"))
}

pub fn runtime_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("runtime")
}

pub fn resolve_workspace(
    cwd: &Path,
    env_data_dir: Option<PathBuf>,
    env_config: Option<PathBuf>,
) -> Workspace {
    let root = find_workspace_root(cwd);
    let data_dir = resolve_workspace_data_dir(cwd, env_data_dir.clone());

    // Determine if we are using a project-local workspace.
    let is_local = if env_data_dir.is_some() {
        env_data_dir
            .as_ref()
            .is_some_and(|dir| dir.ends_with(".spindle") || dir.join("spindle.db").exists())
    } else {
        root.is_some()
    };

    let db_path = if is_local {
        data_dir.join("spindle.db")
    } else {
        data_dir.join("spindle.sqlite")
    };

    let config_path = resolve_workspace_config_path(cwd, env_config);

    Workspace {
        root,
        data_dir,
        db_path,
        config_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_find_workspace_root_direct() {
        let temp = tempdir().unwrap();
        let project_root = temp.path();
        let spindle_dir = project_root.join(".spindle");
        std::fs::create_dir(&spindle_dir).unwrap();

        let root = find_workspace_root(project_root);
        assert_eq!(root, Some(project_root.to_path_buf()));
    }

    #[test]
    fn test_find_workspace_root_nested() {
        let temp = tempdir().unwrap();
        let project_root = temp.path();
        let spindle_dir = project_root.join(".spindle");
        std::fs::create_dir(&spindle_dir).unwrap();

        let nested = project_root.join("src").join("utils");
        std::fs::create_dir_all(&nested).unwrap();

        let root = find_workspace_root(&nested);
        assert_eq!(root, Some(project_root.to_path_buf()));
    }

    #[test]
    fn test_no_workspace_fallback() {
        let temp = tempdir().unwrap();
        let root = find_workspace_root(temp.path());
        assert_eq!(root, None);

        let ws = resolve_workspace(temp.path(), None, None);
        assert_eq!(ws.root, None);
        assert!(ws.db_path.ends_with("spindle.sqlite"));
    }

    #[test]
    fn test_data_dir_override_wins() {
        let temp = tempdir().unwrap();
        let explicit_data_dir = temp.path().join("explicit_data");

        let ws = resolve_workspace(temp.path(), Some(explicit_data_dir.clone()), None);
        assert_eq!(ws.data_dir, explicit_data_dir);
        assert_eq!(ws.db_path, explicit_data_dir.join("spindle.sqlite"));
    }

    #[test]
    fn test_spindle_data_dir_override_uses_local_db_name() {
        let temp = tempdir().unwrap();
        let explicit_data_dir = temp.path().join(".spindle");

        let ws = resolve_workspace(temp.path(), Some(explicit_data_dir.clone()), None);
        assert_eq!(ws.data_dir, explicit_data_dir);
        assert_eq!(ws.db_path, ws.data_dir.join("spindle.db"));
    }

    #[test]
    fn test_config_override_wins() {
        let temp = tempdir().unwrap();
        let explicit_config = temp.path().join("explicit_config.toml");

        let ws = resolve_workspace(temp.path(), None, Some(explicit_config.clone()));
        assert_eq!(ws.config_path, Some(explicit_config));
    }

    #[test]
    fn test_runtime_dir_stays_inside_data_dir() {
        let temp = tempdir().unwrap();
        assert_eq!(runtime_dir(temp.path()), temp.path().join("runtime"));
    }
}
