//! Cluster → character-dossier builder.
//!
//! Pure logic: walks consolidated character clusters and their backing
//! mentions/segments to produce dossier drafts the service layer routes
//! through `Repository::upsert_import_character_dossier`. Ported from the
//! SurrealDB-era `crate::import::character`; SQLite records use `String`
//! ids so `record_id_string(&x.id)` becomes `x.id.clone()`.

use std::collections::{BTreeMap, BTreeSet};

use spindle_core::models::{
    CharacterEmotionalProfileData, CharacterVoiceProfileData, ImportCharacterRelationshipInference,
    ImportCharacterStateTrajectoryPoint, StoryPlacement,
};

use crate::sqlite::records::{ImportEntityCluster, ImportEntityMention, ImportSegment};

#[derive(Debug, Clone)]
pub struct CharacterDossierDraft {
    pub cluster_id: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub importance_rank: i32,
    pub voice_profile: CharacterVoiceProfileData,
    pub emotional_profile: CharacterEmotionalProfileData,
    pub state_trajectory: Vec<ImportCharacterStateTrajectoryPoint>,
    pub relationship_inferences: Vec<ImportCharacterRelationshipInference>,
    pub decision_patterns: Vec<String>,
    pub dialogue_samples: Vec<String>,
    pub confidence: f64,
    pub review_required: bool,
    pub review_notes: Vec<String>,
}

pub fn build_character_dossiers(
    clusters: &[ImportEntityCluster],
    mentions: &[ImportEntityMention],
    segments: &[ImportSegment],
) -> Vec<CharacterDossierDraft> {
    let segments_by_id = segments
        .iter()
        .map(|segment| (segment.id.clone(), segment))
        .collect::<BTreeMap<_, _>>();
    let character_clusters = clusters
        .iter()
        .filter(|cluster| cluster.entity_kind == "character")
        .collect::<Vec<_>>();
    let major_cutoff = major_character_cutoff(character_clusters.len());

    character_clusters
        .into_iter()
        .take(major_cutoff)
        .filter_map(|cluster| build_character_dossier(cluster, mentions, &segments_by_id, clusters))
        .collect()
}

pub fn build_character_dossiers_for_clusters(
    clusters: &[ImportEntityCluster],
    mentions: &[ImportEntityMention],
    segments: &[ImportSegment],
) -> Vec<CharacterDossierDraft> {
    let segments_by_id = segments
        .iter()
        .map(|segment| (segment.id.clone(), segment))
        .collect::<BTreeMap<_, _>>();

    clusters
        .iter()
        .filter(|cluster| cluster.entity_kind == "character")
        .filter_map(|cluster| build_character_dossier(cluster, mentions, &segments_by_id, clusters))
        .collect()
}

fn build_character_dossier(
    cluster: &ImportEntityCluster,
    mentions: &[ImportEntityMention],
    segments_by_id: &BTreeMap<String, &ImportSegment>,
    all_clusters: &[ImportEntityCluster],
) -> Option<CharacterDossierDraft> {
    let mention_ids = cluster.mention_ids.iter().cloned().collect::<BTreeSet<_>>();
    let character_mentions = mentions
        .iter()
        .filter(|mention| mention_ids.contains(&mention.id))
        .collect::<Vec<_>>();
    if character_mentions.is_empty() {
        return None;
    }

    let dialogue_samples = collect_dialogue_samples(&character_mentions);
    let voice_profile = build_voice_profile(&dialogue_samples, &cluster.canonical_name);
    let emotional_profile = build_emotional_profile(&character_mentions);
    let state_trajectory =
        build_state_trajectory(&character_mentions, segments_by_id, &cluster.canonical_name);
    let relationship_inferences =
        build_relationships(cluster, &character_mentions, mentions, all_clusters);
    let decision_patterns = build_decision_patterns(&character_mentions);
    let mut review_notes = cluster.notes.clone();
    if cluster.review_required && review_notes.is_empty() {
        review_notes
            .push("entity consolidation marked this character cluster as ambiguous".to_string());
    }
    if dialogue_samples.len() < 2 {
        review_notes.push("thin dialogue sample for imported voice analysis".to_string());
    }
    if relationship_inferences
        .iter()
        .any(|relation| relation.confidence < 0.65)
    {
        review_notes.push("one or more relationship reads were low-confidence".to_string());
    }
    let confidence = character_confidence(
        dialogue_samples.len(),
        state_trajectory.len(),
        relationship_inferences.len(),
    );

    Some(CharacterDossierDraft {
        cluster_id: cluster.id.clone(),
        canonical_name: cluster.canonical_name.clone(),
        aliases: cluster.aliases.clone(),
        importance_rank: cluster.importance_rank as i32,
        voice_profile,
        emotional_profile,
        state_trajectory,
        relationship_inferences,
        decision_patterns,
        dialogue_samples,
        confidence,
        review_required: cluster.review_required || !review_notes.is_empty(),
        review_notes,
    })
}

fn major_character_cutoff(cluster_count: usize) -> usize {
    cluster_count.clamp(1, 6)
}

fn collect_dialogue_samples(mentions: &[&ImportEntityMention]) -> Vec<String> {
    mentions
        .iter()
        .filter_map(|mention| mention.surrounding_text.clone())
        .filter(|text| {
            text.contains('"')
                || text.contains(" said ")
                || text.contains(" asked ")
                || text.contains(" whispered ")
                || text.contains(" ordered ")
                || text.contains(" warned ")
                || text.contains(" shouted ")
        })
        .take(4)
        .collect()
}

fn build_voice_profile(
    dialogue_samples: &[String],
    canonical_name: &str,
) -> CharacterVoiceProfileData {
    let joined = dialogue_samples.join(" ").to_ascii_lowercase();
    let sentence_structure = if joined.matches(',').count() >= 2 {
        vec!["compound".to_string()]
    } else {
        vec!["direct".to_string()]
    };
    let vocabulary = keyword_tokens(&joined)
        .into_iter()
        .take(5)
        .collect::<Vec<_>>();
    let tics = ["always", "never", "damn", "please"]
        .into_iter()
        .filter(|marker| joined.contains(marker))
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    CharacterVoiceProfileData {
        tone: None,
        vocabulary,
        sentence_structure,
        tics,
        forbidden_words: Vec::new(),
        example_lines: if dialogue_samples.is_empty() {
            vec![format!(
                "{} has limited on-page dialogue in the import.",
                canonical_name
            )]
        } else {
            dialogue_samples.to_vec()
        },
        established_in_scene_id: None,
        updated_at: None,
    }
}

fn build_emotional_profile(mentions: &[&ImportEntityMention]) -> CharacterEmotionalProfileData {
    let joined = mentions
        .iter()
        .filter_map(|mention| mention.surrounding_text.as_deref())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let mut base_emotions = BTreeMap::new();
    for (label, marker) in [
        ("fear", "fear"),
        ("anger", "anger"),
        ("grief", "grief"),
        ("hope", "hope"),
        ("resolve", "vow"),
    ] {
        if joined.contains(marker) {
            base_emotions.insert(label.to_string(), serde_json::Value::from(0.7));
        }
    }
    if base_emotions.is_empty() {
        base_emotions.insert("guarded".to_string(), serde_json::Value::from(0.5));
    }

    let triggers = ["betrayal", "fire", "warning", "threat", "loss"]
        .into_iter()
        .filter(|marker| joined.contains(marker))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let defense_mechanisms = ["control", "humor", "withdrawal", "discipline"]
        .into_iter()
        .filter(|marker| joined.contains(marker))
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    CharacterEmotionalProfileData {
        base_emotions,
        suppressed: Vec::new(),
        triggers,
        defense_mechanisms,
        flex_range: None,
    }
}

fn build_state_trajectory(
    mentions: &[&ImportEntityMention],
    segments_by_id: &BTreeMap<String, &ImportSegment>,
    canonical_name: &str,
) -> Vec<ImportCharacterStateTrajectoryPoint> {
    let mut points = mentions
        .iter()
        .filter_map(|mention| {
            let segment_id = mention.segment_id.clone();
            let segment = segments_by_id.get(&segment_id)?;
            Some(ImportCharacterStateTrajectoryPoint {
                segment_id,
                placement: Some(StoryPlacement {
                    book_number: segment.book_number.unwrap_or(1) as i32,
                    chapter_number: segment.chapter_number.unwrap_or(1) as i32,
                    scene_order: segment.scene_order.map(|value| value as i32),
                    note: segment.label.clone(),
                }),
                summary: mention
                    .surrounding_text
                    .clone()
                    .unwrap_or_else(|| format!("{} appears in this segment.", canonical_name)),
                emotional_state: emotional_state_snapshot(mention.surrounding_text.as_deref()),
                goals: infer_goals(mention.surrounding_text.as_deref()),
                status: infer_status(mention.surrounding_text.as_deref()),
                confidence: mention.confidence,
            })
        })
        .collect::<Vec<_>>();
    points.sort_by_key(|point| {
        point
            .placement
            .as_ref()
            .map(|placement| {
                (
                    placement.book_number,
                    placement.chapter_number,
                    placement.scene_order.unwrap_or(0),
                )
            })
            .unwrap_or((0, 0, 0))
    });
    points
}

fn build_relationships(
    cluster: &ImportEntityCluster,
    character_mentions: &[&ImportEntityMention],
    all_mentions: &[ImportEntityMention],
    all_clusters: &[ImportEntityCluster],
) -> Vec<ImportCharacterRelationshipInference> {
    let cluster_segment_ids = character_mentions
        .iter()
        .map(|mention| mention.segment_id.clone())
        .collect::<BTreeSet<_>>();
    let other_cluster_names = all_clusters
        .iter()
        .filter(|other| other.entity_kind == "character" && other.id != cluster.id)
        .map(|other| {
            (
                other.canonical_name.to_ascii_lowercase(),
                other.id.clone(),
                other.canonical_name.clone(),
            )
        })
        .collect::<Vec<_>>();

    let mut relationships = Vec::new();
    for (normalized_name, cluster_id, canonical_name) in other_cluster_names {
        let related_mentions = all_mentions
            .iter()
            .filter(|mention| {
                cluster_segment_ids.contains(&mention.segment_id)
                    && mention
                        .surface_form
                        .to_ascii_lowercase()
                        .contains(&normalized_name)
            })
            .count();
        if related_mentions == 0 {
            continue;
        }
        let confidence = if related_mentions >= 2 { 0.76 } else { 0.6 };
        relationships.push(ImportCharacterRelationshipInference {
            other_character_cluster_id: cluster_id,
            summary: format!(
                "{} and {} share {} imported segment(s).",
                cluster.canonical_name, canonical_name, related_mentions
            ),
            trust_signal: Some(if related_mentions >= 2 {
                "shared focus".to_string()
            } else {
                "possible alliance".to_string()
            }),
            tension_signal: None,
            confidence,
        });
    }
    relationships
}

fn build_decision_patterns(mentions: &[&ImportEntityMention]) -> Vec<String> {
    let joined = mentions
        .iter()
        .filter_map(|mention| mention.surrounding_text.as_deref())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let mut notes = Vec::new();
    if ["decides", "orders", "refuses", "warns"]
        .iter()
        .any(|marker| joined.contains(marker))
    {
        notes.push("tends to act decisively under pressure".to_string());
    }
    if ["waits", "hesitates", "watches"]
        .iter()
        .any(|marker| joined.contains(marker))
    {
        notes.push("often delays action until more information is available".to_string());
    }
    if notes.is_empty() {
        notes.push("decision pattern is still thin in the imported text".to_string());
    }
    notes
}

fn emotional_state_snapshot(context: Option<&str>) -> BTreeMap<String, serde_json::Value> {
    let mut snapshot = BTreeMap::new();
    let lowered = context.unwrap_or_default().to_ascii_lowercase();
    for marker in ["angry", "afraid", "calm", "grim", "hopeful"] {
        if lowered.contains(marker) {
            snapshot.insert(marker.to_string(), serde_json::Value::from(0.7));
        }
    }
    if snapshot.is_empty() {
        snapshot.insert("guarded".to_string(), serde_json::Value::from(0.5));
    }
    snapshot
}

fn infer_goals(context: Option<&str>) -> Vec<String> {
    let lowered = context.unwrap_or_default().to_ascii_lowercase();
    let mut goals = Vec::new();
    if lowered.contains("warning") {
        goals.push("deliver a warning".to_string());
    }
    if lowered.contains("gate") {
        goals.push("reach the gate".to_string());
    }
    if goals.is_empty() {
        goals.push("maintain control of the current situation".to_string());
    }
    goals
}

fn infer_status(context: Option<&str>) -> Vec<String> {
    let lowered = context.unwrap_or_default().to_ascii_lowercase();
    let mut status = Vec::new();
    if lowered.contains("wounded") {
        status.push("wounded".to_string());
    }
    if lowered.contains("waiting") || lowered.contains("waited") {
        status.push("waiting".to_string());
    }
    if status.is_empty() {
        status.push("active".to_string());
    }
    status
}

fn character_confidence(
    dialogue_count: usize,
    state_points: usize,
    relationship_count: usize,
) -> f64 {
    let dialogue_score = if dialogue_count >= 3 {
        0.9
    } else if dialogue_count >= 1 {
        0.72
    } else {
        0.52
    };
    let state_score = if state_points >= 3 {
        0.86
    } else if state_points >= 1 {
        0.7
    } else {
        0.5
    };
    let relationship_score = if relationship_count >= 2 {
        0.8
    } else if relationship_count == 1 {
        0.68
    } else {
        0.56
    };
    ((dialogue_score + state_score + relationship_score) / 3.0_f64).clamp(0.0_f64, 0.99_f64)
}

fn keyword_tokens(input: &str) -> Vec<String> {
    input
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '\'')
        .filter_map(|token| {
            let token = token.trim().to_ascii_lowercase();
            if token.len() >= 4 { Some(token) } else { None }
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
