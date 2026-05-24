use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubjectTable {
    Project,
    WorldRule,
    Character,
    Location,
    Faction,
    Religion,
    Economy,
    PlotLine,
    Conflict,
    Theme,
    Motif,
    SystemOverlay,
    NarrativePromise,
    CharacterArc,
    Term,
    Relationship,
    TimelineEvent,
    Scene,
    Chapter,
    Book,
}

impl SubjectTable {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubjectTable::Project => "project",
            SubjectTable::WorldRule => "world_rule",
            SubjectTable::Character => "character",
            SubjectTable::Location => "location",
            SubjectTable::Faction => "faction",
            SubjectTable::Religion => "religion",
            SubjectTable::Economy => "economy",
            SubjectTable::PlotLine => "plot_line",
            SubjectTable::Conflict => "conflict",
            SubjectTable::Theme => "theme",
            SubjectTable::Motif => "motif",
            SubjectTable::SystemOverlay => "system_overlay",
            SubjectTable::NarrativePromise => "narrative_promise",
            SubjectTable::CharacterArc => "character_arc",
            SubjectTable::Term => "term",
            SubjectTable::Relationship => "relationship",
            SubjectTable::TimelineEvent => "timeline_event",
            SubjectTable::Scene => "scene",
            SubjectTable::Chapter => "chapter",
            SubjectTable::Book => "book",
        }
    }

    pub fn is_project(&self) -> bool {
        matches!(self, SubjectTable::Project)
    }
}

impl fmt::Display for SubjectTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for SubjectTable {
    type Err = SubjectError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "project" => Ok(SubjectTable::Project),
            "world_rule" => Ok(SubjectTable::WorldRule),
            "character" => Ok(SubjectTable::Character),
            "location" => Ok(SubjectTable::Location),
            "faction" => Ok(SubjectTable::Faction),
            "religion" => Ok(SubjectTable::Religion),
            "economy" => Ok(SubjectTable::Economy),
            "plot_line" => Ok(SubjectTable::PlotLine),
            "conflict" => Ok(SubjectTable::Conflict),
            "theme" => Ok(SubjectTable::Theme),
            "motif" => Ok(SubjectTable::Motif),
            "system_overlay" => Ok(SubjectTable::SystemOverlay),
            "narrative_promise" => Ok(SubjectTable::NarrativePromise),
            "character_arc" => Ok(SubjectTable::CharacterArc),
            "term" => Ok(SubjectTable::Term),
            "relationship" => Ok(SubjectTable::Relationship),
            "timeline_event" => Ok(SubjectTable::TimelineEvent),
            "scene" => Ok(SubjectTable::Scene),
            "chapter" => Ok(SubjectTable::Chapter),
            "book" => Ok(SubjectTable::Book),
            _ => Err(SubjectError::UnknownTable(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    table: SubjectTable,
    id: Option<String>,
}

impl Hash for Subject {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.table.hash(state);
        self.id.hash(state);
    }
}

impl Subject {
    pub fn project() -> Self {
        Subject {
            table: SubjectTable::Project,
            id: None,
        }
    }

    pub fn new(table: SubjectTable, id: impl Into<String>) -> Result<Self, SubjectError> {
        let id = id.into();
        let id = id.trim().to_string();
        if table.is_project() {
            return Err(SubjectError::ProjectNotAllowed);
        }
        if id.is_empty() {
            return Err(SubjectError::EmptyId);
        }
        Ok(Subject {
            table,
            id: Some(id),
        })
    }

    pub fn table(&self) -> SubjectTable {
        self.table
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn is_project_wide(&self) -> bool {
        self.id.is_none()
    }
}

impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref id) = self.id {
            write!(f, "{}:{}", self.table.as_str(), id)
        } else {
            write!(f, "{}", self.table.as_str())
        }
    }
}

impl FromStr for Subject {
    type Err = SubjectError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(SubjectError::EmptyString);
        }

        if let Some((table_str, id_str)) = s.split_once(':') {
            let table_str = table_str.trim();
            let id_str = id_str.trim();

            let table: SubjectTable = table_str.parse()?;

            if table.is_project() {
                return Err(SubjectError::ProjectWithId);
            }

            if id_str.is_empty() {
                return Err(SubjectError::MissingId);
            }

            Ok(Subject {
                table,
                id: Some(id_str.to_string()),
            })
        } else {
            let table: SubjectTable = s.parse()?;

            if table.is_project() {
                return Ok(Subject::project());
            }

            Err(SubjectError::MissingId)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectError {
    EmptyString,
    EmptyId,
    MissingId,
    ProjectWithId,
    ProjectNotAllowed,
    UnknownTable(String),
}

impl fmt::Display for SubjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubjectError::EmptyString => write!(f, "empty subject string"),
            SubjectError::EmptyId => write!(f, "empty id not allowed"),
            SubjectError::MissingId => write!(f, "id required for non-project tables"),
            SubjectError::ProjectWithId => {
                write!(f, "project must not include a colon separator")
            }
            SubjectError::ProjectNotAllowed => {
                write!(
                    f,
                    "use Subject::project() to construct a project-scoped subject"
                )
            }
            SubjectError::UnknownTable(t) => write!(f, "unknown table: {}", t),
        }
    }
}

impl std::error::Error for SubjectError {}

impl Serialize for Subject {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let len = if self.id.is_some() { 2 } else { 1 };
        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("table", self.table.as_str())?;
        if let Some(ref id) = self.id {
            map.serialize_entry("id", id)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Subject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            table: SubjectTable,
            id: Option<String>,
        }

        let repr = Repr::deserialize(deserializer)?;

        if repr.table.is_project() {
            if repr.id.is_some() {
                return Err(serde::de::Error::custom("project does not accept an id"));
            }
            return Ok(Subject::project());
        }

        let id = repr.id.unwrap_or_default();
        let id = id.trim().to_string();
        if id.is_empty() {
            return Err(serde::de::Error::custom(
                "id required for non-project tables",
            ));
        }

        Ok(Subject {
            table: repr.table,
            id: Some(id),
        })
    }
}

impl JsonSchema for Subject {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Subject")
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct SubjectRepr {
            table: SubjectTable,
            id: Option<String>,
        }

        SubjectRepr::json_schema(generator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_table_as_str() {
        assert_eq!(SubjectTable::Project.as_str(), "project");
        assert_eq!(SubjectTable::Character.as_str(), "character");
        assert_eq!(SubjectTable::PlotLine.as_str(), "plot_line");
        assert_eq!(SubjectTable::Book.as_str(), "book");
    }

    #[test]
    fn subject_table_from_str_trait() {
        assert_eq!("project".parse::<SubjectTable>(), Ok(SubjectTable::Project));
        assert_eq!(
            "character".parse::<SubjectTable>(),
            Ok(SubjectTable::Character)
        );
        assert_eq!(
            "plot_line".parse::<SubjectTable>(),
            Ok(SubjectTable::PlotLine)
        );
        assert_eq!("book".parse::<SubjectTable>(), Ok(SubjectTable::Book));
        assert!("invalid".parse::<SubjectTable>().is_err());
    }

    #[test]
    fn subject_project() {
        let s = Subject::project();
        assert!(s.table().is_project());
        assert!(s.id().is_none());
        assert!(s.is_project_wide());
    }

    #[test]
    fn subject_new_valid() {
        let s = Subject::new(SubjectTable::Character, "char_001").unwrap();
        assert_eq!(s.table(), SubjectTable::Character);
        assert_eq!(s.id(), Some("char_001"));
        assert!(!s.is_project_wide());
    }

    #[test]
    fn subject_new_empty_id() {
        let result = Subject::new(SubjectTable::Character, "");
        assert!(matches!(result, Err(SubjectError::EmptyId)));
    }

    #[test]
    fn subject_new_whitespace_only_id() {
        let result = Subject::new(SubjectTable::Character, "   ");
        assert!(matches!(result, Err(SubjectError::EmptyId)));
    }

    #[test]
    fn subject_new_trims_id() {
        let s = Subject::new(SubjectTable::Character, "  char_001  ").unwrap();
        assert_eq!(s.id(), Some("char_001"));
    }

    #[test]
    fn subject_new_project_not_allowed() {
        let result = Subject::new(SubjectTable::Project, "whatever");
        assert!(matches!(result, Err(SubjectError::ProjectNotAllowed)));
    }

    #[test]
    fn subject_display_project() {
        let s = Subject::project();
        assert_eq!(format!("{}", s), "project");
    }

    #[test]
    fn subject_display_with_id() {
        let s = Subject::new(SubjectTable::Character, "char_001").unwrap();
        assert_eq!(format!("{}", s), "character:char_001");
    }

    #[test]
    fn subject_from_str_project() {
        let s: Subject = "project".parse().unwrap();
        assert!(s.table().is_project());
        assert!(s.id().is_none());
    }

    #[test]
    fn subject_from_str_project_with_id_fails() {
        let result: Result<Subject, _> = "project:abc".parse();
        assert!(matches!(result, Err(SubjectError::ProjectWithId)));
    }

    #[test]
    fn subject_from_str_project_colon_empty_fails() {
        let result: Result<Subject, _> = "project:".parse();
        assert!(matches!(result, Err(SubjectError::ProjectWithId)));
    }

    #[test]
    fn subject_from_str_with_id() {
        let s: Subject = "character:char_001".parse().unwrap();
        assert_eq!(s.table(), SubjectTable::Character);
        assert_eq!(s.id(), Some("char_001"));
    }

    #[test]
    fn subject_from_str_missing_id_fails() {
        let result: Result<Subject, _> = "character:".parse();
        assert!(matches!(result, Err(SubjectError::MissingId)));
    }

    #[test]
    fn subject_from_str_no_colon_missing_id_fails() {
        let result: Result<Subject, _> = "character".parse();
        assert!(matches!(result, Err(SubjectError::MissingId)));
    }

    #[test]
    fn subject_from_str_invalid_table() {
        let result: Result<Subject, _> = "invalid_table:abc".parse();
        assert!(result.is_err());
    }

    #[test]
    fn round_trip_project() {
        let s = Subject::project();
        let display = format!("{}", s);
        let parsed: Subject = display.parse().unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn round_trip_with_id() {
        let s = Subject::new(SubjectTable::Character, "char_001").unwrap();
        let display = format!("{}", s);
        let parsed: Subject = display.parse().unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn serde_project_round_trip() {
        let s = Subject::project();
        let json = serde_json::to_string(&s).unwrap();
        let parsed: Subject = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn serde_with_id_round_trip() {
        let s = Subject::new(SubjectTable::Character, "char_001").unwrap();
        let json = serde_json::to_string(&s).unwrap();
        let parsed: Subject = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn serde_project_no_id_field() {
        let s = Subject::project();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"table":"project"}"#);
    }

    #[test]
    fn serde_rejects_character_without_id() {
        let json = r#"{"table":"character"}"#;
        let result: Result<Subject, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn serde_rejects_project_with_id() {
        let json = r#"{"table":"project","id":"whatever"}"#;
        let result: Result<Subject, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn serde_rejects_project_wide_character() {
        let json = r#"{"table":"character","id":null}"#;
        let result: Result<Subject, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn serde_trims_id() {
        let json = r#"{"table":"character","id":"  char_001  "}"#;
        let s: Subject = serde_json::from_str(json).unwrap();
        assert_eq!(s.id(), Some("char_001"));
    }

    #[test]
    fn serde_rejects_whitespace_only_id() {
        let json = r#"{"table":"character","id":"   "}"#;
        let result: Result<Subject, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn serde_uses_subject_table_deserialize() {
        let json = r#"{"table":"character","id":"abc"}"#;
        let s: Subject = serde_json::from_str(json).unwrap();
        assert_eq!(s.table(), SubjectTable::Character);

        let bad_json = r#"{"table":"not_a_table","id":"abc"}"#;
        let result: Result<Subject, _> = serde_json::from_str(bad_json);
        assert!(result.is_err());
    }

    #[test]
    fn subject_hash_key() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        let s = Subject::new(SubjectTable::Character, "abc").unwrap();
        set.insert(s.clone());
        assert!(set.contains(&s));
    }

    #[test]
    fn error_messages_differ() {
        let e1 = SubjectError::ProjectWithId;
        let e2 = SubjectError::ProjectNotAllowed;
        assert_ne!(e1.to_string(), e2.to_string());
    }

    fn all_tables() -> Vec<SubjectTable> {
        vec![
            SubjectTable::Project,
            SubjectTable::WorldRule,
            SubjectTable::Character,
            SubjectTable::Location,
            SubjectTable::Faction,
            SubjectTable::Religion,
            SubjectTable::Economy,
            SubjectTable::PlotLine,
            SubjectTable::Conflict,
            SubjectTable::Theme,
            SubjectTable::Motif,
            SubjectTable::SystemOverlay,
            SubjectTable::NarrativePromise,
            SubjectTable::CharacterArc,
            SubjectTable::Term,
            SubjectTable::Relationship,
            SubjectTable::TimelineEvent,
            SubjectTable::Scene,
            SubjectTable::Chapter,
            SubjectTable::Book,
        ]
    }

    #[test]
    fn all_subject_table_variants_display_round_trip() {
        let tables = all_tables();
        assert_eq!(tables.len(), 20);
        for table in &tables {
            let display = table.to_string();
            let parsed: SubjectTable = display.parse().unwrap();
            assert_eq!(parsed, *table, "round-trip failed for {:?}", table);
        }
    }

    #[test]
    fn all_subject_table_variants_serde_round_trip() {
        for table in all_tables() {
            let json = serde_json::to_string(&table).unwrap();
            let parsed: SubjectTable = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, table, "serde round-trip failed for {:?}", table);
        }
    }

    #[test]
    fn all_non_project_subjects_round_trip_display_from_str() {
        for table in all_tables() {
            if table.is_project() {
                continue;
            }
            let s = Subject::new(table, "test_id").unwrap();
            let display = format!("{}", s);
            let parsed: Subject = display.parse().unwrap();
            assert_eq!(parsed, s, "display round-trip failed for {:?}", table);
        }
    }

    #[test]
    fn all_non_project_subjects_round_trip_serde() {
        for table in all_tables() {
            if table.is_project() {
                continue;
            }
            let s = Subject::new(table, "test_id").unwrap();
            let json = serde_json::to_string(&s).unwrap();
            let parsed: Subject = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, s, "serde round-trip failed for {:?}", table);
        }
    }
}
