//! Chapter advisory critique prompt assembly, parsing, validation, and local
//! critique artifacts. No chapter edit is applied here.

use super::{
    DraftMessages, DraftTokenizer,
    critique::{sha256_hex, strip_thinking_and_fences},
    revision::{RevisionError, fits_budget},
};
use spindle_core::drafting::{
    AuthorDecisionItem, ChapterConcernV1, ChapterCoverageV1, ChapterCritiqueV1,
    ChapterEditProposalV1, ChapterWindowV1, CriticFindingInput, validate_chapter_critique,
};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

pub const CHAPTER_CRITIC_SYSTEM: &str = "\
You are Spindle's chapter developmental editor. Review the supplied chapter as a whole: openings and \
endings, repetition, beat/intensity progression, scene lengths, chapter coherence, and reader \
engagement. Anchor every concern to exact scene UTF-8 bytes when possible; a concern you cannot \
anchor stays advisory. Propose at most two scene edits, each anchored to one scene's exact bytes. \
Never rewrite the chapter or book wholesale, never invent canon, and never override author locks. \
Return ONLY one valid ChapterCritiqueV1 JSON object, without markdown fences or commentary.";

pub const CHAPTER_CRITIC_RUBRIC_VERSION: &str = "chapter-critic-v1";
pub const CHAPTER_CRITIC_RUBRIC: &str = "\
1. Openings and endings: vary entry/exit devices; avoid repeated hooks or summary endings.\n\
2. Repetition: identify repeated images, phrases, gestures, and scene shapes across the chapter.\n\
3. Beat and intensity progression: each scene should change the chapter's pressure; note stalls.\n\
4. Scene lengths: flag clusters of same-length scenes or a scene that starves its turn.\n\
5. Chapter coherence: the chapter should have a legible arc, not just adjacent scenes.\n\
6. Reader engagement: use retained reader/dual-persona notes as evidence, not orders.\n\
7. Coverage: report only what the supplied window/index actually shows; never claim unseen prose.\n\
8. Advisories: an unanchored concern is advisory and must not become an edit proposal.";

/// Per-call chapter-critic reservation (24,000 input / 2,000 output / 512 framing).
#[derive(Debug, Clone)]
pub struct ChapterCriticBudget {
    pub model_id: String,
    pub input_limit: usize,
    pub output_reserve: usize,
    pub framing_reserve: usize,
    pub context_window: usize,
    pub generation_seed: Option<i64>,
}

impl ChapterCriticBudget {
    pub fn scene_default(model_id: String, context_window: usize) -> Self {
        Self {
            model_id,
            input_limit: 24_000,
            output_reserve: 2_000,
            framing_reserve: 512,
            context_window,
            generation_seed: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChapterCriticError {
    BudgetUnknown(String),
    MandatoryOverflow(String),
    MalformedJson(String),
    Invalid(String),
}

impl std::fmt::Display for ChapterCriticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BudgetUnknown(reason) => write!(f, "budget_unknown: {reason}"),
            Self::MandatoryOverflow(reason) => write!(f, "mandatory_overflow: {reason}"),
            Self::MalformedJson(reason) => write!(f, "malformed_json: {reason}"),
            Self::Invalid(reason) => write!(f, "invalid_chapter_critique: {reason}"),
        }
    }
}

impl std::error::Error for ChapterCriticError {}

impl From<RevisionError> for ChapterCriticError {
    fn from(error: RevisionError) -> Self {
        match error {
            RevisionError::BudgetUnknown(reason) => Self::BudgetUnknown(reason),
            RevisionError::MandatoryOverflow(reason) => Self::MandatoryOverflow(reason),
            other => Self::MalformedJson(other.to_string()),
        }
    }
}

pub fn chapter_critique_json_schema() -> serde_json::Value {
    ChapterCritiqueV1::json_schema()
}

pub fn chapter_critique_output_template(chapter_number: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": spindle_core::drafting::CHAPTER_CRITIQUE_SCHEMA_VERSION,
        "chapter_number": chapter_number,
        "chapter_text_hash": "<chapter aggregate hash from this prompt>",
        "concerns": [],
        "edit_proposals": [],
        "strengths_to_preserve": [],
        "requires_author_decision": [],
        "no_edit_reason": "No chapter-level issue found."
    })
}

/// Aggregate hash over the chapter index and exact scene text.
pub fn chapter_text_hash(
    coverage: &ChapterCoverageV1,
    scene_texts: &BTreeMap<String, String>,
) -> String {
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    for scene in &coverage.chapter_index {
        hasher.update(scene.scene_id.as_bytes());
        hasher.update([0]);
        hasher.update(scene.text_sha256.as_bytes());
        hasher.update([0]);
        if let Some(text) = scene_texts.get(&scene.scene_id) {
            hasher.update(text.as_bytes());
        }
        hasher.update([0xff]);
    }
    format!("{:x}", hasher.finalize())
}

/// Assemble and token-verify the one-call chapter critic prompt. The full scene
/// index and prose are mandatory; if they do not fit, the caller must review
/// smaller deterministic windows instead of silently truncating.
pub fn assemble_chapter_critic_messages(
    coverage: &ChapterCoverageV1,
    window: Option<&ChapterWindowV1>,
    scene_texts: &BTreeMap<String, String>,
    feedback: &[CriticFindingInput],
    budget: &ChapterCriticBudget,
    tokenizer: Option<&dyn DraftTokenizer>,
    route_system: &str,
) -> Result<(DraftMessages, String, usize), ChapterCriticError> {
    let tokenizer =
        tokenizer.ok_or_else(|| ChapterCriticError::BudgetUnknown("no tokenizer".into()))?;
    if tokenizer.id().is_empty()
        || tokenizer.version().is_empty()
        || !tokenizer.supports_model(&budget.model_id)
    {
        return Err(ChapterCriticError::BudgetUnknown(
            "tokenizer/model mismatch".into(),
        ));
    }
    let system = if route_system.is_empty() {
        CHAPTER_CRITIC_SYSTEM.to_string()
    } else {
        format!("{route_system}\n\n{CHAPTER_CRITIC_SYSTEM}")
    };
    let index_json = serde_json::to_string(&coverage.chapter_index)
        .map_err(|error| ChapterCriticError::MalformedJson(error.to_string()))?;
    let coverage_json = serde_json::to_string(coverage)
        .map_err(|error| ChapterCriticError::MalformedJson(error.to_string()))?;
    let feedback_json = serde_json::to_string(feedback)
        .map_err(|error| ChapterCriticError::MalformedJson(error.to_string()))?;
    let template_json =
        serde_json::to_string(&chapter_critique_output_template(coverage.chapter_number))
            .map_err(|error| ChapterCriticError::MalformedJson(error.to_string()))?;
    let schema_json = serde_json::to_string(&chapter_critique_json_schema())
        .map_err(|error| ChapterCriticError::MalformedJson(error.to_string()))?;
    let window_refs: Vec<&spindle_core::drafting::ChapterSceneRefV1> = match window {
        Some(window) => {
            let mut refs = Vec::new();
            for scene_id in &window.scene_ids {
                let scene = coverage
                    .chapter_index
                    .iter()
                    .find(|scene| &scene.scene_id == scene_id)
                    .ok_or_else(|| {
                        ChapterCriticError::Invalid(format!(
                            "window references unknown scene {scene_id}"
                        ))
                    })?;
                refs.push(scene);
            }
            refs
        }
        None => coverage.chapter_index.iter().collect(),
    };
    let mut scene_block = String::new();
    for scene in window_refs {
        let text = scene_texts.get(&scene.scene_id).ok_or_else(|| {
            ChapterCriticError::Invalid(format!("missing text for {}", scene.scene_id))
        })?;
        scene_block.push_str(&format!(
            "\n--- SCENE {} (order {}, sha256 {}, {} words) ---\n{}\n",
            scene.scene_id, scene.scene_order, scene.text_sha256, scene.word_count, text
        ));
    }
    let window_scope = match window {
        Some(window) => format!(
            "window {} of {}: scenes {}",
            window.index,
            coverage.windows.len().max(1),
            window.scene_ids.join(", ")
        ),
        None => "full chapter index".to_string(),
    };
    let user = format!(
        "CHAPTER CRITIQUE CONTRACT\n\
         schema_version: {}\n\
         rubric_version: {CHAPTER_CRITIC_RUBRIC_VERSION}\n\
         chapter_number: {}\n\
         coverage_bps: {}\n\
         covered_scene_count: {} / {}\n\
         - Propose at most TWO scene edits.\n\
         - Every concern should carry an exact scene/hash/UTF-8 anchor; otherwise mark it advisory.\n\
         - Never claim to have reviewed a scene outside this chapter index/window.\n\
         - Return ONLY ChapterCritiqueV1 JSON.\n\n\
         CHAPTER CRITIQUE RUBRIC\n{CHAPTER_CRITIC_RUBRIC}\n\n\
         CHAPTER INDEX\n{index_json}\n\n\
         COVERAGE REPORT\n{coverage_json}\n\n\
         WINDOW SCOPE\n{window_scope}\n\n\
         RETAINED CHECKPOINT FEEDBACK\n{feedback_json}\n\n\
         OUTPUT TEMPLATE\n{template_json}\n\n\
         JSON SCHEMA\n{schema_json}\n\n\
         CHAPTER PROSE (MANDATORY, NEVER TRUNCATED)\n{scene_block}",
        spindle_core::drafting::CHAPTER_CRITIQUE_SCHEMA_VERSION,
        coverage.chapter_number,
        coverage.coverage_bps,
        coverage.covered_scene_count,
        coverage.total_scene_count,
    );
    let messages = DraftMessages { system, user };
    let input_tokens = tokenizer
        .count_messages(&messages)
        .map_err(ChapterCriticError::BudgetUnknown)?;
    if !fits_budget(
        input_tokens,
        budget.input_limit,
        budget.output_reserve,
        budget.framing_reserve,
        budget.context_window,
    ) {
        return Err(ChapterCriticError::MandatoryOverflow(
            "full chapter prose exceeds the chapter-critic budget; review deterministic scene windows instead".into(),
        ));
    }
    let prompt_hash = sha256_hex(
        serde_json::to_string(&messages)
            .map_err(|error| ChapterCriticError::MalformedJson(error.to_string()))?
            .as_bytes(),
    );
    Ok((messages, prompt_hash, input_tokens))
}

/// Parse, bind coverage, and validate a chapter critique against exact scene text.
pub fn parse_and_validate_chapter_critique_output(
    raw: &str,
    chapter_number: u32,
    chapter_text_hash: &str,
    coverage: &ChapterCoverageV1,
    scene_texts: &BTreeMap<String, String>,
) -> Result<ChapterCritiqueV1, ChapterCriticError> {
    let stripped = strip_thinking_and_fences(raw);
    let mut critique: ChapterCritiqueV1 = serde_json::from_str(stripped)
        .map_err(|error| ChapterCriticError::MalformedJson(error.to_string()))?;
    critique.chapter_number = chapter_number;
    critique.chapter_text_hash = chapter_text_hash.to_string();
    critique.coverage = Some(coverage.clone());
    validate_chapter_critique(&mut critique, scene_texts)
        .map_err(|error| ChapterCriticError::Invalid(error.to_string()))?;
    Ok(critique)
}

/// Bounded chapter context for both scene-judge orders when applying a selected
/// chapter proposal. Keeps only ids/enums/diagnoses, never the whole chapter.
pub fn build_chapter_judge_context(
    coverage: &ChapterCoverageV1,
    critique: &ChapterCritiqueV1,
    max_chars: usize,
) -> String {
    let mut out = format!(
        "CHAPTER {} CONTEXT (advisory; coverage {} / {} scenes)\n",
        coverage.chapter_number, coverage.covered_scene_count, coverage.total_scene_count
    );
    for concern in critique.concerns.iter().take(6) {
        out.push_str(&format!(
            "- [{}] {}{}: {}\n",
            concern.dimension.as_str(),
            if concern.advisory { "advisory " } else { "" },
            concern.severity_rank_label(),
            concern.diagnosis
        ));
    }
    for proposal in critique.edit_proposals.iter().take(2) {
        out.push_str(&format!(
            "- proposal {} for scene {}: {}\n",
            proposal.id, proposal.scene_id, proposal.requested_change
        ));
    }
    truncate_chars(&out, max_chars)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Save the advisory chapter critique beside the controller artifact.
pub fn save_chapter_critique(path: &Path, critique: &ChapterCritiqueV1) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), critique)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

pub fn load_chapter_critique(path: &Path) -> anyhow::Result<ChapterCritiqueV1> {
    Ok(serde_json::from_reader(std::fs::File::open(path)?)?)
}

trait ConcernLabel {
    fn severity_rank_label(&self) -> &'static str;
}

impl ConcernLabel for ChapterConcernV1 {
    fn severity_rank_label(&self) -> &'static str {
        match self.severity {
            spindle_core::drafting::DirectiveSeverity::Blocker => "blocker",
            spindle_core::drafting::DirectiveSeverity::Major => "major",
            spindle_core::drafting::DirectiveSeverity::Minor => "minor",
            spindle_core::drafting::DirectiveSeverity::Polish => "polish",
        }
    }
}

// Keep unused imports intentional: these types appear in public chapter DTOs
// and are re-exported through the module's tests.
#[allow(dead_code)]
fn _chapter_edit_types(_: &ChapterEditProposalV1, _: &AuthorDecisionItem) {}

/// Persist the advisory book queue beside the controller/run artifacts.
pub fn save_book_advisory_queue(
    path: &Path,
    queue: &spindle_core::drafting::BookAdvisoryQueueV1,
) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), queue)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

pub fn load_book_advisory_queue(
    path: &Path,
) -> anyhow::Result<spindle_core::drafting::BookAdvisoryQueueV1> {
    Ok(serde_json::from_reader(std::fs::File::open(path)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drafting::tests::VerifiedFixtureTokenizer;
    use spindle_core::drafting::{
        ChapterConcernDimension, ChapterEditProposalV1, ChapterSceneRefV1, DirectiveSeverity,
        plan_chapter_windows,
    };

    fn scene_texts() -> BTreeMap<String, String> {
        let mut texts = BTreeMap::new();
        texts.insert("scene:1".to_string(), "Cold water rose.".to_string());
        texts
    }

    fn coverage() -> ChapterCoverageV1 {
        plan_chapter_windows(
            2,
            &[ChapterSceneRefV1 {
                scene_id: "scene:1".into(),
                scene_order: 1,
                text_sha256: sha256_hex(b"Cold water rose."),
                word_count: 3,
            }],
            4,
        )
    }

    #[test]
    fn chapter_prompt_includes_all_scenes_and_rejects_overflow() {
        let budget = ChapterCriticBudget::scene_default("fixture".into(), 20_000);
        let (messages, hash, tokens) = assemble_chapter_critic_messages(
            &coverage(),
            None,
            &scene_texts(),
            &[],
            &budget,
            Some(&VerifiedFixtureTokenizer),
            "",
        )
        .expect("prompt assembles");
        assert!(messages.user.contains("Cold water rose."));
        assert!(messages.user.contains("CHAPTER PROSE"));
        assert!(!hash.is_empty());
        assert!(tokens > 0);
        let tiny = ChapterCriticBudget {
            input_limit: 10,
            ..budget
        };
        let error = assemble_chapter_critic_messages(
            &coverage(),
            None,
            &scene_texts(),
            &[],
            &tiny,
            Some(&VerifiedFixtureTokenizer),
            "",
        )
        .expect_err("overflow must not truncate");
        assert!(matches!(error, ChapterCriticError::MandatoryOverflow(_)));
    }

    #[test]
    fn full_chapter_overflow_dispatches_fitting_windows() {
        let texts = BTreeMap::from([
            (
                "scene:1".to_string(),
                format!("Cold water rose. {}", "a ".repeat(300)),
            ),
            (
                "scene:2".to_string(),
                format!("Warm fire fell. {}", "b ".repeat(1_000)),
            ),
        ]);
        let coverage = plan_chapter_windows(
            3,
            &[
                ChapterSceneRefV1 {
                    scene_id: "scene:1".into(),
                    scene_order: 1,
                    text_sha256: sha256_hex(texts["scene:1"].as_bytes()),
                    word_count: 600,
                },
                ChapterSceneRefV1 {
                    scene_id: "scene:2".into(),
                    scene_order: 2,
                    text_sha256: sha256_hex(texts["scene:2"].as_bytes()),
                    word_count: 2_000,
                },
            ],
            1,
        );
        assert_eq!(coverage.windows.len(), 2);
        let window_budget = ChapterCriticBudget::scene_default("fixture".into(), 20_000);
        let mut max_window_tokens = 0usize;
        for window in &coverage.windows {
            let (_, _, tokens) = assemble_chapter_critic_messages(
                &coverage,
                Some(window),
                &texts,
                &[],
                &window_budget,
                Some(&VerifiedFixtureTokenizer),
                "",
            )
            .expect("each window fits the normal budget");
            max_window_tokens = max_window_tokens.max(tokens);
        }
        let tight = ChapterCriticBudget {
            input_limit: max_window_tokens,
            ..window_budget
        };
        for window in &coverage.windows {
            assemble_chapter_critic_messages(
                &coverage,
                Some(window),
                &texts,
                &[],
                &tight,
                Some(&VerifiedFixtureTokenizer),
                "",
            )
            .expect("each planned window fits on its own");
        }
        let error = assemble_chapter_critic_messages(
            &coverage,
            None,
            &texts,
            &[],
            &tight,
            Some(&VerifiedFixtureTokenizer),
            "",
        )
        .expect_err("the full chapter does not fit the same budget");
        assert!(matches!(error, ChapterCriticError::MandatoryOverflow(_)));
    }

    #[test]
    fn parse_binds_coverage_and_caps_and_drops_invalid_proposals() {
        let text = "Cold water rose.";
        let good_anchor = serde_json::json!({
            "scene_id": "scene:1",
            "text_sha256": sha256_hex(text.as_bytes()),
            "start_byte": 0,
            "end_byte": 4,
            "exact_quote": "Cold"
        });
        let proposal = |id: &str, anchor: serde_json::Value| {
            serde_json::json!({
                "id": id,
                "scene_id": "scene:1",
                "base_text_sha256": sha256_hex(text.as_bytes()),
                "dimension": "openings_endings",
                "anchor": anchor,
                "diagnosis": "Opening is abstract.",
                "requested_change": "Ground the opening in physical action.",
                "preserve": ["POV"],
                "success_test": "The first sentence is physical.",
                "source_concern_ids": []
            })
        };
        let bad_anchor = serde_json::json!({
            "scene_id": "scene:1",
            "text_sha256": sha256_hex(text.as_bytes()),
            "start_byte": 0,
            "end_byte": 4,
            "exact_quote": "Warm"
        });
        let raw = serde_json::json!({
            "schema_version": "chapter_critique_v1",
            "chapter_number": 1,
            "chapter_text_hash": "ignored",
            "concerns": [{
                "id": "concern:1",
                "dimension": "openings_endings",
                "severity": "major",
                "anchor": good_anchor,
                "diagnosis": "Abstract opening.",
                "advisory": false
            }],
            "edit_proposals": [
                proposal("p1", good_anchor.clone()),
                proposal("p2", good_anchor.clone()),
                proposal("p3", good_anchor.clone()),
                proposal("bad", bad_anchor)
            ],
            "strengths_to_preserve": [],
            "requires_author_decision": [],
            "no_edit_reason": null
        })
        .to_string();
        let fenced = format!("```json\n{raw}\n```");
        let critique = parse_and_validate_chapter_critique_output(
            &fenced,
            2,
            "chapter-hash",
            &coverage(),
            &scene_texts(),
        )
        .expect("parses");
        assert_eq!(critique.chapter_number, 2);
        assert_eq!(critique.chapter_text_hash, "chapter-hash");
        assert!(critique.coverage.is_some());
        assert_eq!(critique.edit_proposals.len(), 2);
        assert_eq!(critique.concerns.len(), 1);
        assert!(!critique.concerns[0].advisory);
        assert_eq!(
            critique.concerns[0].dimension,
            ChapterConcernDimension::OpeningsEndings
        );
        assert_eq!(critique.concerns[0].severity, DirectiveSeverity::Major);
        let _ = ChapterEditProposalV1::clone;
    }
}
