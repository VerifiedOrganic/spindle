use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::OnceLock;

/// Embedded Phase 0 pack stub. Phase 1 loads this rather than inventing IDs.
pub const DEFAULT_PACK_TOML: &str =
    include_str!("../../../../../references/anti-slop-shelf-pack.v0.toml");

/// Embedded human catalog. Scanner uses pack IDs/limits; catalog is the
/// rewrite-from-beats source of truth for authors.
pub const DEFAULT_CATALOG_MARKDOWN: &str = include_str!("../../../../../references/anti-slop.md");

#[derive(Debug)]
pub enum AntislopError {
    PackParse(String),
    NotFictionDomain(String),
}

impl fmt::Display for AntislopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PackParse(err) => write!(f, "anti-slop pack parse failed: {err}"),
            Self::NotFictionDomain(domain) => {
                write!(f, "anti-slop pack domain must be fiction, got {domain}")
            }
        }
    }
}

impl std::error::Error for AntislopError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Hard,
    Soft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RewriteMode {
    FromBeats,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ShelfLimit {
    Count(u32),
    Label(String),
}

impl ShelfLimit {
    pub fn allowed_count(&self) -> Option<u32> {
        match self {
            Self::Count(count) => Some(*count),
            Self::Label(_) => None,
        }
    }

    pub fn is_advisory_cluster(&self) -> bool {
        matches!(self, Self::Label(label) if label == "advisory_cluster")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PackPolicy {
    pub fiction_only: bool,
    pub auto_strict_requires_hard_count_zero: bool,
    pub soft_on_save: bool,
    pub fail_closed_hard_on_verify_revise: bool,
    pub max_rewrite_from_beats: u8,
    pub said_bookism_soft_only_by_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GenreOverride {
    pub profile: String,
    #[serde(default)]
    pub disable: Vec<String>,
    #[serde(default)]
    pub soften: Vec<String>,
    #[serde(default)]
    pub promote_to_hard: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ShelfSpec {
    pub id: String,
    pub severity: Severity,
    pub default_limit: ShelfLimit,
    pub limit_scope: String,
    pub enabled: bool,
    pub rewrite: RewriteMode,
    #[serde(default)]
    pub matchers: Vec<String>,
    #[serde(default)]
    pub family: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub exclude_as_tech_gate: Vec<String>,
    #[serde(default)]
    pub non_port: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShelfPack {
    pub schema_version: u32,
    pub pack_id: String,
    pub pack_version: String,
    pub domain: String,
    #[serde(default)]
    pub catalog_uri: Option<String>,
    #[serde(default)]
    pub catalog_path: Option<String>,
    pub policy: PackPolicy,
    #[serde(default)]
    pub genre_overrides: Vec<GenreOverride>,
    pub shelves: Vec<ShelfSpec>,
}

impl ShelfPack {
    pub fn load_default() -> Result<Self, AntislopError> {
        Self::from_toml(DEFAULT_PACK_TOML)
    }

    pub fn from_toml(toml_src: &str) -> Result<Self, AntislopError> {
        let pack: Self =
            toml::from_str(toml_src).map_err(|err| AntislopError::PackParse(err.to_string()))?;
        if pack.domain != "fiction" {
            return Err(AntislopError::NotFictionDomain(pack.domain));
        }
        Ok(pack)
    }

    pub fn shelf(&self, id: &str) -> Option<&ShelfSpec> {
        self.shelves.iter().find(|shelf| shelf.id == id)
    }

    pub fn shelf_mut(&mut self, id: &str) -> Option<&mut ShelfSpec> {
        self.shelves.iter_mut().find(|shelf| shelf.id == id)
    }

    /// Product lock: `said_bookism` stays soft unless a profile promotes it.
    pub fn effective_severity(
        &self,
        shelf: &ShelfSpec,
        profile: Option<&GenreOverride>,
    ) -> Severity {
        if let Some(profile) = profile {
            if profile.promote_to_hard.iter().any(|id| id == &shelf.id) {
                return Severity::Hard;
            }
            if profile.soften.iter().any(|id| id == &shelf.id) {
                return Severity::Soft;
            }
        }
        if shelf.id == "said_bookism" && self.policy.said_bookism_soft_only_by_default {
            return Severity::Soft;
        }
        shelf.severity
    }

    pub fn is_enabled(&self, shelf: &ShelfSpec, profile: Option<&GenreOverride>) -> bool {
        if !shelf.enabled {
            return false;
        }
        if let Some(profile) = profile
            && profile.disable.iter().any(|id| id == &shelf.id)
        {
            return false;
        }
        true
    }

    pub fn rewrite_budget(&self) -> u8 {
        self.policy.max_rewrite_from_beats.clamp(1, 2)
    }

    pub fn shelf_ids(&self) -> Vec<String> {
        self.shelves.iter().map(|shelf| shelf.id.clone()).collect()
    }
}

/// Shelf IDs from the human catalog headings (`### \`id\``).
pub fn catalog_shelf_ids(markdown: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?m)^### `([a-z][a-z0-9_]+)`").expect("catalog ids"));
    re.captures_iter(markdown)
        .map(|cap| cap[1].to_string())
        .collect()
}
