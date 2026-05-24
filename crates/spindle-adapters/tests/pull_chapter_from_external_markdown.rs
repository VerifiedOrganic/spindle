//! Integration test for `pull_chapter_from_file` against an external,
//! non-Spindle-managed Markdown manuscript.
//!
//! This exercises the slicer-backed codepath:
//! `crates/spindle-adapters/src/sqlite/source_bridge.rs::pull_chapter_from_file`
//! routes through `scene_offsets_from_import_slicer`, which runs the
//! import structural slicer (`# Chapter` headers, `***` / `---` separator
//! lines, blank-line transitions) over the source file and maps the
//! resulting scene byte ranges back to the project's `scene_source_link`
//! rows. Before the slicer port the function only handled Spindle-managed
//! delimited files (split on `\n\n---\n\n`); this test guards the
//! restored "external manuscript" capability.

use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    ContentRating, CreateProjectInput, PullChapterFromFileInput, PullStatus, ReaderContract,
    SaveSceneDraftInput, SceneSyncStatus,
};
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);
    (tmp, SqliteSpindleService::new(repo))
}

/// External-manuscript pull: a Markdown file with `# Chapter 1` header
/// and two scenes separated by a `***` line.
///
/// This is the shape the brief calls out for the restored slicer
/// codepath. Without the slicer the pull would fail outright (the file
/// has zero `\n\n---\n\n` chunks but the chapter has two scenes), so the
/// fact that the test passes is itself proof the slicer is wired.
#[tokio::test]
async fn pull_chapter_from_external_markdown_slices_into_scenes() {
    let (tmp, svc) = fresh_service().await;

    let proj = svc
        .create_project(CreateProjectInput {
            name: "External".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Two placeholder scenes — `pull_chapter_from_file` requires at least
    // one scene to exist in the target chapter, and the slicer's per-chapter
    // scene count must match the project's scene count for that chapter.
    // The slicer will detect two scenes in the Markdown file we write below,
    // so we create two placeholders with throwaway bodies.
    for (order, text) in [(1i32, "placeholder one"), (2, "placeholder two")] {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: order,
            full_text: text.into(),
            summary: format!("s{order}"),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();
    }

    let chapter = svc
        .repository()
        .find_chapter_by_number(&proj.project_id, 1, 1)
        .await
        .unwrap()
        .expect("chapter row should exist after save_scene_draft");

    // Write an externally-formatted Markdown file directly under `data_dir`
    // — that's the security envelope the bridge enforces. We use `# Chapter 1`
    // as an explicit chapter heading and a `***` scene separator. These are
    // canonical externally-formatted shapes the slicer is designed for.
    let scene_one_body =
        "She stood at the edge of the cliff, the wind whipping her hair into a tangle.";
    let scene_two_body = "The road back to town was longer than she remembered, and quieter.";
    let source_relative = "external/manuscript.md";
    let source_full_path = svc.repository().data_dir().join(source_relative);
    std::fs::create_dir_all(source_full_path.parent().unwrap()).unwrap();
    let source_content = format!("# Chapter 1\n\n{scene_one_body}\n\n***\n\n{scene_two_body}\n");
    std::fs::write(&source_full_path, &source_content).unwrap();
    // The bridge canonicalizes paths (resolving symlinks like `/var` →
    // `/private/var` on macOS). Canonicalize on our side too so the path
    // comparisons below are apples-to-apples.
    let source_full_path = std::fs::canonicalize(&source_full_path).unwrap();
    let source_disk_bytes = std::fs::read(&source_full_path).unwrap();
    let source_disk_len = source_disk_bytes.len();

    // Pull. Without the slicer wired this would error out ("file contains
    // 1 delimited chunk(s) but chapter has 2 scene(s)"); with the slicer
    // it must succeed and report Diverged because the placeholder bodies
    // differ from the on-disk bodies.
    let report = svc
        .pull_chapter_from_file(PullChapterFromFileInput {
            chapter_id: chapter.id.clone(),
            source_path: source_relative.into(),
        })
        .await
        .expect("pull_chapter_from_file should accept external Markdown");

    assert!(matches!(report.status, PullStatus::Diverged));
    assert_eq!(report.scenes.len(), 2);
    assert!(matches!(report.scenes[0].status, SceneSyncStatus::Updated));
    assert!(matches!(report.scenes[1].status, SceneSyncStatus::Updated));
    assert_eq!(report.source_path, source_full_path.to_string_lossy());
    assert_eq!(report.source_size_bytes, source_disk_len);

    // The reported byte ranges must be real, ordered, and non-overlapping —
    // i.e. produced by the slicer, not a default `(0, 0)` placeholder. We
    // verify they sit inside the source file and that scene 1 ends before
    // scene 2 begins.
    let scene_one_range = &report.scenes[0].byte_range_in_source;
    let scene_two_range = &report.scenes[1].byte_range_in_source;
    assert!(
        scene_one_range.start < scene_one_range.end,
        "scene 1 range must be non-empty, got {:?}",
        scene_one_range,
    );
    assert!(
        scene_two_range.start < scene_two_range.end,
        "scene 2 range must be non-empty, got {:?}",
        scene_two_range,
    );
    assert!(
        scene_one_range.end <= scene_two_range.start,
        "scene 1 end ({}) must come before scene 2 start ({})",
        scene_one_range.end,
        scene_two_range.start,
    );
    assert!(
        scene_two_range.end <= source_disk_len,
        "scene 2 end ({}) must fit inside the source file ({} bytes)",
        scene_two_range.end,
        source_disk_len,
    );

    // The reported byte range for each scene must point at the matching
    // scene body inside the source file. Markdown header extraction can
    // shift the slicer's offsets by a few bytes vs the raw source (the
    // slicer normalizes "# Chapter 1" to "Chapter 1"), so we assert the
    // slice contains the meaningful prefix of the body rather than an
    // exact byte-for-byte match. The prefix is long enough to be
    // unambiguous and to fail loudly if the offsets pointed at the wrong
    // scene (e.g. scene 1's body slipped into scene 2's range).
    let scene_one_slice =
        std::str::from_utf8(&source_disk_bytes[scene_one_range.start..scene_one_range.end])
            .unwrap();
    let scene_one_prefix = &scene_one_body[..scene_one_body.len() - 5];
    assert!(
        scene_one_slice.contains(scene_one_prefix),
        "scene 1 slice does not contain the scene one body prefix {:?}. \
         reported slice: {:?}",
        scene_one_prefix,
        scene_one_slice,
    );
    // Scene 1's slice must NOT contain scene 2's body — that would mean
    // the offsets straddled the scene boundary.
    assert!(
        !scene_one_slice.contains(scene_two_body),
        "scene 1 slice unexpectedly contains scene 2 body: {:?}",
        scene_one_slice,
    );

    let scene_two_slice =
        std::str::from_utf8(&source_disk_bytes[scene_two_range.start..scene_two_range.end])
            .unwrap();
    let scene_two_prefix = &scene_two_body[..scene_two_body.len() - 5];
    assert!(
        scene_two_slice.contains(scene_two_prefix),
        "scene 2 slice does not contain the scene two body prefix {:?}. \
         reported slice: {:?}",
        scene_two_prefix,
        scene_two_slice,
    );
    assert!(
        !scene_two_slice.contains(scene_one_body),
        "scene 2 slice unexpectedly contains scene 1 body: {:?}",
        scene_two_slice,
    );

    // The persisted scene_source_link rows must mirror the report's byte
    // ranges so a later `backfill_scene_source_offsets` (or
    // `evaluate_scene_divergence`) round-trip works.
    let scenes = svc
        .repository()
        .list_scenes_by_chapter(&chapter.id)
        .await
        .unwrap();
    let scene_one_id = scenes
        .iter()
        .find(|s| s.scene_order == 1)
        .map(|s| s.id.clone())
        .expect("scene 1 row exists");
    let scene_two_id = scenes
        .iter()
        .find(|s| s.scene_order == 2)
        .map(|s| s.id.clone())
        .expect("scene 2 row exists");

    let link_one = svc
        .repository()
        .get_scene_source_link_for_scene(&scene_one_id)
        .await
        .unwrap()
        .expect("scene 1 must now have a scene_source_link");
    assert_eq!(
        link_one.source_start_offset,
        Some(scene_one_range.start as i64),
        "scene 1 link start offset must match report",
    );
    assert_eq!(
        link_one.source_end_offset,
        Some(scene_one_range.end as i64),
        "scene 1 link end offset must match report",
    );
    assert_eq!(
        link_one.source_path,
        source_full_path.to_string_lossy().to_string(),
    );

    let link_two = svc
        .repository()
        .get_scene_source_link_for_scene(&scene_two_id)
        .await
        .unwrap()
        .expect("scene 2 must now have a scene_source_link");
    assert_eq!(
        link_two.source_start_offset,
        Some(scene_two_range.start as i64),
        "scene 2 link start offset must match report",
    );
    assert_eq!(
        link_two.source_end_offset,
        Some(scene_two_range.end as i64),
        "scene 2 link end offset must match report",
    );

    // The scene rows themselves must now carry the slicer-extracted bodies
    // (with the trailing-delimiter strip applied for the non-last scene).
    // We check the body PREFIX because the slicer's normalization can
    // include surrounding whitespace/separators in the slice.
    let scene_one = scenes.iter().find(|s| s.scene_order == 1).unwrap();
    let scene_two = scenes.iter().find(|s| s.scene_order == 2).unwrap();
    assert!(
        scene_one.full_text.contains(scene_one_prefix),
        "scene 1 full_text should contain the scene one body prefix, got: {:?}",
        scene_one.full_text,
    );
    assert_ne!(
        scene_one.full_text, "placeholder one",
        "scene 1 full_text must have been replaced by the pulled body",
    );
    assert!(
        scene_two.full_text.contains(scene_two_prefix),
        "scene 2 full_text should contain the scene two body prefix, got: {:?}",
        scene_two.full_text,
    );
    assert_ne!(
        scene_two.full_text, "placeholder two",
        "scene 2 full_text must have been replaced by the pulled body",
    );

    // Keep `tmp` alive so the data_dir + source file outlive the assertions.
    drop(tmp);
}

/// Byte-exact variant: each `scene_source_link` `(start, end)` must
/// slice the SOURCE file into EXACTLY the scene body (no off-by-2
/// from header-stripping, no separator bleed). The slicer now emits
/// `AnalyzedScene::source_byte_range` straight from the on-disk
/// bytes, and `extract_markdown` is a UTF-8 passthrough, so source
/// coordinates flow through end-to-end without translation.
#[tokio::test]
async fn pull_chapter_byte_ranges_slice_source_exactly() {
    let (tmp, svc) = fresh_service().await;

    let proj = svc
        .create_project(CreateProjectInput {
            name: "ByteExact".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    for (order, text) in [(1i32, "placeholder one"), (2, "placeholder two")] {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: order,
            full_text: text.into(),
            summary: format!("s{order}"),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();
    }

    let chapter = svc
        .repository()
        .find_chapter_by_number(&proj.project_id, 1, 1)
        .await
        .unwrap()
        .expect("chapter row should exist after save_scene_draft");

    let scene_one_body =
        "She stood at the edge of the cliff, the wind whipping her hair into a tangle.";
    let scene_two_body = "The road back to town was longer than she remembered, and quieter.";
    let source_relative = "external/byte-exact.md";
    let source_full_path = svc.repository().data_dir().join(source_relative);
    std::fs::create_dir_all(source_full_path.parent().unwrap()).unwrap();
    let source_content = format!("# Chapter 1\n\n{scene_one_body}\n\n***\n\n{scene_two_body}\n");
    std::fs::write(&source_full_path, &source_content).unwrap();
    let source_full_path = std::fs::canonicalize(&source_full_path).unwrap();
    let source_disk_bytes = std::fs::read(&source_full_path).unwrap();

    let report = svc
        .pull_chapter_from_file(PullChapterFromFileInput {
            chapter_id: chapter.id.clone(),
            source_path: source_relative.into(),
        })
        .await
        .expect("pull_chapter_from_file must accept external Markdown");

    assert_eq!(report.scenes.len(), 2);

    let scenes = svc
        .repository()
        .list_scenes_by_chapter(&chapter.id)
        .await
        .unwrap();
    let scene_one_id = scenes
        .iter()
        .find(|s| s.scene_order == 1)
        .map(|s| s.id.clone())
        .unwrap();
    let scene_two_id = scenes
        .iter()
        .find(|s| s.scene_order == 2)
        .map(|s| s.id.clone())
        .unwrap();
    let link_one = svc
        .repository()
        .get_scene_source_link_for_scene(&scene_one_id)
        .await
        .unwrap()
        .unwrap();
    let link_two = svc
        .repository()
        .get_scene_source_link_for_scene(&scene_two_id)
        .await
        .unwrap()
        .unwrap();

    let link_one_start = link_one.source_start_offset.unwrap() as usize;
    let link_one_end = link_one.source_end_offset.unwrap() as usize;
    let link_two_start = link_two.source_start_offset.unwrap() as usize;
    let link_two_end = link_two.source_end_offset.unwrap() as usize;

    let scene_one_slice =
        std::str::from_utf8(&source_disk_bytes[link_one_start..link_one_end]).unwrap();
    let scene_two_slice =
        std::str::from_utf8(&source_disk_bytes[link_two_start..link_two_end]).unwrap();

    assert_eq!(
        scene_one_slice, scene_one_body,
        "scene 1 byte range must slice EXACTLY the scene one body — \
         no header drift, no separator bleed",
    );
    assert_eq!(
        scene_two_slice, scene_two_body,
        "scene 2 byte range must slice EXACTLY the scene two body — \
         no header drift, no separator bleed",
    );

    drop(tmp);
}
