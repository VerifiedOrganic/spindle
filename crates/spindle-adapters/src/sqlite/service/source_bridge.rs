use super::*;

impl SqliteSpindleService {
    /// Re-derive `source_start_offset` / `source_end_offset` for every
    /// `scene_source_link` row whose on-disk file is still readable,
    /// updating any row whose stored offsets disagree with the
    /// position-by-chapter mapping inferred from the file. Used to
    /// recover offsets after a manual file edit or a Spindle upgrade.
    /// Wraps `SourceBridge::backfill_offsets`.
    pub async fn backfill_scene_source_offsets(
        &self,
        input: spindle_core::models::BackfillSceneSourceOffsetsInput,
    ) -> Result<spindle_core::models::BackfillSceneSourceOffsetsOutput> {
        let bridge = crate::sqlite::source_bridge::SourceBridge::new(self.repository.clone());
        bridge
            .backfill_offsets(&input.project_id, &input.branch_id)
            .await
    }

    /// Read scene bodies back from an on-disk source file, updating each
    /// scene in place when the on-disk body differs and refreshing its
    /// `scene_source_link` offsets + SHA-256.
    ///
    /// Accepts both Spindle-managed delimited files (one chapter per
    /// file, scenes separated by `<!-- spindle-scene -->`) and
    /// externally-formatted manuscripts (Markdown `# Chapter` headers,
    /// `***` / `---` scene breaks, blank-line scene transitions, etc.).
    /// See `sqlite::source_bridge` for delimiter vs slicer routing.
    pub async fn pull_chapter_from_file(
        &self,
        input: spindle_core::models::PullChapterFromFileInput,
    ) -> Result<spindle_core::models::PullReport> {
        let bridge = crate::sqlite::source_bridge::SourceBridge::new(self.repository.clone());
        bridge
            .pull_chapter_from_file(&input.chapter_id, std::path::Path::new(&input.source_path))
            .await
    }

    /// Serialise every scene in a chapter to a single text file under
    /// `data_dir`, scene bodies separated by `<!-- spindle-scene -->`. Upserts a
    /// `scene_source_link` row per scene with the on-disk byte range and
    /// SHA-256 hash so subsequent `pull_chapter_from_file` and
    /// `backfill_scene_source_offsets` calls can round-trip cleanly.
    /// Mirrors `SourceBridge::push_chapter_to_file` in 705b835^.
    pub async fn push_chapter_to_file(
        &self,
        input: spindle_core::models::PushChapterToFileInput,
    ) -> Result<spindle_core::models::PushReport> {
        let bridge = crate::sqlite::source_bridge::SourceBridge::new(self.repository.clone());
        bridge
            .push_chapter_to_file(&input.chapter_id, std::path::Path::new(&input.target_path))
            .await
    }
}
