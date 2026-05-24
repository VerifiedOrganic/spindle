//! Manuscript slicer + ingest pipeline.
//!
//! Ports the SurrealDB-era `crate::import::*` source-document ingest path
//! to the SQLite backend. This module is pure logic: it reads source files
//! off disk, normalizes them, detects chapter/scene boundaries, and emits
//! `IngestedSourceDocument` + `AnalyzedChapter` values. No database hits.
//!
//! Service-level code calls `ingest_sources` followed by `analyze_structure`
//! and then persists the result via `Repository::upsert_import_*` methods.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Cursor, Read, Seek};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use roxmltree::Document;
use sha2::{Digest, Sha256};
use spindle_core::models::{
    ImportConfidenceLevel, ImportDuplicateStrategy, ImportPovGuess, ImportPovGuessSource,
    ImportSourceDocumentSummary, ImportSourceFormat, ImportStructuralAnalysisSummary,
};
use zip::ZipArchive;

#[derive(Debug, Clone)]
pub struct IngestedSourceDocument {
    pub display_name: String,
    pub source_path: PathBuf,
    pub copied_path: PathBuf,
    pub normalized_text_path: PathBuf,
    pub source_format: ImportSourceFormat,
    pub original_sha256: String,
    pub normalized_sha256: String,
    pub normalized_text: String,
    /// Per-byte map from each normalized-text byte index back to the
    /// byte index in the ORIGINAL source file (pre-normalization).
    /// Length is `normalized_text.len() + 1`; the trailing entry points
    /// just past the last consumed source byte. Built by
    /// [`normalize_text`].
    pub normalized_to_source_offsets: Vec<usize>,
    pub word_count: usize,
    pub chapter_hint: Option<String>,
    pub source_order: usize,
}

#[derive(Debug, Clone)]
pub struct IngestSourcesOptions<'a> {
    pub data_dir: &'a Path,
    pub existing_source_hashes: &'a BTreeSet<String>,
    pub duplicate_strategy: ImportDuplicateStrategy,
    pub source_format_hint: Option<ImportSourceFormat>,
}

impl<'a> IngestSourcesOptions<'a> {
    pub fn new(data_dir: &'a Path) -> Self {
        static EMPTY: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
        Self {
            data_dir,
            existing_source_hashes: EMPTY.get_or_init(BTreeSet::new),
            duplicate_strategy: ImportDuplicateStrategy::Reject,
            source_format_hint: None,
        }
    }
}

#[derive(Debug, Clone)]
struct ExtractedText {
    text: String,
    chapter_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StructuralAnalysisResult {
    pub source_documents: Vec<IngestedSourceDocument>,
    pub chapters: Vec<AnalyzedChapter>,
}

#[derive(Debug, Clone)]
pub struct AnalyzedChapter {
    pub source_document_index: usize,
    pub book_number: i32,
    pub chapter_number: i32,
    pub title: Option<String>,
    pub start_offset: usize,
    pub end_offset: usize,
    pub word_count: usize,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
    pub review_reason: Option<String>,
    pub scenes: Vec<AnalyzedScene>,
}

#[derive(Debug, Clone)]
pub struct AnalyzedScene {
    pub scene_index: usize,
    pub label: Option<String>,
    pub start_offset: usize,
    pub end_offset: usize,
    /// Byte range in the ORIGINAL source file (pre-normalization)
    /// covering the same scene as `start_offset..end_offset` in the
    /// normalized text. Populated via the
    /// `IngestedSourceDocument::normalized_to_source_offsets` map.
    /// Consumers that need to slice the literal on-disk file (e.g.,
    /// `SourceBridge::pull_chapter_from_file` populating
    /// `scene_source_link.source_start_offset` /
    /// `source_end_offset`) read this instead of round-tripping
    /// through a separate offset translator.
    pub source_byte_range: (usize, usize),
    pub word_count: usize,
    pub character_count: usize,
    pub pov_guess: Option<ImportPovGuess>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
    pub review_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct ChapterBoundary {
    start_offset: usize,
    content_start_offset: usize,
    title: Option<String>,
    explicit_number: Option<i32>,
    confidence: f64,
    review_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct SceneBoundary {
    start_offset: usize,
    label: Option<String>,
    confidence: f64,
    review_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct LineSpan<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

pub fn ingest_sources(
    source_paths: &[PathBuf],
    options: IngestSourcesOptions<'_>,
) -> anyhow::Result<Vec<IngestedSourceDocument>> {
    let mut seen_hashes = BTreeSet::new();
    let mut documents = Vec::with_capacity(source_paths.len());

    for (source_order, source_path) in source_paths.iter().enumerate() {
        let source_format = detect_source_format(source_path, options.source_format_hint.clone())?;
        let bytes = fs::read(source_path)
            .with_context(|| format!("failed to read source file {}", source_path.display()))?;
        let original_sha256 = sha256_hex(&bytes);

        if matches!(options.duplicate_strategy, ImportDuplicateStrategy::Reject)
            && (options.existing_source_hashes.contains(&original_sha256)
                || !seen_hashes.insert(original_sha256.clone()))
        {
            bail!(
                "duplicate import source detected for {}",
                source_path.display()
            );
        }

        let extracted = extract_source_text(&bytes, &source_format)
            .with_context(|| format!("failed to ingest {}", source_path.display()))?;
        // For binary/structured formats (.docx, .epub, .html), the
        // extracted plaintext doesn't correspond byte-for-byte to the
        // original file bytes, so the source-offset map references the
        // extracted intermediate. For txt/md the extracted text IS the
        // source bytes verbatim, so the map points into the file we
        // just read.
        let (normalized_text, normalized_to_source_offsets) = normalize_text(&extracted.text);
        let normalized_sha256 = sha256_hex(normalized_text.as_bytes());
        let copied_path =
            register_source_file(options.data_dir, source_path, &original_sha256, &bytes)?;
        let normalized_text_path =
            register_normalized_text(options.data_dir, &normalized_sha256, &normalized_text)?;

        documents.push(IngestedSourceDocument {
            display_name: display_name(source_path),
            source_path: source_path.clone(),
            copied_path,
            normalized_text_path,
            source_format,
            original_sha256,
            normalized_sha256,
            word_count: normalized_text.split_whitespace().count(),
            chapter_hint: extracted
                .chapter_hint
                .filter(|hint| !hint.trim().is_empty())
                .or_else(|| fallback_chapter_hint(source_path)),
            normalized_text,
            normalized_to_source_offsets,
            source_order,
        });
    }

    Ok(documents)
}

pub fn detect_source_format(
    source_path: &Path,
    hint: Option<ImportSourceFormat>,
) -> anyhow::Result<ImportSourceFormat> {
    if let Some(hint) = hint {
        return Ok(hint);
    }

    let extension = source_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .context("missing source file extension")?;

    match extension.as_str() {
        "txt" => Ok(ImportSourceFormat::Txt),
        "md" | "markdown" => Ok(ImportSourceFormat::Md),
        "html" | "htm" => Ok(ImportSourceFormat::Html),
        "epub" => Ok(ImportSourceFormat::Epub),
        "docx" => Ok(ImportSourceFormat::Docx),
        _ => bail!(
            "unsupported import source format: {}",
            source_path.display()
        ),
    }
}

/// Normalize an input source text and emit a per-byte map from each
/// normalized byte index back to the byte index in the ORIGINAL source.
///
/// Normalization steps (in order):
/// 1. Strip a leading UTF-8 BOM (`\u{feff}`).
/// 2. Collapse `\r\n` and bare `\r` to `\n`.
/// 3. Replace non-breaking spaces (`\u{00a0}`) with regular spaces.
/// 4. Trim trailing `[ \t]` from each line.
/// 5. Collapse runs of blank lines to a single blank line.
/// 6. Strip leading + trailing blank lines.
/// 7. Append a single trailing `\n` if the result is non-empty.
///
/// The returned `Vec<usize>` has length `normalized.len() + 1`; index
/// `i` holds the source-byte offset that produced normalized byte `i`,
/// and index `normalized.len()` points just past the last consumed
/// source byte. Downstream callers use this map to translate
/// normalized-text scene boundaries (what `analyze_structure` emits)
/// back to original-source byte ranges — which is what
/// `SourceBridge::pull_chapter_from_file` needs when persisting
/// `scene_source_link` rows for externally-formatted manuscripts.
pub fn normalize_text(input: &str) -> (String, Vec<usize>) {
    let mut lines = Vec::<NormalizedLineMapping>::new();
    let source_bytes = input.as_bytes();
    let bom_len = usize::from(input.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
    let mut cursor = bom_len;
    let mut line_start = bom_len;
    let mut line_text = String::new();
    let mut line_offsets = vec![line_start];

    while cursor < source_bytes.len() {
        let ch = input[cursor..]
            .chars()
            .next()
            .expect("valid utf-8 boundary");
        let char_len = ch.len_utf8();
        if ch == '\r' {
            let newline_end = if input[cursor..].starts_with("\r\n") {
                cursor + 2
            } else {
                cursor + 1
            };
            push_normalized_line(
                &mut lines,
                &line_text,
                &line_offsets,
                Some(newline_end),
                line_start,
            );
            cursor = newline_end;
            line_start = cursor;
            line_text.clear();
            line_offsets.clear();
            line_offsets.push(line_start);
            continue;
        }
        if ch == '\n' {
            let newline_end = cursor + 1;
            push_normalized_line(
                &mut lines,
                &line_text,
                &line_offsets,
                Some(newline_end),
                line_start,
            );
            cursor = newline_end;
            line_start = cursor;
            line_text.clear();
            line_offsets.clear();
            line_offsets.push(line_start);
            continue;
        }

        let normalized_ch = if ch == '\u{00a0}' { ' ' } else { ch };
        line_text.push(normalized_ch);
        let normalized_len = normalized_ch.len_utf8();
        for byte_idx in 1..=normalized_len {
            line_offsets.push(if byte_idx == normalized_len {
                cursor + char_len
            } else {
                cursor
            });
        }
        cursor += char_len;
    }
    push_normalized_line(&mut lines, &line_text, &line_offsets, None, line_start);

    let mut collapsed_lines = Vec::<NormalizedLineMapping>::new();
    let mut previous_blank = true;
    for line in lines {
        let is_blank = line.text.trim().is_empty();
        if is_blank {
            if !previous_blank {
                collapsed_lines.push(line);
            }
            previous_blank = true;
        } else {
            collapsed_lines.push(line);
            previous_blank = false;
        }
    }
    while collapsed_lines
        .first()
        .is_some_and(|line| line.text.is_empty())
    {
        collapsed_lines.remove(0);
    }
    while collapsed_lines
        .last()
        .is_some_and(|line| line.text.is_empty())
    {
        collapsed_lines.pop();
    }

    if collapsed_lines.is_empty() {
        return (String::new(), vec![input.len()]);
    }

    let mut normalized = String::new();
    let mut source_offsets = vec![collapsed_lines[0].source_offsets[0]];
    for (index, line) in collapsed_lines.iter().enumerate() {
        normalized.push_str(&line.text);
        source_offsets.extend_from_slice(&line.source_offsets[1..]);
        let newline_end = line
            .newline_end_offset
            .unwrap_or_else(|| *line.source_offsets.last().unwrap_or(&input.len()));
        if index + 1 < collapsed_lines.len() {
            normalized.push('\n');
            source_offsets.push(newline_end);
        }
    }
    normalized.push('\n');
    source_offsets.push(
        collapsed_lines
            .last()
            .and_then(|line| line.newline_end_offset)
            .unwrap_or(input.len()),
    );
    (normalized, source_offsets)
}

/// Bookkeeping for `normalize_text`: each entry holds a normalized line
/// plus the per-byte map back to the original source.
#[derive(Debug, Clone)]
struct NormalizedLineMapping {
    text: String,
    source_offsets: Vec<usize>,
    newline_end_offset: Option<usize>,
}

fn push_normalized_line(
    lines: &mut Vec<NormalizedLineMapping>,
    line_text: &str,
    line_offsets: &[usize],
    newline_end_offset: Option<usize>,
    line_start: usize,
) {
    let trimmed_len = line_text.trim_end_matches([' ', '\t']).len();
    let mut offsets = line_offsets
        .get(..=trimmed_len)
        .map(|slice| slice.to_vec())
        .unwrap_or_else(|| vec![line_start]);
    if offsets.is_empty() {
        offsets.push(line_start);
    }
    lines.push(NormalizedLineMapping {
        text: line_text[..trimmed_len].to_string(),
        source_offsets: offsets,
        newline_end_offset,
    });
}

pub fn analyze_structure(
    source_documents: Vec<IngestedSourceDocument>,
) -> StructuralAnalysisResult {
    let mut chapters = Vec::new();
    let mut next_chapter_number = 1i32;

    for (source_document_index, document) in source_documents.iter().enumerate() {
        let boundaries = detect_chapter_boundaries(document, next_chapter_number);
        for (boundary_index, boundary) in boundaries.iter().enumerate() {
            let end_offset = boundaries
                .get(boundary_index + 1)
                .map(|next| next.start_offset)
                .unwrap_or_else(|| document.normalized_text.len());
            let chapter_number = boundary.explicit_number.unwrap_or(next_chapter_number);
            next_chapter_number = chapter_number + 1;

            let scenes = detect_scene_slices(document, boundary, end_offset);
            let content_start = boundary.content_start_offset.min(end_offset);
            let chapter_text = document
                .normalized_text
                .get(content_start..end_offset)
                .unwrap_or_default();

            chapters.push(AnalyzedChapter {
                source_document_index,
                book_number: 1,
                chapter_number,
                title: boundary.title.clone(),
                start_offset: boundary.start_offset,
                end_offset,
                word_count: chapter_text.split_whitespace().count(),
                confidence: boundary.confidence,
                confidence_level: confidence_level(boundary.confidence),
                review_reason: boundary.review_reason.clone(),
                scenes,
            });
        }
    }

    StructuralAnalysisResult {
        source_documents,
        chapters,
    }
}

pub fn structural_summary(analysis: &StructuralAnalysisResult) -> ImportStructuralAnalysisSummary {
    ImportStructuralAnalysisSummary {
        source_documents: analysis
            .source_documents
            .iter()
            .map(source_document_summary)
            .collect(),
        chapters: Vec::new(),
        review_items_created: analysis.review_item_count(),
    }
}

impl StructuralAnalysisResult {
    pub fn total_segments(&self) -> usize {
        self.chapters.len()
            + self
                .chapters
                .iter()
                .map(|chapter| chapter.scenes.len())
                .sum::<usize>()
    }

    pub fn review_item_count(&self) -> usize {
        self.chapters
            .iter()
            .map(|chapter| {
                usize::from(chapter.review_reason.is_some())
                    + chapter
                        .scenes
                        .iter()
                        .filter(|scene| scene.review_reason.is_some())
                        .count()
            })
            .sum()
    }
}

fn source_document_summary(document: &IngestedSourceDocument) -> ImportSourceDocumentSummary {
    ImportSourceDocumentSummary {
        document_id: String::new(),
        display_name: document.display_name.clone(),
        source_path: document.source_path.display().to_string(),
        copied_path: document.copied_path.display().to_string(),
        source_format: document.source_format.clone(),
        original_sha256: document.original_sha256.clone(),
        normalized_sha256: document.normalized_sha256.clone(),
        word_count: document.word_count,
        chapter_hint: document.chapter_hint.clone(),
        source_order: document.source_order,
    }
}

fn detect_chapter_boundaries(
    document: &IngestedSourceDocument,
    fallback_chapter_number: i32,
) -> Vec<ChapterBoundary> {
    let lines = line_spans(&document.normalized_text);
    let mut boundaries = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.text.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some((title, explicit_number, confidence, review_reason)) =
            chapter_heading_details(trimmed, index, &lines)
        {
            boundaries.push(ChapterBoundary {
                start_offset: line.start,
                content_start_offset: line.end,
                title,
                explicit_number,
                confidence,
                review_reason,
            });
            continue;
        }

        if is_separator_line(trimmed)
            && trimmed.len() >= 5
            && looks_like_chapter_separator(index, &lines)
            && !boundaries.is_empty()
        {
            boundaries.push(ChapterBoundary {
                start_offset: line.start,
                content_start_offset: line.end,
                title: None,
                explicit_number: None,
                confidence: 0.42,
                review_reason: Some(
                    "chapter boundary inferred from separator-only transition".to_string(),
                ),
            });
        }
    }

    if boundaries.is_empty() {
        boundaries.push(ChapterBoundary {
            start_offset: 0,
            content_start_offset: 0,
            title: document.chapter_hint.clone(),
            explicit_number: Some(fallback_chapter_number),
            confidence: if document.chapter_hint.is_some() {
                0.62
            } else {
                0.34
            },
            review_reason: Some(
                "chapter boundary was inferred because the source had no explicit chapter marker"
                    .to_string(),
            ),
        });
    }

    boundaries
}

fn detect_scene_slices(
    document: &IngestedSourceDocument,
    boundary: &ChapterBoundary,
    chapter_end_offset: usize,
) -> Vec<AnalyzedScene> {
    let start_offset = boundary.content_start_offset.min(chapter_end_offset);
    let chapter_text = document
        .normalized_text
        .get(start_offset..chapter_end_offset)
        .unwrap_or_default();
    let lines = line_spans(chapter_text);
    let mut boundaries = vec![SceneBoundary {
        start_offset,
        label: None,
        confidence: 0.91,
        review_reason: None,
    }];

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.text.trim();
        if !is_separator_line(trimmed) && !looks_like_blankline_scene_break(index, &lines) {
            continue;
        }

        let next_content = next_nonempty_line(index + 1, &lines);
        let Some(next_line) = next_content else {
            continue;
        };
        let absolute_start = start_offset + next_line.start;
        if absolute_start <= boundaries.last().map(|item| item.start_offset).unwrap_or(0) {
            continue;
        }

        let confidence = if is_separator_line(trimmed) {
            0.9
        } else {
            0.58
        };
        let review_reason = (confidence < 0.65).then(|| {
            "scene boundary was inferred from a blank-line transition and should be reviewed"
                .to_string()
        });
        boundaries.push(SceneBoundary {
            start_offset: absolute_start,
            label: scene_label(next_line.text.trim()),
            confidence,
            review_reason,
        });
    }

    let mut scenes = Vec::new();
    for (scene_index, scene_boundary) in boundaries.iter().enumerate() {
        let end_offset = boundaries
            .get(scene_index + 1)
            .map(|next| next.start_offset)
            .unwrap_or(chapter_end_offset);
        let raw = document
            .normalized_text
            .get(scene_boundary.start_offset..end_offset)
            .unwrap_or_default();
        let scene_text = raw.trim();
        if scene_text.is_empty() {
            continue;
        }

        // Tighten the normalized-text range to cover only the body —
        // strip leading whitespace, strip trailing whitespace, and
        // strip any trailing separator lines (`***` / `---`) the
        // boundary loop carried in from the next scene's break.
        // `source_byte_range_for` then reports byte-exact body spans
        // in the original source file.
        let trim_start_rel = raw.len() - raw.trim_start().len();
        let mut body_end_rel = trim_start_rel + raw[trim_start_rel..].trim_end().len();
        loop {
            let body = &raw[trim_start_rel..body_end_rel];
            let last_line_start = body.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let last_line = body[last_line_start..].trim();
            if last_line.is_empty() || is_separator_line(last_line) {
                let cut_to = trim_start_rel + last_line_start;
                if cut_to <= trim_start_rel {
                    body_end_rel = trim_start_rel;
                    break;
                }
                body_end_rel = trim_start_rel + raw[trim_start_rel..cut_to].trim_end().len();
                if body_end_rel <= trim_start_rel {
                    body_end_rel = trim_start_rel;
                    break;
                }
            } else {
                break;
            }
        }
        let body_start_offset = scene_boundary.start_offset + trim_start_rel;
        let body_end_offset = scene_boundary.start_offset + body_end_rel;

        let pov_guess = infer_pov(scene_text);
        let pov_confidence = pov_guess
            .as_ref()
            .map(|guess| guess.confidence)
            .unwrap_or(0.28);
        let review_reason = scene_boundary.review_reason.clone().or_else(|| {
            (pov_guess.is_none() || pov_confidence < 0.65).then(|| {
                "POV assignment was low-confidence and should be reviewed before later passes"
                    .to_string()
            })
        });
        let confidence = scene_boundary
            .confidence
            .min((pov_confidence + scene_boundary.confidence) / 2.0 + 0.15);

        let source_byte_range = source_byte_range_for(document, body_start_offset, body_end_offset);
        scenes.push(AnalyzedScene {
            scene_index: scenes.len() + 1,
            label: scene_boundary.label.clone(),
            start_offset: scene_boundary.start_offset,
            end_offset,
            source_byte_range,
            word_count: scene_text.split_whitespace().count(),
            character_count: scene_text.chars().count(),
            pov_guess,
            confidence,
            confidence_level: confidence_level(confidence),
            review_reason,
        });
    }

    if scenes.is_empty() {
        let scene_text = chapter_text.trim();
        if !scene_text.is_empty() {
            let pov_guess = infer_pov(scene_text);
            let pov_confidence = pov_guess
                .as_ref()
                .map(|guess| guess.confidence)
                .unwrap_or(0.28);
            // Same body-tightening logic as the per-scene loop above.
            let trim_start_rel = chapter_text.len() - chapter_text.trim_start().len();
            let trim_end_rel = trim_start_rel + scene_text.len();
            let body_start_offset = start_offset + trim_start_rel;
            let body_end_offset = start_offset + trim_end_rel;
            let source_byte_range =
                source_byte_range_for(document, body_start_offset, body_end_offset);
            scenes.push(AnalyzedScene {
                scene_index: 1,
                label: None,
                start_offset,
                end_offset: chapter_end_offset,
                source_byte_range,
                word_count: scene_text.split_whitespace().count(),
                character_count: scene_text.chars().count(),
                pov_guess,
                confidence: 0.61,
                confidence_level: ImportConfidenceLevel::Medium,
                review_reason: (pov_confidence < 0.65).then(|| {
                    "chapter was treated as a single scene because no reliable scene break was found"
                        .to_string()
                }),
            });
        }
    }

    scenes
}

/// Translate a (start, end) byte range in the document's normalized
/// text into the corresponding byte range in the ORIGINAL source file
/// via `IngestedSourceDocument::normalized_to_source_offsets`. Clamps
/// to the map's bounds — the trailing past-the-end entry covers the
/// `end == normalized_text.len()` case naturally.
fn source_byte_range_for(
    document: &IngestedSourceDocument,
    normalized_start: usize,
    normalized_end: usize,
) -> (usize, usize) {
    let map = &document.normalized_to_source_offsets;
    let last = map.last().copied().unwrap_or(0);
    let start = map.get(normalized_start).copied().unwrap_or(last);
    let end = map.get(normalized_end).copied().unwrap_or(last);
    (start, end)
}

fn line_spans(input: &str) -> Vec<LineSpan<'_>> {
    let mut lines = Vec::new();
    let mut offset = 0usize;
    for slice in input.split_inclusive('\n') {
        let text = slice.strip_suffix('\n').unwrap_or(slice);
        let start = offset;
        offset += slice.len();
        lines.push(LineSpan {
            text,
            start,
            end: offset,
        });
    }

    if !input.is_empty()
        && !input.ends_with('\n')
        && let Some(last) = lines.last_mut()
    {
        last.end = input.len();
    }

    lines
}

#[allow(clippy::type_complexity)]
fn chapter_heading_details(
    line: &str,
    index: usize,
    lines: &[LineSpan<'_>],
) -> Option<(Option<String>, Option<i32>, f64, Option<String>)> {
    // Markdown-friendly: peel any leading `#` characters + whitespace
    // off the line so `# Chapter 1`, `## Chapter 1`, etc. parse as
    // chapter headings. The original `line` is preserved as-is on the
    // input side; the returned title is the de-marked text so book/
    // chapter rows don't carry markdown hash prefixes.
    let stripped = line.trim_start_matches('#').trim_start();
    let title = if stripped.is_empty() {
        line.to_string()
    } else {
        stripped.to_string()
    };
    let lowercase = stripped.to_ascii_lowercase();

    if let Some(explicit_number) = parse_numbered_chapter_heading(&lowercase) {
        return Some((Some(title), Some(explicit_number), 0.97, None));
    }

    if matches!(lowercase.as_str(), "prologue" | "epilogue") {
        return Some((
            Some(title),
            None,
            0.82,
            Some("named chapter heading did not include an explicit chapter number".to_string()),
        ));
    }

    if looks_like_named_chapter_heading(stripped, index, lines) {
        return Some((
            Some(title),
            None,
            0.56,
            Some("named chapter heading did not include an explicit chapter number".to_string()),
        ));
    }

    None
}

fn parse_numbered_chapter_heading(lowercase: &str) -> Option<i32> {
    let suffix = lowercase.strip_prefix("chapter ")?.trim();
    parse_heading_number(suffix)
}

fn parse_heading_number(raw: &str) -> Option<i32> {
    if let Ok(value) = raw.parse::<i32>() {
        return Some(value);
    }
    roman_to_int(raw).or_else(|| small_number_word(raw))
}

fn roman_to_int(raw: &str) -> Option<i32> {
    let mut total = 0i32;
    let mut prev = 0i32;
    let mut saw = false;
    for ch in raw.chars().rev() {
        let value = match ch.to_ascii_uppercase() {
            'I' => 1,
            'V' => 5,
            'X' => 10,
            'L' => 50,
            'C' => 100,
            'D' => 500,
            'M' => 1000,
            _ => return None,
        };
        saw = true;
        if value < prev {
            total -= value;
        } else {
            total += value;
            prev = value;
        }
    }
    saw.then_some(total).filter(|value| *value > 0)
}

fn small_number_word(raw: &str) -> Option<i32> {
    match raw.trim() {
        "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        "eleven" => Some(11),
        "twelve" => Some(12),
        _ => None,
    }
}

fn looks_like_named_chapter_heading(line: &str, index: usize, lines: &[LineSpan<'_>]) -> bool {
    let trimmed = line.trim();
    if trimmed.len() > 48 || trimmed.split_whitespace().count() > 6 {
        return false;
    }
    let has_blank_before = index == 0 || lines[index.saturating_sub(1)].text.trim().is_empty();
    let has_blank_after = lines
        .get(index + 1)
        .is_some_and(|line| line.text.trim().is_empty());
    let title_like = trimmed.split_whitespace().all(|word| {
        word.chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
    });

    has_blank_before && has_blank_after && title_like
}

fn looks_like_chapter_separator(index: usize, lines: &[LineSpan<'_>]) -> bool {
    let previous_words = previous_nonempty_line(index, lines)
        .map(|line| line.text.split_whitespace().count())
        .unwrap_or(0);
    let next_words = next_nonempty_line(index + 1, lines)
        .map(|line| line.text.split_whitespace().count())
        .unwrap_or(0);
    previous_words >= 8 && next_words >= 3
}

fn looks_like_blankline_scene_break(index: usize, lines: &[LineSpan<'_>]) -> bool {
    let current = lines[index].text.trim();
    if !current.is_empty() {
        return false;
    }

    let Some(previous) = previous_nonempty_line(index, lines) else {
        return false;
    };
    let Some(next) = next_nonempty_line(index + 1, lines) else {
        return false;
    };

    let previous_text = previous.text.trim();
    let next_text = next.text.trim();
    if is_separator_line(next_text) {
        return false;
    }
    if looks_like_named_chapter_heading(next_text, index + 1, lines)
        || next_text.to_ascii_lowercase().starts_with("chapter ")
        || matches!(
            next_text.to_ascii_lowercase().as_str(),
            "prologue" | "epilogue"
        )
    {
        return false;
    }
    previous_text.split_whitespace().count() >= 8
        && next_text.split_whitespace().count() <= 6
        && (next_text.chars().all(|ch| !ch.is_ascii_lowercase())
            || next_text.split_whitespace().all(|word| {
                word.chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_uppercase())
            }))
}

fn infer_pov(scene_text: &str) -> Option<ImportPovGuess> {
    let normalized = scene_text.to_ascii_lowercase();
    let first_person_hits = [" i ", " i'm ", " i'd ", " i've ", " me ", " my ", " mine "]
        .iter()
        .map(|needle| normalized.matches(needle).count())
        .sum::<usize>();
    if first_person_hits >= 4 {
        return Some(ImportPovGuess {
            character_name: Some("First-person narrator".to_string()),
            cluster_id: None,
            confidence: 0.72,
            confidence_level: ImportConfidenceLevel::Medium,
            source: ImportPovGuessSource::Heuristic,
            rationale: Some("dense first-person pronoun usage dominated the scene".to_string()),
        });
    }

    let name_counts = candidate_name_counts(scene_text);
    let mut ranked = name_counts.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let (name, count) = ranked.first()?.clone();
    let next_count = ranked.get(1).map(|(_, count)| *count).unwrap_or(0);
    let confidence = if count >= 4 && count >= next_count + 2 {
        0.88
    } else if count >= 2 && count > next_count {
        0.68
    } else {
        0.51
    };

    Some(ImportPovGuess {
        character_name: Some(name),
        cluster_id: None,
        confidence,
        confidence_level: confidence_level(confidence),
        source: ImportPovGuessSource::Heuristic,
        rationale: Some("the scene repeatedly centered one named character".to_string()),
    })
}

fn candidate_name_counts(scene_text: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for token in scene_text.split(|ch: char| !ch.is_ascii_alphabetic() && ch != '\'') {
        if token.len() < 2 {
            continue;
        }
        if !token
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
        {
            continue;
        }
        if token
            .chars()
            .skip(1)
            .any(|ch| !ch.is_ascii_lowercase() && ch != '\'')
        {
            continue;
        }
        if matches!(
            token,
            "The"
                | "A"
                | "An"
                | "Chapter"
                | "Prologue"
                | "Epilogue"
                | "He"
                | "She"
                | "They"
                | "His"
                | "Her"
                | "Their"
                | "I"
        ) {
            continue;
        }
        *counts.entry(token.to_string()).or_insert(0) += 1;
    }
    counts
}

fn confidence_level(confidence: f64) -> ImportConfidenceLevel {
    if confidence >= 0.8 {
        ImportConfidenceLevel::High
    } else if confidence >= 0.55 {
        ImportConfidenceLevel::Medium
    } else {
        ImportConfidenceLevel::Low
    }
}

fn previous_nonempty_line<'a>(index: usize, lines: &'a [LineSpan<'a>]) -> Option<&'a LineSpan<'a>> {
    lines[..index]
        .iter()
        .rev()
        .find(|line| !line.text.trim().is_empty())
}

fn next_nonempty_line<'a>(index: usize, lines: &'a [LineSpan<'a>]) -> Option<&'a LineSpan<'a>> {
    lines[index..]
        .iter()
        .find(|line| !line.text.trim().is_empty())
}

fn scene_label(line: &str) -> Option<String> {
    let trimmed = line.trim();
    (trimmed.split_whitespace().count() <= 6
        && trimmed.chars().any(|ch| ch.is_ascii_alphabetic())
        && trimmed.chars().all(|ch| !ch.is_ascii_lowercase()))
    .then(|| trimmed.to_string())
}

fn is_separator_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty()
        && trimmed.len() >= 3
        && trimmed
            .chars()
            .all(|ch| matches!(ch, '*' | '-' | '_' | '#'))
}

fn extract_source_text(
    bytes: &[u8],
    source_format: &ImportSourceFormat,
) -> anyhow::Result<ExtractedText> {
    match source_format {
        ImportSourceFormat::Txt => extract_txt(bytes),
        ImportSourceFormat::Md => extract_markdown(bytes),
        ImportSourceFormat::Html => extract_html(bytes),
        ImportSourceFormat::Epub => extract_epub(bytes),
        ImportSourceFormat::Docx => extract_docx(bytes),
    }
}

fn extract_txt(bytes: &[u8]) -> anyhow::Result<ExtractedText> {
    let text = decode_utf8(bytes)?;
    let chapter_hint = first_nonempty_line(&text)
        .filter(|line| looks_like_chapter_hint(line))
        .map(ToString::to_string);
    Ok(ExtractedText { text, chapter_hint })
}

/// Markdown extraction is a UTF-8 passthrough. The slicer's
/// `normalize_text` + `chapter_heading_details` already handle the
/// markdown shapes this importer cares about (`# Chapter N`-style
/// headings, `***` / `---` separator lines, blank-line scene
/// transitions). Passing source bytes through verbatim preserves the
/// byte-for-byte correspondence that `SourceBridge::pull_chapter_from_file`
/// requires to populate accurate `scene_source_link` ranges — without
/// it, every stripped `# ` or rewritten link shifted the slicer's
/// offsets a few bytes ahead of the on-disk file.
///
/// Scene `full_text` will preserve the raw markdown formatting
/// (`# Chapter 1`, `**bold**`, `[label](url)`, etc.). Downstream
/// consumers that want stripped prose can run their own markdown
/// renderer over the body; the import side no longer mutates the input.
///
/// `chapter_hint` still gets populated from the first `#`-prefixed
/// non-empty heading line so the import flow's chapter-naming
/// heuristic continues to work.
fn extract_markdown(bytes: &[u8]) -> anyhow::Result<ExtractedText> {
    let text = decode_utf8(bytes)?;
    let chapter_hint = text
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with('#') {
                return None;
            }
            let heading = trimmed.trim_start_matches('#').trim();
            if heading.is_empty() {
                None
            } else {
                Some(heading.to_string())
            }
        })
        .next();
    Ok(ExtractedText { text, chapter_hint })
}

fn extract_html(bytes: &[u8]) -> anyhow::Result<ExtractedText> {
    let html = decode_utf8(bytes)?;
    let chapter_hint = xml_first_tag_text(&html, &["title", "h1", "h2"]);
    let text =
        html2text::from_read(html.as_bytes(), 10_000).context("failed to render html as text")?;
    Ok(ExtractedText { text, chapter_hint })
}

fn extract_epub(bytes: &[u8]) -> anyhow::Result<ExtractedText> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).context("failed to open epub archive")?;
    let container_xml = read_zip_string(&mut archive, "META-INF/container.xml")?;
    let package_path = epub_rootfile_path(&container_xml)?;
    let package_xml = read_zip_string(&mut archive, &package_path)?;
    let package = parse_epub_package(&package_xml)?;
    let base_dir = Path::new(&package_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));

    let mut sections = Vec::new();
    for item_id in package.spine {
        let href = package
            .manifest
            .get(&item_id)
            .with_context(|| format!("missing epub manifest item for spine id {item_id}"))?;
        let content_path = normalize_zip_path(base_dir.join(href));
        let content = read_zip_string(&mut archive, &content_path)
            .with_context(|| format!("failed to read epub spine item {content_path}"))?;
        let extracted = extract_html(content.as_bytes())?;
        if !extracted.text.trim().is_empty() {
            sections.push(extracted.text);
        }
    }

    if sections.is_empty() {
        bail!("epub contained no readable spine text");
    }

    Ok(ExtractedText {
        text: sections.join("\n\n"),
        chapter_hint: package.title,
    })
}

fn extract_docx(bytes: &[u8]) -> anyhow::Result<ExtractedText> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).context("failed to open docx archive")?;
    let document_xml = read_zip_string(&mut archive, "word/document.xml")?;
    let document = Document::parse(&document_xml).context("failed to parse docx document.xml")?;
    let mut paragraphs = Vec::new();

    for paragraph in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "p")
    {
        let mut paragraph_text = String::new();
        collect_docx_paragraph_text(paragraph, &mut paragraph_text);
        let paragraph_text = paragraph_text.trim().to_string();
        if !paragraph_text.is_empty() {
            paragraphs.push(paragraph_text);
        }
    }

    if paragraphs.is_empty() {
        bail!("docx contained no readable document text");
    }

    let chapter_hint = paragraphs
        .first()
        .filter(|line| looks_like_chapter_hint(line))
        .cloned();

    Ok(ExtractedText {
        text: paragraphs.join("\n\n"),
        chapter_hint,
    })
}

fn decode_utf8(bytes: &[u8]) -> anyhow::Result<String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).context("source file is not valid UTF-8")
}

fn read_zip_string<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> anyhow::Result<String> {
    let mut file = archive
        .by_name(name)
        .with_context(|| format!("zip entry not found: {name}"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("failed to read zip entry: {name}"))?;
    decode_utf8(&bytes)
}

fn epub_rootfile_path(container_xml: &str) -> anyhow::Result<String> {
    let document = Document::parse(container_xml).context("failed to parse epub container.xml")?;
    let rootfile = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "rootfile")
        .and_then(|node| node.attribute("full-path"))
        .context("epub container.xml missing rootfile full-path")?;
    Ok(rootfile.to_string())
}

#[derive(Debug)]
struct EpubPackage {
    title: Option<String>,
    manifest: std::collections::BTreeMap<String, String>,
    spine: Vec<String>,
}

fn parse_epub_package(package_xml: &str) -> anyhow::Result<EpubPackage> {
    let document = Document::parse(package_xml).context("failed to parse epub package")?;
    let title = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "title")
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let manifest = document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "item")
        .filter_map(|node| {
            Some((
                node.attribute("id")?.to_string(),
                node.attribute("href")?.to_string(),
            ))
        })
        .collect();
    let spine = document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "itemref")
        .filter_map(|node| node.attribute("idref").map(ToString::to_string))
        .collect::<Vec<_>>();

    if spine.is_empty() {
        bail!("epub package contained no spine entries");
    }

    Ok(EpubPackage {
        title,
        manifest,
        spine,
    })
}

fn collect_docx_paragraph_text(node: roxmltree::Node<'_, '_>, output: &mut String) {
    if !node.is_element() {
        return;
    }

    match node.tag_name().name() {
        "t" => {
            if let Some(text) = node.text() {
                output.push_str(text);
            }
        }
        "tab" => output.push('\t'),
        "br" | "cr" => output.push('\n'),
        _ => {
            for child in node.children() {
                collect_docx_paragraph_text(child, output);
            }
        }
    }
}

fn xml_first_tag_text(input: &str, names: &[&str]) -> Option<String> {
    let document = Document::parse(input).ok()?;
    for name in names {
        if let Some(value) = document
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == *name)
            .and_then(|node| node.text())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }
    None
}

fn register_source_file(
    data_dir: &Path,
    source_path: &Path,
    original_sha256: &str,
    bytes: &[u8],
) -> anyhow::Result<PathBuf> {
    let target_dir = data_dir
        .join("imports")
        .join("sources")
        .join(original_sha256);
    fs::create_dir_all(&target_dir)
        .with_context(|| format!("failed to create import data dir {}", target_dir.display()))?;
    let target_path = target_dir.join(sanitize_filename(&display_name(source_path)));
    if !target_path.exists() {
        fs::write(&target_path, bytes)
            .with_context(|| format!("failed to copy source file to {}", target_path.display()))?;
    }
    Ok(target_path)
}

fn register_normalized_text(
    data_dir: &Path,
    normalized_sha256: &str,
    normalized_text: &str,
) -> anyhow::Result<PathBuf> {
    let target_dir = data_dir.join("imports").join("normalized");
    fs::create_dir_all(&target_dir).with_context(|| {
        format!(
            "failed to create normalized text dir {}",
            target_dir.display()
        )
    })?;
    let target_path = target_dir.join(format!("{normalized_sha256}.txt"));
    if !target_path.exists() {
        fs::write(&target_path, normalized_text).with_context(|| {
            format!("failed to write normalized text {}", target_path.display())
        })?;
    }
    Ok(target_path)
}

fn normalize_zip_path(path: PathBuf) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::RootDir | Component::Prefix(_) => parts.clear(),
        }
    }
    parts.join("/")
}

fn display_name(source_path: &Path) -> String {
    source_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("source")
        .to_string()
}

fn fallback_chapter_hint(source_path: &Path) -> Option<String> {
    source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.replace(['_', '-'], " "))
}

fn looks_like_chapter_hint(line: &str) -> bool {
    let trimmed = line.trim();
    let lowercase = trimmed.to_ascii_lowercase();
    lowercase.starts_with("chapter ")
        || lowercase == "prologue"
        || lowercase == "epilogue"
        || (trimmed.len() <= 48 && trimmed.split_whitespace().count() <= 6)
}

fn first_nonempty_line(input: &str) -> Option<&str> {
    input.lines().map(str::trim).find(|line| !line.is_empty())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn sanitize_filename(input: &str) -> String {
    let sanitized = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "source".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn normalize_text_collapses_blank_lines_and_strips_bom() {
        let raw = "\u{feff}First\r\n\r\n\r\nSecond\n";
        let (normalized, source_offsets) = normalize_text(raw);
        assert_eq!(normalized, "First\n\nSecond\n");
        // Offset map is dense: one entry per normalized byte plus one
        // past-the-end. Mapping the start of "First" lands past the
        // 3-byte BOM (`\u{feff}` = 3 bytes UTF-8).
        assert_eq!(source_offsets.len(), normalized.len() + 1);
        let bom_len = '\u{feff}'.len_utf8();
        assert_eq!(source_offsets[0], bom_len);
        // "First" starts at byte `bom_len` in the source; the F at
        // normalized[0] must map there.
        assert_eq!(&raw[source_offsets[0]..source_offsets[0] + 1], "F");
        // The trailing past-the-end entry must point at-or-past the
        // last source byte.
        assert!(source_offsets[normalized.len()] <= raw.len());
    }

    #[test]
    fn detect_source_format_uses_extension() {
        let path = PathBuf::from("/tmp/chapter.md");
        let fmt = detect_source_format(&path, None).unwrap();
        assert!(matches!(fmt, ImportSourceFormat::Md));
    }

    #[test]
    fn ingest_and_analyze_slice_a_minimal_txt_into_chapters_and_scenes() {
        let tmp = TempDir::new().unwrap();
        let source_path = tmp.path().join("ch1.txt");
        std::fs::write(
            &source_path,
            "Chapter 1\n\nThe night was long. Aaron walked the gate.\n\n* * *\n\nAaron paused.\n",
        )
        .unwrap();

        let docs = ingest_sources(&[source_path], IngestSourcesOptions::new(tmp.path())).unwrap();
        assert_eq!(docs.len(), 1);
        assert!(docs[0].original_sha256.len() == 64);

        let analysis = analyze_structure(docs);
        assert_eq!(analysis.chapters.len(), 1);
        assert_eq!(analysis.chapters[0].chapter_number, 1);
        assert!(!analysis.chapters[0].scenes.is_empty());
    }
}
