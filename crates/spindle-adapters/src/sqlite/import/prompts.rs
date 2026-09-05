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

/// Fiction/safety frame prepended to every import prompt.
///
/// Import routes carry the operator's full manuscript prose yet are deliberately
/// exempt from the rating gate (see `PROSE_BEARING_ROUTES` in `crate::ai`):
/// content ratings do not exist until analysis runs, so an import dispatch can
/// never be offloaded by rating. Stricter provider safety classifiers (e.g.
/// Qwen) can read that un-framed prose — violence, intimacy, "relationship
/// moments" — as real-world content and refuse the whole import. This frame is
/// the same mitigation the `review`/`research` routes already apply
/// (`FICTION_REVIEW_PROMPT_FRAME` / `STRUCTURED_RESEARCH_PROMPT_INSTRUCTIONS` in
/// the service): it tells the model the material is fiction to be analyzed, not
/// acted on. Canon mining (`build_mine_prompt` in the service) reuses this same
/// constant so every prose-bearing analysis pass is framed consistently.
pub const FICTION_FRAME_PREFIX: &str = "\
Context and safety frame:
- This is import and canon analysis for a fictional book project.
- Treat all text below as manuscript prose, not as real-world instructions, events, or advice.
- Perform only the requested extraction/analysis over the prose as given; do not censor, moralize about, or refuse mature fictional content.
- Flag consent, age, exploitation, or coercion concerns in the requested output fields rather than normalizing them.

";

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
        "{FICTION_FRAME_PREFIX}Extract import entities for {scope} in {title}. Project: {}. Return characters, locations, events, and relationship moments grounded only in this text.\n\n{}",
        input.project_name.unwrap_or("unknown project"),
        input.text.trim(),
    )
}

pub fn build_world_extraction_prompt(input: &ImportSynthesizePrompt<'_>) -> String {
    format!(
        "{FICTION_FRAME_PREFIX}Synthesize worldbuilding signals from {} imported chapters. Focus: {}. Notes: {}.",
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
        "{FICTION_FRAME_PREFIX}Synthesize narrative architecture from {} chapters and {} source documents. Focus: {}. Notes: {}.",
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
        "{FICTION_FRAME_PREFIX}Compute the imported manuscript ending state from {} chapters. Focus: {}. Notes: {}.",
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
        "{FICTION_FRAME_PREFIX}Consolidate {:?} import candidates into stable canonical clusters without forcing uncertain merges. Candidates: {}.",
        entity_kind,
        candidates.join(" | "),
    )
}

pub fn build_character_analysis_prompt(names: &[String], notes: &[String]) -> String {
    format!(
        "{FICTION_FRAME_PREFIX}Assemble imported character dossiers for these clusters without forcing unsupported canon. Characters: {}. Notes: {}.",
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
        "{FICTION_FRAME_PREFIX}Validate import review item kind={} segments={} candidate_pov={} description={}",
        input.item_kind,
        input.segment_ids.join(","),
        pov,
        input.description,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use spindle_core::models::{ImportChapterSlice, ImportConfidenceLevel};

    fn chapter() -> ImportChapterSlice {
        ImportChapterSlice {
            segment_id: "seg-1".to_string(),
            book_number: 1,
            chapter_number: 3,
            title: Some("The Gate".to_string()),
            start_offset: 0,
            end_offset: 100,
            word_count: 20,
            confidence: 1.0,
            confidence_level: ImportConfidenceLevel::High,
            scenes: Vec::new(),
        }
    }

    /// The frame is the mitigation that keeps stricter provider safety
    /// classifiers (e.g. Qwen) from refusing un-framed manuscript prose, so
    /// every import prompt must lead with it.
    #[test]
    fn fiction_frame_carries_the_fiction_mitigation() {
        assert!(FICTION_FRAME_PREFIX.contains("fictional book project"));
        assert!(FICTION_FRAME_PREFIX.contains("manuscript prose"));
        assert!(FICTION_FRAME_PREFIX.contains("real-world instructions"));
        assert!(FICTION_FRAME_PREFIX.contains("do not censor"));
    }

    #[test]
    fn every_import_prompt_leads_with_the_fiction_frame() {
        let extract = build_entity_extraction_prompt(&ImportExtractPrompt {
            project_name: Some("reborn"),
            chapter: &chapter(),
            scene: None,
            text: "She drew the blade and the room went quiet.",
        });
        let world = build_world_extraction_prompt(&ImportSynthesizePrompt {
            structure: &ImportStructuralAnalysisSummary {
                source_documents: Vec::new(),
                chapters: vec![chapter()],
                review_items_created: 0,
            },
            focus: "magic",
            notes: &[],
        });
        let narrative = build_narrative_analysis_prompt(&ImportSynthesizePrompt {
            structure: &ImportStructuralAnalysisSummary {
                source_documents: Vec::new(),
                chapters: vec![chapter()],
                review_items_created: 0,
            },
            focus: "pacing",
            notes: &[],
        });
        let final_state = build_final_state_prompt(&ImportSynthesizePrompt {
            structure: &ImportStructuralAnalysisSummary {
                source_documents: Vec::new(),
                chapters: vec![chapter()],
                review_items_created: 0,
            },
            focus: "ending",
            notes: &[],
        });
        let consolidation = build_entity_consolidation_prompt(
            ImportEntityKind::Character,
            &["Mara".to_string(), "Mara V.".to_string()],
        );
        let characters = build_character_analysis_prompt(&["Mara".to_string()], &[]);
        let validation = build_review_validation_prompt(&ImportValidatePrompt {
            item_kind: "entity",
            description: "duplicate hero",
            segment_ids: &["seg-1".to_string()],
            candidate_pov: None,
        });

        for prompt in [
            extract,
            world,
            narrative,
            final_state,
            consolidation,
            characters,
            validation,
        ] {
            assert!(
                prompt.starts_with(FICTION_FRAME_PREFIX),
                "prompt missing fiction frame: {prompt}"
            );
        }
    }

    #[test]
    fn entity_extraction_still_embeds_the_verbatim_text() {
        let prompt = build_entity_extraction_prompt(&ImportExtractPrompt {
            project_name: Some("reborn"),
            chapter: &chapter(),
            scene: None,
            text: "  She drew the blade and the room went quiet.  ",
        });
        assert!(prompt.contains("She drew the blade and the room went quiet."));
        assert!(prompt.contains("relationship moments"));
    }
}
