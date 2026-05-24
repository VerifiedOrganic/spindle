use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Range;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RecordId(pub String);

impl RecordId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for RecordId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

impl From<String> for RecordId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for RecordId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    Scene {
        scene_id: RecordId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        byte_range: Option<Range<usize>>,
    },
    Chapter {
        chapter_id: RecordId,
    },
    Book {
        book_id: RecordId,
    },
    File {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        byte_range: Option<Range<usize>>,
    },
    AssertedByAuthor {
        at: DateTime<Utc>,
    },
    Imported {
        source_path: String,
        at: DateTime<Utc>,
    },
    Derived {
        from: Box<Provenance>,
        by: String,
    },
}

impl Provenance {
    pub fn asserted_by_author(at: DateTime<Utc>) -> Self {
        Self::AssertedByAuthor { at }
    }

    pub fn scene(scene_id: RecordId, byte_range: Option<Range<usize>>) -> Self {
        Self::Scene {
            scene_id,
            byte_range,
        }
    }

    pub fn chapter(chapter_id: RecordId) -> Self {
        Self::Chapter { chapter_id }
    }

    pub fn book(book_id: RecordId) -> Self {
        Self::Book { book_id }
    }

    pub fn file(path: impl Into<String>, byte_range: Option<Range<usize>>) -> Self {
        Self::File {
            path: path.into(),
            byte_range,
        }
    }

    pub fn imported(source_path: impl Into<String>, at: DateTime<Utc>) -> Self {
        Self::Imported {
            source_path: source_path.into(),
            at,
        }
    }

    pub fn derived(from: Provenance, by: impl Into<String>) -> Self {
        Self::Derived {
            from: Box::new(from),
            by: by.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    #[test]
    fn round_trip_scene() {
        let p = Provenance::scene(RecordId::new("scene:abc123"), Some(10..50));
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn round_trip_scene_no_byte_range() {
        let p = Provenance::scene(RecordId::new("scene:abc123"), None);
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn round_trip_chapter() {
        let p = Provenance::chapter(RecordId::new("chapter:xyz789"));
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn round_trip_book() {
        let p = Provenance::book(RecordId::new("book:001"));
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn round_trip_file() {
        let p = Provenance::file("/path/to/chapter1.md", Some(100..250));
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn round_trip_file_no_byte_range() {
        let p = Provenance::file("/path/to/chapter1.md", None);
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn round_trip_asserted_by_author() {
        let p = Provenance::asserted_by_author(ts(2025, 1, 15));
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn round_trip_imported() {
        let p = Provenance::imported("manuscript.epub", ts(2025, 3, 1));
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn round_trip_derived() {
        let base = Provenance::scene(RecordId::new("scene:abc"), Some(0..100));
        let p = Provenance::derived(base, "summarization");
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn round_trip_nested_derived() {
        let base = Provenance::asserted_by_author(ts(2024, 6, 10));
        let mid = Provenance::derived(base, "extraction");
        let p = Provenance::derived(mid, "consolidation");
        let json = serde_json::to_string(&p).unwrap();
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn record_id_display() {
        let id = RecordId::new("scene:abc123");
        assert_eq!(format!("{id}"), "scene:abc123");
    }

    #[test]
    fn record_id_from_str() {
        let id: RecordId = "project:xyz".parse().unwrap();
        assert_eq!(id.0, "project:xyz");
    }

    #[test]
    fn provenance_json_structure() {
        let p = Provenance::scene(RecordId::new("scene:abc"), Some(10..20));
        let val: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(val["kind"], "scene");
        assert_eq!(val["scene_id"], "scene:abc");
        assert_eq!(val["byte_range"]["start"], 10);
        assert_eq!(val["byte_range"]["end"], 20);
    }

    #[test]
    fn asserted_by_author_json() {
        let p = Provenance::asserted_by_author(ts(2025, 1, 15));
        let val: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(val["kind"], "asserted_by_author");
        assert!(val["at"].is_string());
    }

    #[test]
    fn provenance_equality() {
        let p1 = Provenance::chapter(RecordId::new("ch:1"));
        let p2 = Provenance::chapter(RecordId::new("ch:1"));
        let p3 = Provenance::chapter(RecordId::new("ch:2"));
        assert_eq!(p1, p2);
        assert_ne!(p1, p3);
    }
}
