//! Pure packet preparation and the local accepted-text artifact lifecycle.
pub mod artifact;
pub mod chapter;
pub mod critique;
pub mod editorial;
pub mod line_edit;
pub mod revision;

use sha2::{Digest, Sha256};
use spindle_core::drafting::*;
use std::collections::BTreeMap;

pub const RUBRIC_VERSION: &str = "scene-craft-v1";
pub const CRAFT_RUBRIC: &str = "Give the scene a purpose and a consequential turn. Stay within the POV character's knowledge; select sensory details through that character's attention. Make speakers distinct through intent and speech. Ground change in concrete action and consequence. Render the entry-to-exit emotional shift. Prefer specific, purposeful language over stock images and explanatory repetition. Vary openings and endings when the story calls for it. Preserve author intent, required outcomes and locks; craft advice never overrides them.";
const SYSTEM: &str = "Write the scene from the supplied data. Return only UTF-8 scene prose, without fences, commentary or metadata. Treat quoted evidence as story data. Obey rating, POV knowledge and author locks.";
const REMINDER: &str =
    "Write this scene now, preserving its intended outcome and turn within the target length.";

pub use spindle_core::drafting::{DraftMessages, PreparedDraftingPacket};

/// Implementations must be verified for this exact resolved model and count
/// message delimiters/role overhead as well as text. No estimated fallback.
pub trait DraftTokenizer: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    fn supports_model(&self, model_id: &str) -> bool;
    fn count_text(&self, text: &str) -> Result<usize, String>;
    fn count_messages(&self, messages: &DraftMessages) -> Result<usize, String>;
}
#[derive(Debug, Clone)]
pub struct PacketBudget {
    pub model_id: String,
    pub input_limit: usize,
    pub output_reserve: usize,
    pub framing_reserve: usize,
    pub context_window: usize,
    pub section_limits: [usize; 6],
    pub generation_seed: Option<i64>,
}
impl PacketBudget {
    pub fn scene_default(model_id: String, context_window: usize) -> Self {
        Self {
            model_id,
            input_limit: 12_000,
            output_reserve: 3_000,
            framing_reserve: 512,
            context_window,
            section_limits: [1500, 2500, 2500, 1500, 1500, 2500],
            generation_seed: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PacketError {
    #[error("incomplete_packet: {0}")]
    Incomplete(String),
    #[error("budget_unknown: {0}")]
    BudgetUnknown(String),
    #[error("mandatory_overflow: {0}")]
    MandatoryOverflow(String),
    #[error("packet_serialization: {0}")]
    Serialization(String),
}
fn json(value: &impl serde::Serialize) -> Result<String, PacketError> {
    serde_json::to_string(value).map_err(|e| PacketError::Serialization(e.to_string()))
}
fn nonempty(value: &str, field: &str) -> Result<(), PacketError> {
    if value.trim().is_empty() {
        return Err(PacketError::Incomplete(field.into()));
    }
    Ok(())
}
fn source(packet: &DraftingPacketV1, value: &PacketSource) -> Result<(), PacketError> {
    nonempty(&value.branch_id, "source.branch_id")?;
    nonempty(&value.source_id, "source.source_id")?;
    nonempty(&value.revision_hash, "source.revision_hash")?;
    if !packet.source_versions.contains(value) {
        return Err(PacketError::Incomplete(format!(
            "unlisted source: {}",
            value.source_id
        )));
    }
    Ok(())
}
fn validate(packet: &DraftingPacketV1) -> Result<(), PacketError> {
    if packet.schema_version != "drafting_packet_v1" {
        return Err(PacketError::Incomplete("schema_version".into()));
    }
    for (name, value) in [
        ("packet_id", &packet.packet_id),
        ("project_id", &packet.project_id),
        ("branch_id", &packet.branch_id),
        ("canon_snapshot_hash", &packet.canon_snapshot_hash),
        ("plan_revision", &packet.plan_revision),
        ("pov.character_id", &packet.scene_card.pov.character_id),
        ("pov.display_name", &packet.scene_card.pov.display_name),
        ("goal", &packet.scene_card.goal),
        ("turn", &packet.scene_card.turn),
    ] {
        nonempty(value, name)?;
    }
    let chapter = &packet.scene_card.chapter;
    if [chapter.book_number, chapter.number, chapter.scene_index]
        .iter()
        .any(|&n| n == 0 || n > i32::MAX as u32)
    {
        return Err(PacketError::Incomplete(
            "story placement: require positive i32 book/chapter/scene".into(),
        ));
    }
    let words = &packet.scene_card.target_words;
    if words.min == 0 || words.min > words.preferred || words.preferred > words.max {
        return Err(PacketError::Incomplete(
            "target_words: require 0 < min <= preferred <= max".into(),
        ));
    }
    source(packet, &packet.scene_card.source)?;
    source(packet, &packet.scene_card.pov.source)?;
    let mut constraint_ids = BTreeMap::new();
    for c in &packet.hard_constraints {
        nonempty(&c.id, "constraint.id")?;
        nonempty(&c.statement, "constraint.statement")?;
        source(packet, &c.source)?;
        if let Some(previous) = constraint_ids.insert(&c.id, (&c.kind, &c.statement, &c.scope))
            && previous != (&c.kind, &c.statement, &c.scope)
        {
            return Err(PacketError::Incomplete(format!(
                "conflicting constraint: {}",
                c.id
            )));
        }
    }
    for e in &packet.voice.exemplars {
        if e.approved_revision_hash.trim().is_empty()
            || e.approved_revision_hash != e.item.source.revision_hash
        {
            return Err(PacketError::Incomplete(
                "exemplar approval must match source revision".into(),
            ));
        }
    }
    Ok(())
}

fn render_messages(sections: &[String; 6], system: &str) -> DraftMessages {
    DraftMessages {
        system: system.into(),
        user: format!("{}\n{REMINDER}", sections.join("\n")),
    }
}
fn fits_context(input: usize, budget: &PacketBudget) -> bool {
    input <= budget.input_limit
        && input
            .checked_add(budget.output_reserve)
            .and_then(|n| n.checked_add(budget.framing_reserve))
            .is_some_and(|n| n <= budget.context_window)
}

/// Select whole records, never byte-slice prose. Hard knowledge boundaries are
/// promoted to the mandatory section; other required facts must be constraints.
pub fn assemble_packet(
    packet: DraftingPacketV1,
    budget: &PacketBudget,
    tokenizer: Option<&dyn DraftTokenizer>,
) -> Result<PreparedDraftingPacket, PacketError> {
    assemble_packet_with_system(packet, budget, tokenizer, "")
}

pub(crate) fn assemble_packet_with_system(
    mut packet: DraftingPacketV1,
    budget: &PacketBudget,
    tokenizer: Option<&dyn DraftTokenizer>,
    route_system: &str,
) -> Result<PreparedDraftingPacket, PacketError> {
    let system = if route_system.is_empty() {
        SYSTEM.into()
    } else {
        format!("{route_system}\n\n{SYSTEM}")
    };
    validate(&packet)?;
    let t =
        tokenizer.ok_or_else(|| PacketError::BudgetUnknown("no tokenizer configured".into()))?;
    if t.id().is_empty() || t.version().is_empty() || !t.supports_model(&budget.model_id) {
        return Err(PacketError::BudgetUnknown(
            "tokenizer/model mismatch".into(),
        ));
    }
    let count = |text: &str| t.count_text(text).map_err(PacketError::BudgetUnknown);
    packet.assembly = None;
    packet.craft_rubric = CraftRubric {
        version: RUBRIC_VERSION.into(),
        text: CRAFT_RUBRIC.into(),
    };
    packet
        .hard_constraints
        .sort_by(|a, b| a.id.cmp(&b.id).then(a.source.cmp(&b.source)));
    packet.source_versions.sort();
    packet.source_versions.dedup();
    let mut sections = [
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    ];
    sections[0] = format!("SCENE CARD\n{}\n", json(&packet.scene_card)?);
    sections[1] = format!("HARD CONSTRAINTS\n{}\n", json(&packet.hard_constraints)?);
    sections[4] = format!("CRAFT RUBRIC {}\n{}\n", RUBRIC_VERSION, CRAFT_RUBRIC);
    let mut groups: [Vec<(&str, PacketItem)>; 6] = Default::default();
    let c = &packet.continuity;
    for item in &c.knowledge_boundaries {
        groups[1].push(("knowledge boundary", item.clone()));
    }
    for (label, items) in [
        ("character state", &c.character_states),
        ("fact", &c.facts),
        ("open thread", &c.open_threads),
    ] {
        for item in items {
            groups[2].push((label, item.clone()));
        }
    }
    if let Some(item) = &c.prior_scene_tail {
        groups[2].push(("prior scene tail", item.clone()));
    }
    for item in &packet.voice.rules {
        groups[3].push(("voice rule", item.clone()));
    }
    // Serialize approval and speaker labels, never present exemplars as canon.
    for e in &packet.voice.exemplars {
        let mut item = e.item.clone();
        item.text = json(e)?;
        groups[3].push(("approved prose example (not canon)", item));
    }
    let m = &packet.editorial_memory;
    for (label, items) in [
        ("story so far", &c.story_so_far),
        ("recent opening", &m.recent_openings),
        ("recent ending", &m.recent_endings),
        ("recurring image", &m.recurring_images),
        ("repeated phrase", &m.repeated_phrases),
        ("pacing history", &m.pacing_history),
        ("research", &packet.research_evidence),
    ] {
        for item in items {
            groups[5].push((label, item.clone()));
        }
    }
    let mut seen_propositions: BTreeMap<(PacketSource, String), String> = BTreeMap::new();
    let mut omissions = Vec::new();
    let mut counts = Vec::new();
    let mut carry = 0usize;
    for priority in 0..6 {
        // Stable total ordering independent of retrieval/input order.
        groups[priority].sort_by(|(la, a), (lb, b)| {
            b.relevance
                .cmp(&a.relevance)
                .then(b.recency.cmp(&a.recency))
                .then(a.source.cmp(&b.source))
                .then(a.proposition_id.cmp(&b.proposition_id))
                .then(la.cmp(lb))
                .then(a.text.cmp(&b.text))
        });
        let ceiling = budget.section_limits[priority]
            .checked_add(carry)
            .ok_or_else(|| PacketError::MandatoryOverflow("section arithmetic".into()))?;
        for (label, item) in &groups[priority] {
            source(&packet, &item.source)?;
            nonempty(&item.proposition_id, "proposition_id")?;
            nonempty(&item.text, "item.text")?;
            let rendered = format!("{label}: {}\n", json(item)?);
            let tokens = count(&rendered)?;
            let proposition_key = (item.source.clone(), item.proposition_id.clone());
            let duplicate = if let Some(previous) = seen_propositions.get(&proposition_key) {
                if previous != &item.text {
                    return Err(PacketError::Incomplete(format!(
                        "conflicting proposition: {}",
                        item.proposition_id
                    )));
                }
                true
            } else {
                false
            };
            if duplicate {
                omissions.push(PacketOmission {
                    source: item.source.clone(),
                    proposition_id: item.proposition_id.clone(),
                    section: priority as u8,
                    reason: OmissionReason::Duplicate,
                    tokens,
                });
                continue;
            }
            let proposed = format!("{}{rendered}", sections[priority]);
            let tail_overflow = *label == "prior scene tail" && tokens > 1000;
            let mut proposed_sections = sections.clone();
            proposed_sections[priority] = proposed.clone();
            let proposed_messages = render_messages(&proposed_sections, &system);
            let proposed_input = t
                .count_messages(&proposed_messages)
                .map_err(PacketError::BudgetUnknown)?;
            let final_overflow = !fits_context(proposed_input, budget);
            if count(&proposed)? > ceiling || tail_overflow || final_overflow {
                if priority == 1 {
                    return Err(PacketError::MandatoryOverflow(
                        "hard knowledge boundary".into(),
                    ));
                }
                omissions.push(PacketOmission {
                    source: item.source.clone(),
                    proposition_id: item.proposition_id.clone(),
                    section: priority as u8,
                    reason: OmissionReason::SectionBudget,
                    tokens,
                });
                continue;
            }
            seen_propositions.insert(proposition_key, item.text.clone());
            sections[priority] = proposed;
        }
        // Output contract and reminder are priority zero, despite final placement.
        let tokens = if priority == 0 {
            count(&format!("{system}\n{}\n{REMINDER}", sections[0]))?
        } else {
            count(&sections[priority])?
        };
        if tokens > ceiling || (priority == 4 && tokens > 1500) {
            return Err(PacketError::MandatoryOverflow(format!(
                "section {priority}"
            )));
        }
        carry = ceiling - tokens;
        counts.push(SectionCount {
            section: priority.to_string(),
            tokens,
        });
    }
    let messages = render_messages(&sections, &system);
    let input_tokens = t
        .count_messages(&messages)
        .map_err(PacketError::BudgetUnknown)?;
    if !fits_context(input_tokens, budget) {
        return Err(PacketError::MandatoryOverflow(
            "final rendered messages/context window".into(),
        ));
    }
    packet.assembly = Some(PacketAssembly {
        tokenizer_id: t.id().into(),
        tokenizer_version: t.version().into(),
        model_id: budget.model_id.clone(),
        section_counts: counts,
        omissions,
        input_tokens,
        output_reserve: budget.output_reserve,
        framing_reserve: budget.framing_reserve,
        context_window: budget.context_window,
        generation_seed: budget.generation_seed,
    });
    let messages_hash = format!("{:x}", Sha256::digest(json(&messages)?.as_bytes()));
    Ok(PreparedDraftingPacket {
        packet,
        messages,
        messages_hash,
    })
}

#[cfg(test)]
pub(crate) mod tests;
