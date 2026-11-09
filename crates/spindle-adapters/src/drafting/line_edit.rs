//! Optional post-acceptance line editing: diction, syntax, and dialogue polish
//! only. The candidate still passes the deterministic checks and the same
//! swapped-order acceptance gate before promotion.

use super::{
    DraftMessages, DraftTokenizer,
    critique::sha256_hex,
    revision::{RevisionError, fits_budget},
};
use spindle_core::drafting::DraftingPacketV1;

pub const LINE_EDIT_SYSTEM: &str = "\
You are Spindle's line editor. Polish the supplied accepted scene prose at the line level: diction, \
syntax, rhythm, and dialogue tagging/voice. Preserve every event, the scene turn, planned outcome, \
POV knowledge, rating, author locks, and paragraph structure. Do not add new plot, facts, characters, \
or imagery. Return ONLY the polished UTF-8 scene prose, without markdown fences, JSON, headings, or commentary.";

pub const LINE_EDIT_RUBRIC_VERSION: &str = "line-edit-v1";

/// Per-call token reservation for optional line editing. Same shape and limits
/// as the prior-draft revision pass.
#[derive(Debug, Clone)]
pub struct LineEditBudget {
    pub model_id: String,
    pub input_limit: usize,
    pub output_reserve: usize,
    pub framing_reserve: usize,
    pub context_window: usize,
    pub generation_seed: Option<i64>,
}

impl LineEditBudget {
    pub fn scene_default(model_id: String, context_window: usize) -> Self {
        Self {
            model_id,
            input_limit: 18_000,
            output_reserve: 3_000,
            framing_reserve: 512,
            context_window,
            generation_seed: None,
        }
    }
}

/// Assemble and token-verify the line-edit prompt. The full accepted scene text
/// and scene contract are mandatory; the packet's voice/editorial memory are
/// included only while they fit the line-edit budget.
pub fn assemble_line_edit_messages(
    packet: &DraftingPacketV1,
    accepted_text: &str,
    accepted_text_hash: &str,
    budget: &LineEditBudget,
    tokenizer: Option<&dyn DraftTokenizer>,
    route_system: &str,
) -> Result<(DraftMessages, String, usize), RevisionError> {
    let tokenizer =
        tokenizer.ok_or_else(|| RevisionError::BudgetUnknown("no tokenizer configured".into()))?;
    if tokenizer.id().is_empty()
        || tokenizer.version().is_empty()
        || !tokenizer.supports_model(&budget.model_id)
    {
        return Err(RevisionError::BudgetUnknown(
            "tokenizer/model mismatch".into(),
        ));
    }
    let system = if route_system.is_empty() {
        LINE_EDIT_SYSTEM.to_string()
    } else {
        format!("{route_system}\n\n{LINE_EDIT_SYSTEM}")
    };
    let scene_card_json = serde_json::to_string(&packet.scene_card)
        .map_err(|error| RevisionError::MalformedJson(error.to_string()))?;
    let constraints_json = serde_json::to_string(&packet.hard_constraints)
        .map_err(|error| RevisionError::MalformedJson(error.to_string()))?;
    let knowledge_json = serde_json::to_string(&packet.continuity.knowledge_boundaries)
        .map_err(|error| RevisionError::MalformedJson(error.to_string()))?;
    let voice_json = serde_json::to_string(&packet.voice)
        .map_err(|error| RevisionError::MalformedJson(error.to_string()))?;
    let memory_json = serde_json::to_string(&packet.editorial_memory)
        .map_err(|error| RevisionError::MalformedJson(error.to_string()))?;

    let render = |voice_sec: &str, memory_sec: &str| -> String {
        format!(
            "LINE EDIT CONTRACT\n\
             rubric_version: {LINE_EDIT_RUBRIC_VERSION}\n\
             accepted_text_hash: {accepted_text_hash}\n\
             accepted_utf8_byte_len: {}\n\
             - Polish diction, syntax, rhythm, and dialogue only.\n\
             - Preserve every event, the scene turn, planned outcome, POV, rating, and author locks.\n\
             - Do not add or remove plot, facts, characters, or major imagery.\n\
             - Return ONLY the polished UTF-8 scene prose.\n\n\
             SCENE CARD\n{scene_card_json}\n\n\
             HARD CONSTRAINTS\n{constraints_json}\n\n\
             KNOWLEDGE BOUNDARIES (MANDATORY)\n{knowledge_json}\n\n\
             VOICE & STYLE CONTEXT\n{voice_sec}\n\
             EDITORIAL MEMORY\n{memory_sec}\n\
             ACCEPTED SCENE PROSE UTF-8\n{accepted_text}\n",
            accepted_text.len()
        )
    };

    let mandatory = DraftMessages {
        system: system.clone(),
        user: render("", ""),
    };
    let mandatory_tokens = tokenizer
        .count_messages(&mandatory)
        .map_err(RevisionError::BudgetUnknown)?;
    if !fits_budget(
        mandatory_tokens,
        budget.input_limit,
        budget.output_reserve,
        budget.framing_reserve,
        budget.context_window,
    ) {
        return Err(RevisionError::MandatoryOverflow(
            "mandatory line-edit context (scene card, constraints, knowledge, and accepted prose) exceeds line-edit budget".into(),
        ));
    }

    let try_section = |current: &str, extra: &str| -> Option<String> {
        let proposed = format!("{current}{extra}");
        let messages = DraftMessages {
            system: system.clone(),
            user: render(&proposed, ""),
        };
        let tokens = tokenizer.count_messages(&messages).ok()?;
        fits_budget(
            tokens,
            budget.input_limit,
            budget.output_reserve,
            budget.framing_reserve,
            budget.context_window,
        )
        .then_some(proposed)
    };
    let mut voice_sec = String::new();
    if let Some(proposed) = try_section(&voice_sec, &format!("{voice_json}\n")) {
        voice_sec = proposed;
    }
    let mut memory_sec = String::new();
    if let Some(proposed) = try_section(&memory_sec, &format!("{memory_json}\n")) {
        memory_sec = proposed;
    }

    let messages = DraftMessages {
        system,
        user: render(&voice_sec, &memory_sec),
    };
    let input_tokens = tokenizer
        .count_messages(&messages)
        .map_err(RevisionError::BudgetUnknown)?;
    if !fits_budget(
        input_tokens,
        budget.input_limit,
        budget.output_reserve,
        budget.framing_reserve,
        budget.context_window,
    ) {
        return Err(RevisionError::MandatoryOverflow(
            "line-edit prompt exceeds input budget".into(),
        ));
    }
    let prompt_hash = sha256_hex(
        serde_json::to_string(&messages)
            .map_err(|error| RevisionError::MalformedJson(error.to_string()))?
            .as_bytes(),
    );
    Ok((messages, prompt_hash, input_tokens))
}
