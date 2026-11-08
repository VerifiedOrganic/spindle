use super::*;

#[tokio::test]
async fn backfill_scene_source_offsets_repairs_drifted_offsets() {
    use spindle_core::models::{
        BackfillSceneSourceOffsetsInput, ContentRating, PushChapterToFileInput, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "P".into(),
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

    for (order, text) in [(1i32, "alpha"), (2, "beta")] {
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
            ..Default::default()
        })
        .await
        .unwrap();
    }

    let chapter = svc
        .repository()
        .find_chapter_by_number(&proj.project_id, 1, 1)
        .await
        .unwrap()
        .expect("chapter exists");

    // Push to create a file and `scene_source_link` rows with correct offsets.
    svc.push_chapter_to_file(PushChapterToFileInput {
        chapter_id: chapter.id.clone(),
        target_path: "ch1.txt".into(),
    })
    .await
    .unwrap();

    // Drift the stored offsets on one link. After this, backfill should
    // detect the drift and reset them to the correct file positions.
    let links = svc
        .repository()
        .list_scene_source_links_by_project(&proj.project_id)
        .await
        .unwrap();
    let mut target_link: Option<crate::sqlite::records::SceneSourceLink> = None;
    for link in &links {
        let scene = svc.repository().get_scene(&link.scene_id).await.unwrap();
        if scene.scene_order == 1 {
            target_link = Some(link.clone());
            break;
        }
    }
    let target_link = target_link.expect("link for scene 1 exists");
    svc.repository()
        .upsert_scene_source_link(
            &proj.project_id,
            &target_link.scene_id,
            &target_link.source_path,
            &target_link.content_sha256,
            Some(999), // intentionally wrong
            Some(1234),
        )
        .await
        .unwrap();

    let branch_id = svc
        .repository()
        .get_active_branch(&proj.project_id)
        .await
        .unwrap()
        .id;
    let out = svc
        .backfill_scene_source_offsets(BackfillSceneSourceOffsetsInput {
            project_id: proj.project_id.clone(),
            branch_id,
        })
        .await
        .unwrap();
    assert_eq!(out.scanned_links, 2);
    assert_eq!(out.updated_links, 1);
    assert_eq!(out.unresolved_links, 0);

    // Verify the offsets actually got fixed.
    let restored = svc
        .repository()
        .get_scene_source_link_for_scene(&target_link.scene_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.source_start_offset, Some(0));
    // The slicer now emits byte-exact body ranges (the trailing
    // `\n\n---\n\n` separator is stripped from each scene's
    // `source_byte_range`). For the Spindle-managed file
    // "alpha\n\n---\n\nbeta", scene 1's body is just "alpha" so
    // the end offset is 5.
    assert_eq!(restored.source_end_offset, Some(5));
}

#[tokio::test]
async fn pull_chapter_from_file_updates_scene_bodies() {
    use spindle_core::models::{
        ContentRating, PullChapterFromFileInput, PullStatus, PushChapterToFileInput,
        SaveSceneDraftInput, SceneSyncStatus,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "P".into(),
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

    for (order, text) in [(1i32, "first body"), (2, "second body")] {
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
            ..Default::default()
        })
        .await
        .unwrap();
    }

    let chapter = svc
        .repository()
        .find_chapter_by_number(&proj.project_id, 1, 1)
        .await
        .unwrap()
        .expect("chapter exists");

    // Push so we have a Spindle-formatted file on disk.
    let pushed = svc
        .push_chapter_to_file(PushChapterToFileInput {
            chapter_id: chapter.id.clone(),
            target_path: "ch1.txt".into(),
        })
        .await
        .unwrap();

    // Mutate the second scene body in the file.
    let updated_text = "first body\n\n---\n\nsecond body — REWRITTEN";
    std::fs::write(&pushed.target_path, updated_text).unwrap();

    // Pull and expect Diverged + Updated for scene 2, Match for scene 1.
    let report = svc
        .pull_chapter_from_file(PullChapterFromFileInput {
            chapter_id: chapter.id.clone(),
            source_path: "ch1.txt".into(),
        })
        .await
        .unwrap();
    assert!(matches!(report.status, PullStatus::Diverged));
    assert_eq!(report.scenes.len(), 2);
    assert!(matches!(report.scenes[0].status, SceneSyncStatus::Match));
    assert!(matches!(report.scenes[1].status, SceneSyncStatus::Updated));

    // Repository now reflects the pulled bodies.
    let scenes = svc
        .repository()
        .list_scenes_by_chapter(&chapter.id)
        .await
        .unwrap();
    let s2 = scenes.iter().find(|s| s.scene_order == 2).unwrap();
    assert_eq!(s2.full_text, "second body — REWRITTEN");
}

/// Regression: pull used to re-derive scene boundaries with the import
/// slicer's structural heuristics, so any edit that changed byte lengths
/// (or added a blank-line transition the slicer reads as a scene break)
/// failed with "could not map N scene range(s)" — the file round-trip
/// only worked for length-preserving edits. Spindle-delimited files now
/// split on the exact push-time delimiter, so real revisions map.
#[tokio::test]
async fn pull_chapter_from_file_survives_length_changing_edits() {
    use spindle_core::models::{
        ContentRating, PullChapterFromFileInput, PullStatus, PushChapterToFileInput,
        SaveSceneDraftInput, SceneSyncStatus,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "P".into(),
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

    for (order, text) in [(1i32, "first body"), (2, "second body"), (3, "third body")] {
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
            ..Default::default()
        })
        .await
        .unwrap();
    }

    let chapter = svc
        .repository()
        .find_chapter_by_number(&proj.project_id, 1, 1)
        .await
        .unwrap()
        .expect("chapter exists");
    let pushed = svc
        .push_chapter_to_file(PushChapterToFileInput {
            chapter_id: chapter.id.clone(),
            target_path: "ch1.txt".into(),
        })
        .await
        .unwrap();

    // Rewrite the FIRST scene much longer, including a blank-line
    // transition the import slicer would read as a phantom scene break
    // (long line, blank, short all-caps line). Under the old mapping
    // this either errored or — worse — shifted every later scene onto
    // the wrong text.
    let rewritten_scene_one = "Nate counted every bruise the long corridor had given him \
                                   twice over slowly.\n\nHUD FLASHED BRIGHT\n\nThen he moved on.";
    let updated_file =
        format!("{rewritten_scene_one}\n\n---\n\nsecond body\n\n---\n\nthird body\n");
    std::fs::write(&pushed.target_path, &updated_file).unwrap();

    let report = svc
        .pull_chapter_from_file(PullChapterFromFileInput {
            chapter_id: chapter.id.clone(),
            source_path: "ch1.txt".into(),
        })
        .await
        .expect("pull must map length-changing edits on a pushed file");
    assert!(matches!(report.status, PullStatus::Diverged));
    assert_eq!(report.scenes.len(), 3);
    assert!(matches!(report.scenes[0].status, SceneSyncStatus::Updated));
    // Later scenes shifted byte positions but are content-identical
    // (the editor's trailing newline is trimmed, not adopted).
    assert!(matches!(report.scenes[1].status, SceneSyncStatus::Match));
    assert!(matches!(report.scenes[2].status, SceneSyncStatus::Match));

    let scenes = svc
        .repository()
        .list_scenes_by_chapter(&chapter.id)
        .await
        .unwrap();
    let s1 = scenes.iter().find(|s| s.scene_order == 1).unwrap();
    assert_eq!(s1.full_text, rewritten_scene_one);
    let s2 = scenes.iter().find(|s| s.scene_order == 2).unwrap();
    assert_eq!(s2.full_text, "second body");
    let s3 = scenes.iter().find(|s| s.scene_order == 3).unwrap();
    assert_eq!(s3.full_text, "third body");
}

/// Adding or removing a scene in the file changes the delimiter split
/// count; pull must say so plainly instead of mis-mapping or emitting
/// the opaque "could not map N scene range(s)".
#[tokio::test]
async fn pull_chapter_from_file_scene_count_mismatch_errors_clearly() {
    use spindle_core::models::{
        ContentRating, PullChapterFromFileInput, PushChapterToFileInput, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "P".into(),
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
    for (order, text) in [(1i32, "first body"), (2, "second body")] {
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
            ..Default::default()
        })
        .await
        .unwrap();
    }
    let chapter = svc
        .repository()
        .find_chapter_by_number(&proj.project_id, 1, 1)
        .await
        .unwrap()
        .expect("chapter exists");
    let pushed = svc
        .push_chapter_to_file(PushChapterToFileInput {
            chapter_id: chapter.id.clone(),
            target_path: "ch1.txt".into(),
        })
        .await
        .unwrap();

    // The author splits scene 2 into two scenes in the file.
    std::fs::write(
            &pushed.target_path,
            "first body\n\n<!-- spindle-scene -->\n\nsecond body part one\n\n<!-- spindle-scene -->\n\nsecond body part two",
        )
        .unwrap();

    let err = svc
        .pull_chapter_from_file(PullChapterFromFileInput {
            chapter_id: chapter.id.clone(),
            source_path: "ch1.txt".into(),
        })
        .await
        .expect_err("a scene-count change must fail");
    let msg = format!("{err:#}");
    assert!(
        msg.contains('3'),
        "error names the file's scene count: {msg}"
    );
    assert!(
        msg.contains('2'),
        "error names the chapter's scene count: {msg}"
    );
    assert!(
        msg.contains("save_scene_draft"),
        "error names the recovery path: {msg}"
    );
}

#[tokio::test]
async fn push_chapter_to_file_writes_scenes_with_delimiter() {
    use spindle_core::models::{ContentRating, PushChapterToFileInput, SaveSceneDraftInput};

    let (tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "P".into(),
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

    for (order, text) in [(1i32, "scene one body"), (2, "scene two body")] {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: order,
            full_text: text.into(),
            summary: format!("summary {order}"),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    }

    // Find the chapter_id for chapter 1 of this project.
    let chapter = svc
        .repository()
        .find_chapter_by_number(&proj.project_id, 1, 1)
        .await
        .unwrap()
        .expect("chapter 1 should exist after save_scene_draft");

    // Push into a path inside data_dir.
    let rel = std::path::Path::new("ch1.txt");
    let report = svc
        .push_chapter_to_file(PushChapterToFileInput {
            chapter_id: chapter.id.clone(),
            target_path: rel.to_string_lossy().to_string(),
        })
        .await
        .unwrap();

    // Assert: 2 scene entries, file contains delimited bodies.
    assert_eq!(report.scenes.len(), 2);
    let written = std::fs::read_to_string(&report.target_path).unwrap();
    assert_eq!(
        written,
        "scene one body\n\n<!-- spindle-scene -->\n\nscene two body"
    );

    // Sanity: temp dir contains the file we just wrote.
    let _ = tmp;
}

/// Regression: the outside-data_dir error used to name `source_path`
/// even though push_chapter_to_file's parameter is `target_path`.
#[tokio::test]
async fn push_chapter_to_file_outside_data_dir_error_names_target_path() {
    use spindle_core::models::PushChapterToFileInput;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "P".into(),
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
    // create_project mints book 1 chapter 1.
    let chapter = svc
        .repository()
        .find_chapter_by_number(&proj.project_id, 1, 1)
        .await
        .unwrap()
        .expect("bootstrap chapter exists");

    let err = svc
        .push_chapter_to_file(PushChapterToFileInput {
            chapter_id: chapter.id.clone(),
            target_path: "/tmp/spindle-outside-data-dir.txt".into(),
        })
        .await
        .expect_err("a target outside data_dir must fail");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("target_path"),
        "error must name the target_path parameter, got: {msg}"
    );
    assert!(
        !msg.contains("source_path"),
        "error must not name a parameter the tool does not take, got: {msg}"
    );
}
