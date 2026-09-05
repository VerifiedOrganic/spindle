//! SourceBridge — read/write Spindle scenes against on-disk source files.
//!
//! It powers four service entry points:
//!   * `backfill_scene_source_offsets`
//!   * `pull_chapter_from_file`
//!   * `push_chapter_to_file`
//!   * the divergence-detection paths feeding `get_writer_state`'s
//!     `unsynced_local_files` / `drift_warnings` and `export_epub`'s
//!     `divergence_warnings`.
//!
//! Divergence detection (`evaluate_scene_divergence`) is faithful to the
//! reference: it tries a primary "unique span" match against the source
//! file, then falls back to the persisted tracked offsets when that match
//! is ambiguous or missing.
//!
//! `pull_chapter_from_file` and `backfill_offsets` route through the
//! import structural slicer (`crate::sqlite::import::{ingest_sources,
//! analyze_structure}`) so they accept both Spindle-managed delimited
//! files and arbitrary externally-formatted manuscripts (Markdown
//! `# Chapter` headers, `***` / `---` scene breaks, blank-line scene
//! transitions, etc.). See `scene_offsets_from_import_slicer` below for
//! the slicer-to-bridge translation logic.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use sha2::{Digest, Sha256};

use super::import::{IngestSourcesOptions, analyze_structure, ingest_sources};
use super::records::{Scene, SceneSourceLink};
use super::repository::Repository;
use spindle_core::models::*;

#[derive(Debug, Clone)]
enum SceneSourceHashResolution {
    Resolved { current_hash: String },
    Unknown { reason: String },
}

#[derive(Debug, Clone)]
pub(crate) struct SceneDivergenceObservation {
    pub kind: DivergenceKind,
    pub detail: String,
}

/// Per-(chapter_number, scene_position) byte ranges produced by running the
/// import structural slicer over a source file. Mirrors the SurrealDB-era
/// `SourceSceneOffsetsByPosition` from `services/source_bridge.rs` in
/// git ref `705b835^` — scene positions here are 1-based within each
/// chapter, matching the order the slicer emits.
struct SourceSceneOffsetsByPosition {
    scene_offsets: BTreeMap<(i32, i32), (usize, usize)>,
}

/// Marker `push_chapter_to_file` writes between scene bodies. An HTML comment
/// so it cannot be confused with a markdown thematic break (`---`).
const SOURCE_BRIDGE_SCENE_DELIMITER: &str = "\n\n<!-- spindle-scene -->\n\n";
/// Delimiter used by pushes before the dedicated marker. Pull still splits on
/// it when the piece count matches the chapter, so existing files round-trip.
const LEGACY_SOURCE_BRIDGE_SCENE_DELIMITER: &str = "\n\n---\n\n";

pub struct SourceBridge {
    repository: Repository,
}

impl SourceBridge {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }

    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    pub async fn divergence_for_scene(
        &self,
        scene_id: &str,
    ) -> anyhow::Result<Option<SourceBridgeDivergenceStatus>> {
        let scene = self.repository.get_scene(scene_id).await?;
        let link = match self
            .repository
            .get_scene_source_link_for_scene(scene_id)
            .await?
        {
            Some(l) => l,
            None => return Ok(None),
        };
        let observation = evaluate_scene_divergence(&link, &scene);
        Ok(observation.map(|obs| SourceBridgeDivergenceStatus {
            scene_id: scene_id.to_string(),
            source_path: link.source_path.clone(),
            kind: obs.kind,
            detail: obs.detail,
        }))
    }

    pub async fn pull_chapter_from_file(
        &self,
        chapter_id: &str,
        path: &Path,
    ) -> anyhow::Result<PullReport> {
        let chapter = self.repository.get_chapter(chapter_id).await?;
        let source_path = resolve_source_bridge_path(
            self.repository.data_dir(),
            path,
            true,
            "pull_chapter_from_file",
            "source_path",
        )?;
        let source_path_string = source_path.to_string_lossy().to_string();
        let source_text = std::fs::read_to_string(&source_path)
            .with_context(|| format!("failed to read source file {}", source_path.display()))?;

        let scenes = self.repository.list_scenes_by_chapter(chapter_id).await?;
        if scenes.is_empty() {
            anyhow::bail!(
                "pull_chapter_from_file requires at least one scene in the target chapter"
            );
        }

        // Map each db scene to its byte range in the file, aligned to the
        // scene_order-sorted scene list. Three mapping paths:
        //
        // 1. Files that contain the dedicated Spindle marker (anything
        //    push_chapter_to_file writes now): split on that marker. Count
        //    mismatch is a real add/remove and errors rather than guessing.
        // 2. Legacy Spindle pushes that used `\n\n---\n\n`: split that way
        //    only when the piece count matches the chapter. A markdown HR
        //    inside a scene would change the count, so those files fall
        //    through to the slicer instead of being mis-split.
        // 3. External files: the import slicer's structural analysis.
        let mut sorted_scenes: Vec<&Scene> = scenes.iter().collect();
        sorted_scenes.sort_by_key(|scene| scene.scene_order);

        let aligned_ranges: Vec<Option<(usize, usize)>> =
            if source_text.contains(SOURCE_BRIDGE_SCENE_DELIMITER) {
                let delimiter_ranges =
                    delimiter_split_ranges(&source_text, SOURCE_BRIDGE_SCENE_DELIMITER);
                if delimiter_ranges.len() != sorted_scenes.len() {
                    anyhow::bail!(
                        "pull_chapter_from_file found {} scene body/bodies in the source file \
                         (split on the Spindle scene delimiter) but the chapter has {} scene(s); \
                         scenes were added or removed in the file — mirror the change in Spindle \
                         (save_scene_draft / delete_scene) and re-push, or restore the delimiter \
                         structure",
                        delimiter_ranges.len(),
                        sorted_scenes.len()
                    );
                }
                delimiter_ranges.into_iter().map(Some).collect()
            } else {
                let legacy_ranges =
                    delimiter_split_ranges(&source_text, LEGACY_SOURCE_BRIDGE_SCENE_DELIMITER);
                if legacy_ranges.len() == sorted_scenes.len() && legacy_ranges.len() > 1 {
                    legacy_ranges.into_iter().map(Some).collect()
                } else if sorted_scenes.len() == 1 {
                    // Single-scene chapter: the whole trimmed file is the body.
                    vec![Some(trim_range(&source_text, 0, source_text.len()))]
                } else {
                    let slicer_offsets = scene_offsets_from_import_slicer(
                        self.repository.data_dir(),
                        &source_path_string,
                    )?;
                    // Slicer scenes come out dense (1, 2, 3, ...); db scene_orders
                    // may be sparse from deletes. Sorted-order is the contract.
                    sorted_scenes
                        .iter()
                        .enumerate()
                        .map(|(pos_0, scene)| {
                            let _ = scene;
                            slicer_offsets
                                .scene_offsets
                                .get(&(chapter.chapter_number, (pos_0 + 1) as i32))
                                .copied()
                        })
                        .collect()
                }
            };
        let last_scene_order = scenes
            .iter()
            .map(|scene| scene.scene_order)
            .max()
            .unwrap_or(0);

        struct StagedPullUpdate {
            scene_id: String,
            full_text: String,
            content_sha256: String,
            source_start_offset: usize,
            source_end_offset: usize,
            report_entry: PullSceneEntry,
            changed: bool,
        }

        let mut staged_updates = Vec::with_capacity(scenes.len());
        let mut unmatched_text_ranges = Vec::new();

        for (scene, aligned_range) in sorted_scenes.iter().zip(aligned_ranges.iter()) {
            let Some((source_start_offset, source_end_offset)) = *aligned_range else {
                unmatched_text_ranges.push(TextByteRange { start: 0, end: 0 });
                continue;
            };
            if source_end_offset <= source_start_offset {
                unmatched_text_ranges.push(TextByteRange {
                    start: source_start_offset,
                    end: source_end_offset,
                });
                continue;
            }

            // Trim a trailing Spindle-managed delimiter if one slipped into
            // the slicer's slice (only happens when this isn't the last
            // scene). Mirrors the SurrealDB reference.
            let mut effective_end_offset = source_end_offset;
            let raw_slice = source_text
                .get(source_start_offset..effective_end_offset)
                .context("source slice was out of UTF-8 bounds")?;
            let source_slice = if scene.scene_order < last_scene_order {
                if let Some(stripped) = strip_trailing_scene_delimiter(raw_slice) {
                    effective_end_offset -= raw_slice.len() - stripped.len();
                    stripped
                } else {
                    raw_slice
                }
            } else {
                raw_slice
            };

            let chars_added = source_slice
                .chars()
                .count()
                .saturating_sub(scene.full_text.chars().count());
            let chars_deleted = scene
                .full_text
                .chars()
                .count()
                .saturating_sub(source_slice.chars().count());
            let changed = source_slice != scene.full_text;

            staged_updates.push(StagedPullUpdate {
                scene_id: scene.id.clone(),
                full_text: source_slice.to_string(),
                content_sha256: sha256_hex(source_slice.as_bytes()),
                source_start_offset,
                source_end_offset: effective_end_offset,
                report_entry: PullSceneEntry {
                    scene_id: scene.id.clone(),
                    position: scene.scene_order as u32,
                    byte_range_in_source: TextByteRange {
                        start: source_start_offset,
                        end: effective_end_offset,
                    },
                    status: if changed {
                        SceneSyncStatus::Updated
                    } else {
                        SceneSyncStatus::Match
                    },
                    diff: if changed {
                        Some(TextDiffSummary {
                            chars_added,
                            chars_deleted,
                            chunks: Vec::new(),
                        })
                    } else {
                        None
                    },
                },
                changed,
            });
        }

        if !unmatched_text_ranges.is_empty() {
            anyhow::bail!(
                "pull_chapter_from_file could not map {} scene range(s) from source file",
                unmatched_text_ranges.len()
            );
        }

        let changed_any = staged_updates.iter().any(|staged| staged.changed);
        let branch_id = self
            .repository
            .active_branch_id_public(&chapter.project_id)
            .await?;
        for staged in &staged_updates {
            if staged.changed {
                let save_input = SaveSceneDraftInput {
                    project_id: chapter.project_id.clone(),
                    book_number: chapter.book_number,
                    chapter_number: chapter.chapter_number,
                    chapter_id: Some(chapter.id.clone()),
                    scene_order: scene_order_from_id(&scenes, &staged.scene_id),
                    full_text: staged.full_text.clone(),
                    summary: summary_for_scene(&scenes, &staged.scene_id),
                    content_rating: parse_scene_content_rating(content_rating_for_scene(
                        &scenes,
                        &staged.scene_id,
                    ))?,
                    tone: tone_for_scene(&scenes, &staged.scene_id),
                    source_path: Some(source_path_string.clone()),
                    generation_id: None,
                    ..Default::default()
                };
                self.repository
                    .save_scene_draft(&chapter.project_id, &branch_id, &save_input)
                    .await?;
            }
            self.repository
                .upsert_scene_source_link(
                    &chapter.project_id,
                    &staged.scene_id,
                    &source_path_string,
                    &staged.content_sha256,
                    Some(staged.source_start_offset as i64),
                    Some(staged.source_end_offset as i64),
                )
                .await?;
        }
        let report_scenes = staged_updates
            .into_iter()
            .map(|staged| staged.report_entry)
            .collect::<Vec<_>>();

        Ok(PullReport {
            chapter_id: chapter.id.clone(),
            source_path: source_path_string,
            source_size_bytes: source_text.len(),
            scenes: report_scenes,
            unmatched_text_ranges,
            status: if changed_any {
                PullStatus::Diverged
            } else {
                PullStatus::Clean
            },
        })
    }

    pub async fn push_chapter_to_file(
        &self,
        chapter_id: &str,
        path: &Path,
    ) -> anyhow::Result<PushReport> {
        let chapter = self.repository.get_chapter(chapter_id).await?;
        let source_path = resolve_source_bridge_path(
            self.repository.data_dir(),
            path,
            false,
            "push_chapter_to_file",
            "target_path",
        )?;
        let source_path_string = source_path.to_string_lossy().to_string();
        let scenes = self.repository.list_scenes_by_chapter(chapter_id).await?;

        let mut source_text = String::new();
        let mut entries = Vec::with_capacity(scenes.len());
        let mut pending_links = Vec::with_capacity(scenes.len());
        for (index, scene) in scenes.iter().enumerate() {
            let start = source_text.len();
            source_text.push_str(&scene.full_text);
            let end = source_text.len();
            if index + 1 < scenes.len() {
                source_text.push_str(SOURCE_BRIDGE_SCENE_DELIMITER);
            }
            pending_links.push((
                scene.id.clone(),
                sha256_hex(scene.full_text.as_bytes()),
                start,
                end,
            ));
            entries.push(PushSceneEntry {
                scene_id: scene.id.clone(),
                position: scene.scene_order as u32,
                byte_range_in_file: TextByteRange { start, end },
            });
        }

        std::fs::write(&source_path, &source_text)
            .with_context(|| format!("failed to write source file {}", source_path.display()))?;
        for (scene_id, content_sha256, start, end) in pending_links {
            self.repository
                .upsert_scene_source_link(
                    &chapter.project_id,
                    &scene_id,
                    &source_path_string,
                    &content_sha256,
                    Some(start as i64),
                    Some(end as i64),
                )
                .await?;
        }

        Ok(PushReport {
            chapter_id: chapter.id.clone(),
            target_path: source_path_string,
            scenes: entries,
        })
    }

    pub async fn backfill_offsets(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> anyhow::Result<BackfillSceneSourceOffsetsOutput> {
        self.repository.get_project(project_id).await?;
        let branch = self.repository.get_branch(branch_id).await?;
        if let Some(branch_project) = &branch.project_id
            && branch_project != project_id
        {
            anyhow::bail!("branch does not belong to the requested project");
        }

        let scenes = self
            .repository
            .list_scenes_by_project_and_branch(project_id, branch_id)
            .await?;
        let scenes_by_id: BTreeMap<String, &Scene> = scenes
            .iter()
            .map(|scene| (scene.id.clone(), scene))
            .collect();
        let source_links = self
            .repository
            .list_scene_source_links_by_project(project_id)
            .await?;

        let mut links_by_source_path: BTreeMap<String, Vec<&SceneSourceLink>> = BTreeMap::new();
        for link in &source_links {
            links_by_source_path
                .entry(link.source_path.clone())
                .or_default()
                .push(link);
        }

        let mut updated_links = 0usize;
        let mut unresolved_links = 0usize;
        let mut skipped_links = 0usize;

        for (source_path, links) in links_by_source_path {
            // We still read the source file to validate it exists +
            // length-check upserts; we no longer need a separate
            // normalization pass — the slicer emits source byte ranges
            // directly via `AnalyzedScene::source_byte_range`.
            let source_text = match std::fs::read_to_string(&source_path) {
                Ok(t) => t,
                Err(_) => {
                    unresolved_links += links.len();
                    continue;
                }
            };
            let managed_ranges = source_text
                .contains(SOURCE_BRIDGE_SCENE_DELIMITER)
                .then(|| delimiter_split_ranges(&source_text, SOURCE_BRIDGE_SCENE_DELIMITER));
            let slicer_offsets = if managed_ranges.is_some() {
                SourceSceneOffsetsByPosition {
                    scene_offsets: BTreeMap::new(),
                }
            } else {
                match scene_offsets_from_import_slicer(self.repository.data_dir(), &source_path) {
                    Ok(offsets) => offsets,
                    Err(_) => {
                        unresolved_links += links.len();
                        continue;
                    }
                }
            };

            let links_by_scene_id: BTreeMap<String, &SceneSourceLink> = links
                .iter()
                .map(|link| (link.scene_id.clone(), *link))
                .collect();

            let db_scenes_by_chapter: BTreeMap<i32, Vec<&Scene>> = scenes_by_id
                .iter()
                .filter(|(id, _)| links_by_scene_id.contains_key(*id))
                .map(|(_, scene)| (scene.chapter_number, *scene))
                .fold(BTreeMap::new(), |mut acc, (ch, scene)| {
                    acc.entry(ch).or_insert_with(Vec::new).push(scene);
                    acc
                });

            // Slicer scenes are dense 1-based; db scene_orders may be
            // sparse. Sort each chapter's scene_orders and zip the
            // i-th db scene_order to the i-th slicer position.
            let mut positional_offsets: BTreeMap<(i32, i32), (usize, usize)> = BTreeMap::new();
            for (chapter_number, scenes) in &db_scenes_by_chapter {
                let mut sorted_scene_orders: Vec<i32> =
                    scenes.iter().map(|s| s.scene_order).collect();
                sorted_scene_orders.sort_unstable();
                if let Some(ranges) = &managed_ranges {
                    // A push writes exactly one chapter. Use its explicit
                    // boundaries, just as pull does, never the prose slicer's
                    // guesses about blank lines or Markdown horizontal rules.
                    if db_scenes_by_chapter.len() == 1 && ranges.len() == sorted_scene_orders.len()
                    {
                        positional_offsets.extend(
                            sorted_scene_orders
                                .iter()
                                .zip(ranges)
                                .map(|(order, range)| ((*chapter_number, *order), *range)),
                        );
                    }
                    continue;
                }
                for (pos_0, scene_order) in sorted_scene_orders.iter().enumerate() {
                    let slicer_key = (*chapter_number, (pos_0 + 1) as i32);
                    if let Some(range) = slicer_offsets.scene_offsets.get(&slicer_key).copied() {
                        positional_offsets.insert((*chapter_number, *scene_order), range);
                    }
                }
            }

            for link in links {
                let Some(scene) = scenes_by_id.get(&link.scene_id) else {
                    skipped_links += 1;
                    continue;
                };

                let Some((source_start_offset, source_end_offset)) = positional_offsets
                    .get(&(scene.chapter_number, scene.scene_order))
                    .copied()
                else {
                    unresolved_links += 1;
                    continue;
                };
                if source_end_offset <= source_start_offset {
                    unresolved_links += 1;
                    continue;
                }
                if link.source_start_offset == Some(source_start_offset as i64)
                    && link.source_end_offset == Some(source_end_offset as i64)
                {
                    continue;
                }
                self.repository
                    .upsert_scene_source_link(
                        project_id,
                        &link.scene_id,
                        &link.source_path,
                        &link.content_sha256,
                        Some(source_start_offset as i64),
                        Some(source_end_offset as i64),
                    )
                    .await?;
                updated_links += 1;
            }
        }

        Ok(BackfillSceneSourceOffsetsOutput {
            scanned_links: source_links.len(),
            updated_links,
            unresolved_links,
            skipped_links,
        })
    }
}

fn parse_scene_content_rating(value: &str) -> anyhow::Result<ContentRating> {
    match value.to_ascii_lowercase().as_str() {
        "general" => Ok(ContentRating::General),
        "teen" => Ok(ContentRating::Teen),
        "mature" => Ok(ContentRating::Mature),
        "explicit" => Ok(ContentRating::Explicit),
        _ => anyhow::bail!("invalid content_rating"),
    }
}

fn scene_order_from_id(scenes: &[Scene], scene_id: &str) -> i32 {
    scenes
        .iter()
        .find(|s| s.id == scene_id)
        .map(|s| s.scene_order)
        .unwrap_or_default()
}

fn summary_for_scene(scenes: &[Scene], scene_id: &str) -> String {
    scenes
        .iter()
        .find(|s| s.id == scene_id)
        .map(|s| s.summary.clone())
        .unwrap_or_default()
}

fn content_rating_for_scene<'a>(scenes: &'a [Scene], scene_id: &str) -> &'a str {
    scenes
        .iter()
        .find(|s| s.id == scene_id)
        .map(|s| s.content_rating.as_str())
        .unwrap_or("General")
}

fn tone_for_scene(scenes: &[Scene], scene_id: &str) -> Option<String> {
    scenes
        .iter()
        .find(|s| s.id == scene_id)
        .and_then(|s| s.tone.clone())
}

fn resolve_source_bridge_path(
    data_dir: &Path,
    source_path: &Path,
    must_exist: bool,
    tool_name: &str,
    param_name: &str,
) -> anyhow::Result<PathBuf> {
    if source_path.as_os_str().is_empty() {
        anyhow::bail!("{tool_name} requires a non-empty {param_name}");
    }
    if source_path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        anyhow::bail!("{tool_name} rejected {param_name} with parent-directory traversal");
    }

    let resolved = if source_path.is_absolute() {
        source_path.to_path_buf()
    } else {
        data_dir.join(source_path)
    };
    let allowed_root = std::fs::canonicalize(data_dir).with_context(|| {
        format!(
            "{tool_name} failed to resolve data_dir {}",
            data_dir.display()
        )
    })?;

    if must_exist {
        let canonical = std::fs::canonicalize(&resolved).with_context(|| {
            format!(
                "{tool_name} failed to resolve {param_name} {}",
                resolved.display()
            )
        })?;
        if !canonical.starts_with(&allowed_root) {
            anyhow::bail!(
                "{tool_name} {param_name} must be inside data_dir ({})",
                allowed_root.display()
            );
        }
        return Ok(canonical);
    }

    let parent = resolved
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{tool_name} {param_name} must include a filename"))?;
    let mut existing_ancestor = parent.to_path_buf();
    while !existing_ancestor.exists() {
        existing_ancestor = existing_ancestor
            .parent()
            .ok_or_else(|| anyhow::anyhow!("{tool_name} {param_name} parent is invalid"))?
            .to_path_buf();
    }
    let canonical_ancestor = std::fs::canonicalize(&existing_ancestor).with_context(|| {
        format!(
            "{tool_name} failed to resolve parent ancestor {}",
            existing_ancestor.display()
        )
    })?;
    if !canonical_ancestor.starts_with(&allowed_root) {
        anyhow::bail!(
            "{tool_name} {param_name} must be inside data_dir ({})",
            allowed_root.display()
        );
    }
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "{tool_name} failed to create parent directory {}",
            parent.display()
        )
    })?;
    let canonical_parent = std::fs::canonicalize(parent).with_context(|| {
        format!(
            "{tool_name} failed to resolve parent directory {}",
            parent.display()
        )
    })?;
    if !canonical_parent.starts_with(&allowed_root) {
        anyhow::bail!(
            "{tool_name} {param_name} must be inside data_dir ({})",
            allowed_root.display()
        );
    }
    let filename = resolved
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{tool_name} {param_name} must include a filename"))?;
    Ok(canonical_parent.join(filename))
}

pub(crate) fn evaluate_scene_divergence(
    link: &SceneSourceLink,
    scene: &Scene,
) -> Option<SceneDivergenceObservation> {
    let path = Path::new(&link.source_path);
    if !path.exists() {
        return Some(SceneDivergenceObservation {
            kind: DivergenceKind::SourceMissing,
            detail: "local source file no longer exists".to_string(),
        });
    }

    let source_resolution = match resolve_scene_source_hash(link, scene) {
        Ok(resolution) => resolution,
        Err(err) => {
            return Some(SceneDivergenceObservation {
                kind: DivergenceKind::Unknown,
                detail: format!("unable to read source file: {err}"),
            });
        }
    };

    match source_resolution {
        SceneSourceHashResolution::Unknown { reason } => Some(SceneDivergenceObservation {
            kind: DivergenceKind::Unknown,
            detail: format!("divergence unresolved: {reason}"),
        }),
        SceneSourceHashResolution::Resolved { current_hash } => {
            if current_hash != link.content_sha256 {
                Some(SceneDivergenceObservation {
                    kind: DivergenceKind::ContentMismatch,
                    detail: format!(
                        "local file content differs from Spindle scene (expected hash {}, got {})",
                        &link.content_sha256[..8],
                        &current_hash[..8]
                    ),
                })
            } else {
                None
            }
        }
    }
}

#[allow(dead_code)]
pub(crate) fn divergence_observation_to_consistency_issue(
    observation: &SceneDivergenceObservation,
    source_path: &str,
    scene_id: &str,
) -> ConsistencyIssue {
    match observation.kind {
        DivergenceKind::ContentMismatch => ConsistencyIssue {
            severity: "error".to_string(),
            check_type: "scene_divergence".to_string(),
            message: format!(
                "local file '{}' has changed since scene {} was last saved to Spindle",
                source_path, scene_id
            ),
            entity_ids: vec![scene_id.to_string()],
            suggested_action: Some(
                "adopt the file's edits with pull_chapter_from_file, or discard them by \
                 re-saving the scene / re-pushing with push_chapter_to_file"
                    .to_string(),
            ),
        },
        DivergenceKind::SourceMissing => ConsistencyIssue {
            severity: "warning".to_string(),
            check_type: "scene_divergence".to_string(),
            message: format!(
                "source file '{}' for scene {} no longer exists on disk",
                source_path, scene_id
            ),
            entity_ids: vec![scene_id.to_string()],
            suggested_action: Some("re-link the scene to its current file location".to_string()),
        },
        DivergenceKind::Unknown => ConsistencyIssue {
            severity: "warning".to_string(),
            check_type: "scene_divergence".to_string(),
            message: format!(
                "unable to determine divergence for scene {} from '{}': {}",
                scene_id, source_path, observation.detail
            ),
            entity_ids: vec![scene_id.to_string()],
            suggested_action: Some(
                "run backfill_scene_source_offsets and relink the scene if offsets remain unresolved"
                    .to_string(),
            ),
        },
    }
}

pub fn find_unique_scene_source_span(
    source_text: &str,
    scene_text: &str,
) -> Option<(usize, usize)> {
    if scene_text.is_empty() {
        return None;
    }
    let mut matches = source_text.match_indices(scene_text);
    let start = matches.next()?.0;
    if matches.next().is_some() {
        return None;
    }
    Some((start, start + scene_text.len()))
}

/// Byte ranges of the scene bodies in a Spindle-delimited source file —
/// the exact inverse of `push_chapter_to_file`'s write loop. Each piece is
/// trimmed of leading/trailing whitespace (editors routinely add a trailing
/// newline) with the offsets adjusted to cover only the body. A file with
/// no delimiter yields a single range covering the trimmed whole.
fn delimiter_split_ranges(source_text: &str, delimiter: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    for (index, _) in source_text.match_indices(delimiter) {
        ranges.push(trim_range(source_text, cursor, index));
        cursor = index + delimiter.len();
    }
    ranges.push(trim_range(source_text, cursor, source_text.len()));
    ranges
}

fn strip_trailing_scene_delimiter(slice: &str) -> Option<&str> {
    for delimiter in [
        SOURCE_BRIDGE_SCENE_DELIMITER,
        LEGACY_SOURCE_BRIDGE_SCENE_DELIMITER,
    ] {
        if let Some(stripped) = slice.strip_suffix(delimiter) {
            return Some(stripped);
        }
    }
    None
}

fn trim_range(text: &str, start: usize, end: usize) -> (usize, usize) {
    let slice = &text[start..end];
    let lead = slice.len() - slice.trim_start().len();
    let trimmed_len = slice.trim().len();
    (start + lead, start + lead + trimmed_len)
}

fn tracked_scene_source_slice<'a>(
    source_text: &'a str,
    link: &SceneSourceLink,
) -> Result<&'a str, &'static str> {
    let Some(start_raw) = link.source_start_offset else {
        return Err("missing tracked source offsets");
    };
    let Some(end_raw) = link.source_end_offset else {
        return Err("missing tracked source offsets");
    };
    if start_raw < 0 || end_raw < 0 {
        return Err("tracked source offsets are out of bounds");
    }
    if end_raw <= start_raw {
        return Err("tracked source offsets are invalid");
    }
    let start = start_raw as usize;
    let end = end_raw as usize;
    source_text
        .get(start..end)
        .ok_or("tracked source offsets are out of bounds")
}

fn resolve_scene_source_hash(
    link: &SceneSourceLink,
    scene: &Scene,
) -> anyhow::Result<SceneSourceHashResolution> {
    let source_text = std::fs::read_to_string(&link.source_path)
        .with_context(|| format!("failed to read source file {}", link.source_path))?;

    if let Some((start, end)) = find_unique_scene_source_span(&source_text, &scene.full_text) {
        return Ok(SceneSourceHashResolution::Resolved {
            current_hash: sha256_hex(&source_text.as_bytes()[start..end]),
        });
    }

    let tracked_failure_reason = match tracked_scene_source_slice(&source_text, link) {
        Ok(slice) => {
            return Ok(SceneSourceHashResolution::Resolved {
                current_hash: sha256_hex(slice.as_bytes()),
            });
        }
        Err(reason) => reason,
    };
    Ok(SceneSourceHashResolution::Unknown {
        reason: tracked_failure_reason.to_string(),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

/// Run the import structural slicer over one source file and reshape
/// the result into `(chapter_number, scene_position) -> (start, end)`
/// byte offsets in the ORIGINAL source file.
///
/// Reads `AnalyzedScene::source_byte_range` straight out of the slicer
/// output — the slicer already tracks the original-source byte
/// positions while normalizing. No second normalization pass needed on
/// the bridge side; the returned coordinates index into the unmodified
/// source file bytes.
fn scene_offsets_from_import_slicer(
    data_dir: &Path,
    source_path: &str,
) -> anyhow::Result<SourceSceneOffsetsByPosition> {
    let source_documents = ingest_sources(
        &[PathBuf::from(source_path)],
        IngestSourcesOptions::new(data_dir),
    )?;
    let analysis = analyze_structure(source_documents);
    analysis
        .source_documents
        .first()
        .context("import slicer returned no source documents")?;
    let mut scene_offsets = BTreeMap::new();
    for chapter in &analysis.chapters {
        for (pos, scene) in chapter.scenes.iter().enumerate() {
            scene_offsets.insert(
                (chapter.chapter_number, (pos + 1) as i32),
                scene.source_byte_range,
            );
        }
    }
    Ok(SourceSceneOffsetsByPosition { scene_offsets })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_unique_scene_source_span_finds_unique_match() {
        let source = "alpha beta gamma";
        let result = find_unique_scene_source_span(source, "beta");
        assert_eq!(result, Some((6, 10)));
    }

    #[test]
    fn find_unique_scene_source_span_returns_none_for_ambiguous() {
        let source = "abc abc abc";
        let result = find_unique_scene_source_span(source, "abc");
        assert_eq!(result, None);
    }

    #[test]
    fn sha256_hex_produces_stable_lowercase_digest() {
        // sha256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn dedicated_delimiter_survives_a_markdown_hr_in_a_scene() {
        let scene_a = "First scene.\n\n---\n\nA thematic break inside the prose.";
        let scene_b = "Second scene.";
        let source = format!("{scene_a}{SOURCE_BRIDGE_SCENE_DELIMITER}{scene_b}");
        let ranges = delimiter_split_ranges(&source, SOURCE_BRIDGE_SCENE_DELIMITER);
        assert_eq!(ranges.len(), 2, "an inner markdown HR must not add a split");
        assert_eq!(&source[ranges[0].0..ranges[0].1], scene_a);
        assert_eq!(&source[ranges[1].0..ranges[1].1], scene_b);
    }

    #[test]
    fn legacy_delimiter_still_splits_old_pushes() {
        let source = "Alpha.\n\n---\n\nBeta.";
        let ranges = delimiter_split_ranges(source, LEGACY_SOURCE_BRIDGE_SCENE_DELIMITER);
        assert_eq!(ranges.len(), 2);
        assert_eq!(&source[ranges[0].0..ranges[0].1], "Alpha.");
        assert_eq!(&source[ranges[1].0..ranges[1].1], "Beta.");
    }
}
