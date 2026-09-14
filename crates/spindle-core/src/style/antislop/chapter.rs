//! Rolling chapter counters for chapter-scoped shelf quotas.
//!
//! Scene-scoped advisory clusters (including `solitary_fade`) never enter
//! these counters. Numeric `limit_scope = "chapter"` shelves accumulate raw
//! match counts across scenes so scene 2 can consume leftover quota from
//! scene 1.

use super::pack::ShelfPack;
use super::scan::raw_match_count;
use std::collections::BTreeMap;

/// Per-chapter raw match usage for chapter-scoped shelves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChapterCounters {
    used: BTreeMap<String, u32>,
}

impl ChapterCounters {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one scene's raw chapter-scoped matches.
    pub fn add_scene(&mut self, pack: &ShelfPack, prose: &str) {
        for shelf in &pack.shelves {
            if !is_chapter_scoped(&shelf.limit_scope) {
                continue;
            }
            let n = raw_match_count(shelf, prose);
            if n > 0 {
                *self.used.entry(shelf.id.clone()).or_insert(0) += n;
            }
        }
    }

    pub fn used(&self, shelf_id: &str) -> u32 {
        self.used.get(shelf_id).copied().unwrap_or(0)
    }
}

pub fn is_chapter_scoped(limit_scope: &str) -> bool {
    limit_scope == "chapter"
}
