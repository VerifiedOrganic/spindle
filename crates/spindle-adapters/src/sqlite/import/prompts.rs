//! Import-pass prompt builders.
//!
//! Pure-string builders for the LLM routes used by the import pipeline.
//! Ported from the SurrealDB-era `crate::import::prompts` with no changes:
//! the prompts depend only on `spindle_core::models::*` types, never on
//! repository or record-id types.

use spindle_core::models::{
    ImportChapterSlice, ImportEntityKind, ImportPovGuess, ImportSceneSlice,
    ImportStructuralAnalysisSummary,
};

pub struct ImportExtractPrompt<'a> {
    pub project_name: Option<&'a str>,
    pub chapter: &'a ImportChapterSlice,
    pub scene: Option<&'a ImportSceneSlice>,
    pub text: &'a str,
}

pub struct ImportSynthesizePrompt<'a> {
    pub structure: &'a ImportStructuralAnalysisSummary,
    pub focus: &'a str,
    pub notes: &'a [String],
}

pub struct ImportValidatePrompt<'a> {
    pub item_kind: &'a str,
    pub description: &'a str,
    pub segment_ids: &'a [String],
    pub candidate_pov: Option<&'a ImportPovGuess>,
}

pub fn build_entity_extraction_prompt(input: &ImportExtractPrompt<'_>) -> String {
    let scope = input
        .scene
        .map(|scene| format!("scene {}", scene.scene_index))
        .unwrap_or_else(|| "chapter summary".to_string());
    let title = input
        .chapter
        .title
        .clone()
        .unwrap_or_else(|| format!("Chapter {}", input.chapter.chapter_number));
    format!(
        "Extract import entities for {scope} in {title}. Project: {}. Return characters, locations, events, and relationship moments grounded only in this text.\n\n{}",
        input.project_name.unwrap_or("unknown project"),
        input.text.trim(),
    )
}

pub fn build_world_extraction_prompt(input: &ImportSynthesizePrompt<'_>) -> String {
    format!(
        "Synthesize worldbuilding signals from {} imported chapters. Focus: {}. Notes: {}.",
        input.structure.chapters.len(),
        input.focus,
        if input.notes.is_empty() {
            "none".to_string()
        } else {
            input.notes.join(" | ")
        }
    )
}

pub fn build_narrative_analysis_prompt(input: &ImportSynthesizePrompt<'_>) -> String {
    format!(
        "Synthesize narrative architecture from {} chapters and {} source documents. Focus: {}. Notes: {}.",
        input.structure.chapters.len(),
        input.structure.source_documents.len(),
        input.focus,
        if input.notes.is_empty() {
            "none".to_string()
        } else {
            input.notes.join(" | ")
        }
    )
}

pub fn build_final_state_prompt(input: &ImportSynthesizePrompt<'_>) -> String {
    format!(
        "Compute the imported manuscript ending state from {} chapters. Focus: {}. Notes: {}.",
        input.structure.chapters.len(),
        input.focus,
        if input.notes.is_empty() {
            "none".to_string()
        } else {
            input.notes.join(" | ")
        }
    )
}

pub fn build_entity_consolidation_prompt(
    entity_kind: ImportEntityKind,
    candidates: &[String],
) -> String {
    format!(
        "Consolidate {:?} import candidates into stable canonical clusters without forcing uncertain merges. Candidates: {}.",
        entity_kind,
        candidates.join(" | "),
    )
}

pub fn build_character_analysis_prompt(names: &[String], notes: &[String]) -> String {
    format!(
        "Assemble imported character dossiers for these clusters without forcing unsupported canon. Characters: {}. Notes: {}.",
        names.join(" | "),
        if notes.is_empty() {
            "none".to_string()
        } else {
            notes.join(" | ")
        }
    )
}

pub fn build_review_validation_prompt(input: &ImportValidatePrompt<'_>) -> String {
    let pov = input
        .candidate_pov
        .and_then(|guess| guess.character_name.clone())
        .unwrap_or_else(|| "none".to_string());
    format!(
        "Validate import review item kind={} segments={} candidate_pov={} description={}",
        input.item_kind,
        input.segment_ids.join(","),
        pov,
        input.description,
    )
}
