//! Final-state computation pass.
//!
//! Pure logic: walks character dossiers + world/narrative drafts to compute
//! the import resume snapshot the service layer persists via
//! `Repository::upsert_import_resume_snapshot`. Ported from the SurrealDB-era
//! `crate::import::final_state`; SQLite records use plain `String` ids so
//! no `record_id_string` translation is needed.

use std::collections::{BTreeMap, BTreeSet};

use spindle_core::models::{
    ImportCharacterDossierSummary, ImportConfidenceLevel, ImportFinalCharacterStateSnapshot,
    ImportFinalLocationStateSnapshot, ImportFinalRelationshipSnapshot,
    ImportNarrativeDossierSummary, ImportPlotThreadSnapshot, ImportResumeSnapshotSummary,
    ImportWorldDossierSummary, normalize_name,
};

use crate::sqlite::records::ImportSegment;

#[derive(Debug, Clone)]
pub struct FinalStateReviewDraft {
    pub title: String,
    pub description: String,
    pub related_segment_ids: Vec<String>,
    pub target_record_id: Option<String>,
    pub corrected_summary: String,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub struct FinalStateDraft {
    pub resume_snapshot: ImportResumeSnapshotSummary,
    pub review_items: Vec<FinalStateReviewDraft>,
}

pub fn compute_final_state(
    segments: &[ImportSegment],
    character_dossiers: &[ImportCharacterDossierSummary],
    world: Option<&ImportWorldDossierSummary>,
    narrative: Option<&ImportNarrativeDossierSummary>,
) -> FinalStateDraft {
    let characters = build_character_snapshots(character_dossiers);
    let relationships = build_relationship_snapshots(character_dossiers);
    let locations = build_location_snapshots(world);
    let plot_threads = build_plot_thread_snapshots(narrative);
    let (book_number, chapter_number, scene_order) = resume_position(segments);
    let summary = build_resume_summary(
        book_number,
        chapter_number,
        scene_order,
        &characters,
        &locations,
        &plot_threads,
    );
    let resume_snapshot = ImportResumeSnapshotSummary {
        book_number,
        chapter_number,
        scene_order,
        summary,
        characters,
        relationships,
        locations,
        plot_threads,
    };
    let review_items = build_review_items(character_dossiers, &resume_snapshot);

    FinalStateDraft {
        resume_snapshot,
        review_items,
    }
}

fn build_character_snapshots(
    character_dossiers: &[ImportCharacterDossierSummary],
) -> Vec<ImportFinalCharacterStateSnapshot> {
    let mut snapshots = character_dossiers
        .iter()
        .map(|dossier| {
            let final_point = dossier.state_trajectory.last();
            let emotional_state = final_point
                .filter(|point| !point.emotional_state.is_empty())
                .map(|point| point.emotional_state.clone())
                .unwrap_or_else(|| dossier.emotional_profile.base_emotions.clone());
            let goals = final_point
                .filter(|point| !point.goals.is_empty())
                .map(|point| point.goals.clone())
                .unwrap_or_else(|| infer_goals_from_patterns(&dossier.decision_patterns));
            let status = final_point
                .filter(|point| !point.status.is_empty())
                .map(|point| point.status.clone())
                .unwrap_or_else(|| {
                    final_point
                        .map(|point| infer_status_from_summary(&point.summary))
                        .unwrap_or_default()
                });
            let mut notes = final_point
                .map(|point| vec![point.summary.clone()])
                .unwrap_or_default();
            if dossier.state_trajectory.len() < 2 {
                notes.push("ending state is inferred from limited on-page evidence".to_string());
            }
            let point_confidence = final_point.map(|point| point.confidence).unwrap_or(0.55);
            let confidence = ((dossier.confidence + point_confidence) / 2.0_f64).clamp(0.0, 0.95);

            ImportFinalCharacterStateSnapshot {
                cluster_id: dossier.cluster_id.clone(),
                canonical_name: dossier.canonical_name.clone(),
                emotional_state,
                goals,
                status,
                notes,
                confidence,
                confidence_level: confidence_level(confidence),
            }
        })
        .collect::<Vec<_>>();
    snapshots.sort_by_key(|snapshot| normalize_name(&snapshot.canonical_name));
    snapshots
}

fn build_relationship_snapshots(
    character_dossiers: &[ImportCharacterDossierSummary],
) -> Vec<ImportFinalRelationshipSnapshot> {
    character_dossiers
        .iter()
        .flat_map(|dossier| {
            dossier.relationship_inferences.iter().map(|inference| {
                let trust = trust_score(
                    inference.trust_signal.as_deref(),
                    inference.tension_signal.as_deref(),
                    &inference.summary,
                );
                let tension = tension_score(
                    inference.trust_signal.as_deref(),
                    inference.tension_signal.as_deref(),
                    &inference.summary,
                );
                let confidence =
                    ((dossier.confidence + inference.confidence) / 2.0_f64).clamp(0.0, 0.92);
                ImportFinalRelationshipSnapshot {
                    source_character_cluster_id: dossier.cluster_id.clone(),
                    target_character_cluster_id: inference.other_character_cluster_id.clone(),
                    relationship_type: relationship_type(trust, tension).to_string(),
                    trust,
                    tension,
                    summary: inference.summary.clone(),
                    confidence,
                    confidence_level: confidence_level(confidence),
                }
            })
        })
        .collect()
}

fn build_location_snapshots(
    world: Option<&ImportWorldDossierSummary>,
) -> Vec<ImportFinalLocationStateSnapshot> {
    let Some(world) = world else {
        return Vec::new();
    };

    world
        .locations
        .iter()
        .map(|location| {
            let controlling_faction = world
                .entities
                .iter()
                .find(|entity| {
                    matches!(
                        entity.entity_kind,
                        spindle_core::models::ImportEntityKind::Faction
                    ) && (shares_segment(&location.source_segment_ids, &entity.source_segment_ids)
                        || normalize_name(&location.summary)
                            .contains(&normalize_name(&entity.canonical_name)))
                })
                .map(|entity| entity.canonical_name.clone());
            let status = infer_location_status(&location.summary);
            let threat_level = infer_threat_level(&location.summary);
            let stability = infer_stability(&location.summary, threat_level.as_deref());
            let prosperity = infer_prosperity(&location.summary);
            let confidence = (location.confidence
                - if controlling_faction.is_none() {
                    0.05
                } else {
                    0.0
                })
            .clamp(0.0, 0.9);

            ImportFinalLocationStateSnapshot {
                location_name: location.name.clone(),
                controlling_faction,
                status,
                prosperity,
                stability,
                threat_level,
                sensory_details: extract_sensory_details(&location.summary),
                confidence,
                confidence_level: confidence_level(confidence),
            }
        })
        .collect()
}

fn build_plot_thread_snapshots(
    narrative: Option<&ImportNarrativeDossierSummary>,
) -> Vec<ImportPlotThreadSnapshot> {
    let Some(narrative) = narrative else {
        return Vec::new();
    };

    narrative
        .plot_lines
        .iter()
        .map(|plot_line| {
            let next_expected_beat = narrative
                .narrative_promises
                .iter()
                .find(|promise| {
                    promise.status != "paid_off" && plot_line_matches_promise(plot_line, promise)
                })
                .map(|promise| promise.description.clone())
                .or_else(|| {
                    narrative
                        .conflicts
                        .iter()
                        .find(|conflict| conflict.conflict_type == plot_line.plot_type)
                        .map(|conflict| conflict.stakes.clone())
                });
            let status = plot_line.status.clone().unwrap_or_else(|| {
                if next_expected_beat.is_some() {
                    "active".to_string()
                } else {
                    "introduced".to_string()
                }
            });

            ImportPlotThreadSnapshot {
                name: plot_line.name.clone(),
                status,
                next_expected_beat,
                source_cluster_ids: if plot_line.plot_type == "character" {
                    narrative
                        .arcs
                        .iter()
                        .map(|arc| arc.character_cluster_id.clone())
                        .take(3)
                        .collect()
                } else {
                    Vec::new()
                },
                confidence: plot_line.confidence,
                confidence_level: plot_line.confidence_level.clone(),
            }
        })
        .collect()
}

fn resume_position(segments: &[ImportSegment]) -> (i32, i32, Option<i32>) {
    segments
        .iter()
        .filter(|segment| segment.segment_type == "scene")
        .max_by_key(|segment| {
            (
                segment.book_number.unwrap_or(1),
                segment.chapter_number.unwrap_or(1),
                segment.scene_order.unwrap_or(0),
                segment.source_order,
            )
        })
        .map(|segment| {
            (
                segment.book_number.unwrap_or(1) as i32,
                segment.chapter_number.unwrap_or(1) as i32,
                segment.scene_order.map(|value| value as i32 + 1),
            )
        })
        .unwrap_or((1, 1, Some(1)))
}

fn build_resume_summary(
    book_number: i32,
    chapter_number: i32,
    scene_order: Option<i32>,
    characters: &[ImportFinalCharacterStateSnapshot],
    locations: &[ImportFinalLocationStateSnapshot],
    plot_threads: &[ImportPlotThreadSnapshot],
) -> String {
    let cast = characters
        .iter()
        .take(2)
        .map(|character| character.canonical_name.clone())
        .collect::<Vec<_>>();
    let active_threads = plot_threads
        .iter()
        .filter(|thread| thread.status != "paid_off" && thread.status != "resolved")
        .count();
    let location = locations
        .first()
        .map(|location| format!(" at {}", location.location_name))
        .unwrap_or_default();

    format!(
        "Resume after book {}, chapter {}, scene {}{}. {} closes the imported manuscript with {} active plot thread(s).",
        book_number,
        chapter_number,
        scene_order.unwrap_or(0),
        location,
        if cast.is_empty() {
            "The cast".to_string()
        } else {
            cast.join(" and ")
        },
        active_threads,
    )
}

fn build_review_items(
    character_dossiers: &[ImportCharacterDossierSummary],
    resume_snapshot: &ImportResumeSnapshotSummary,
) -> Vec<FinalStateReviewDraft> {
    let mut reviews = character_dossiers
        .iter()
        .filter(|dossier| dossier.state_trajectory.len() < 2 || dossier.confidence < 0.65)
        .map(|dossier| FinalStateReviewDraft {
            title: format!("Review final state for '{}'", dossier.canonical_name),
            description: "This ending state is inferred from limited trajectory evidence and should be validated before hydration.".to_string(),
            related_segment_ids: dossier
                .state_trajectory
                .last()
                .map(|point| vec![point.segment_id.clone()])
                .unwrap_or_default(),
            target_record_id: Some(dossier.cluster_id.clone()),
            corrected_summary: dossier
                .state_trajectory
                .last()
                .map(|point| point.summary.clone())
                .unwrap_or_else(|| format!("Confirm {}'s final state.", dossier.canonical_name)),
            confidence: dossier.confidence,
        })
        .collect::<Vec<_>>();

    let name_by_cluster = character_dossiers
        .iter()
        .map(|dossier| (dossier.cluster_id.clone(), dossier.canonical_name.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut pair_map = BTreeMap::<(String, String), Vec<&ImportFinalRelationshipSnapshot>>::new();
    for relationship in &resume_snapshot.relationships {
        let key = canonical_pair(
            &relationship.source_character_cluster_id,
            &relationship.target_character_cluster_id,
        );
        pair_map.entry(key).or_default().push(relationship);
    }
    for ((left_id, right_id), pair) in pair_map {
        let high_variance = pair
            .iter()
            .map(|relationship| relationship.trust)
            .max()
            .zip(pair.iter().map(|relationship| relationship.trust).min())
            .is_some_and(|(max, min)| max - min >= 25);
        let volatile = pair
            .iter()
            .any(|relationship| relationship.trust >= 65 && relationship.tension >= 65);
        if !high_variance && !volatile {
            continue;
        }
        let left_name = name_by_cluster.get(&left_id).cloned().unwrap_or(left_id);
        let right_name = name_by_cluster.get(&right_id).cloned().unwrap_or(right_id);
        reviews.push(FinalStateReviewDraft {
            title: format!("Review ending relationship between '{}' and '{}'", left_name, right_name),
            description: "Ending relationship signals point in different directions and should be resolved before hydration.".to_string(),
            related_segment_ids: Vec::new(),
            target_record_id: None,
            corrected_summary: format!(
                "Confirm whether {} and {} end the manuscript aligned, estranged, or unstable.",
                left_name, right_name
            ),
            confidence: pair
                .iter()
                .map(|relationship| relationship.confidence)
                .sum::<f64>()
                / pair.len() as f64,
        });
    }

    reviews.extend(
        resume_snapshot
            .plot_threads
            .iter()
            .filter(|thread| thread.status == "active" && thread.next_expected_beat.is_none())
            .map(|thread| FinalStateReviewDraft {
                title: format!("Review plot thread '{}'", thread.name),
                description: "This plot thread remains active, but the next expected beat is unclear from the imported ending state.".to_string(),
                related_segment_ids: Vec::new(),
                target_record_id: None,
                corrected_summary: format!("Clarify the next beat for '{}'.", thread.name),
                confidence: thread.confidence,
            }),
    );

    reviews
}

fn canonical_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_string(), right.to_string())
    } else {
        (right.to_string(), left.to_string())
    }
}

fn plot_line_matches_promise(
    plot_line: &spindle_core::models::ImportPlotLineCandidate,
    promise: &spindle_core::models::ImportNarrativePromiseCandidate,
) -> bool {
    let plot_name = normalize_name(&plot_line.name);
    let plot_summary = normalize_name(&plot_line.summary);
    let promise_description = normalize_name(&promise.description);
    let promise_type = normalize_name(&promise.promise_type);

    promise_description.contains(&plot_name)
        || plot_summary.contains(&promise_type)
        || (plot_line.plot_type == "mystery" && promise_type == "mystery")
        || (plot_line.plot_type == "character"
            && matches!(promise_type.as_str(), "quest" | "relationship"))
        || (plot_line.plot_type == "external" && promise_type == "threat")
}

fn trust_score(trust_signal: Option<&str>, tension_signal: Option<&str>, summary: &str) -> i32 {
    let trust_signal = trust_signal.unwrap_or_default().to_ascii_lowercase();
    let tension_signal = tension_signal.unwrap_or_default().to_ascii_lowercase();
    let summary = summary.to_ascii_lowercase();

    if trust_signal.contains("alliance") || trust_signal.contains("shared focus") {
        68
    } else if trust_signal.contains("trust") || trust_signal.contains("loyal") {
        78
    } else if tension_signal.contains("betrayal") || summary.contains("betrayal") {
        32
    } else {
        52
    }
}

fn tension_score(trust_signal: Option<&str>, tension_signal: Option<&str>, summary: &str) -> i32 {
    let trust_signal = trust_signal.unwrap_or_default().to_ascii_lowercase();
    let tension_signal = tension_signal.unwrap_or_default().to_ascii_lowercase();
    let summary = summary.to_ascii_lowercase();

    if tension_signal.contains("betrayal")
        || summary.contains("betrayal")
        || summary.contains("fire")
    {
        78
    } else if trust_signal.contains("shared focus") || trust_signal.contains("alliance") {
        32
    } else {
        48
    }
}

fn relationship_type(trust: i32, tension: i32) -> &'static str {
    if trust >= 65 && tension <= 40 {
        "allied"
    } else if trust <= 40 && tension >= 65 {
        "adversarial"
    } else if trust >= 55 && tension >= 55 {
        "fraught"
    } else {
        "uncertain"
    }
}

fn infer_goals_from_patterns(patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .filter_map(|pattern| {
            let lowered = pattern.to_ascii_lowercase();
            if lowered.contains("decisively") {
                Some("protect the immediate objective".to_string())
            } else if lowered.contains("more information") {
                Some("gather enough information before acting".to_string())
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn infer_status_from_summary(summary: &str) -> Vec<String> {
    let lowered = summary.to_ascii_lowercase();
    let mut status = Vec::new();
    if lowered.contains("guard") || lowered.contains("hold") {
        status.push("holding the line".to_string());
    }
    if lowered.contains("burn") || lowered.contains("smoke") {
        status.push("under pressure".to_string());
    }
    if lowered.contains("warn") {
        status.push("warning others".to_string());
    }
    status
}

fn shares_segment(left: &[String], right: &[String]) -> bool {
    let right = right.iter().collect::<BTreeSet<_>>();
    left.iter().any(|segment_id| right.contains(&segment_id))
}

fn infer_location_status(summary: &str) -> Option<String> {
    let lowered = summary.to_ascii_lowercase();
    if lowered.contains("burn") || lowered.contains("fire") {
        Some("damaged".to_string())
    } else if lowered.contains("occupied") || lowered.contains("siege") {
        Some("contested".to_string())
    } else if lowered.contains("market") || lowered.contains("trade") {
        Some("active".to_string())
    } else {
        None
    }
}

fn infer_threat_level(summary: &str) -> Option<String> {
    let lowered = summary.to_ascii_lowercase();
    if ["fire", "attack", "threat", "warning", "betrayal"]
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        Some("high".to_string())
    } else if ["uneasy", "fragile", "scarcity"]
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        Some("rising".to_string())
    } else {
        None
    }
}

fn infer_stability(summary: &str, threat_level: Option<&str>) -> Option<String> {
    let lowered = summary.to_ascii_lowercase();
    if matches!(threat_level, Some("high") | Some("rising")) {
        Some("fragile".to_string())
    } else if ["guard", "watch", "court", "law"]
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        Some("stable".to_string())
    } else {
        None
    }
}

fn infer_prosperity(summary: &str) -> Option<String> {
    let lowered = summary.to_ascii_lowercase();
    if ["tariff", "ration", "scarcity", "burn"]
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        Some("strained".to_string())
    } else if ["market", "trade", "harbor"]
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        Some("active".to_string())
    } else {
        None
    }
}

fn extract_sensory_details(summary: &str) -> Vec<String> {
    let lowered = summary.to_ascii_lowercase();
    let mut details = Vec::new();
    for (marker, detail) in [
        ("smoke", "smoke in the air"),
        ("fire", "heat and ash"),
        ("blood", "the metallic sting of blood"),
        ("market", "crowded market noise"),
        ("harbor", "salt and water"),
        ("archive", "dust and paper"),
    ] {
        if lowered.contains(marker) {
            details.push(detail.to_string());
        }
    }
    if details.is_empty() {
        details.push(summary.trim().to_string());
    }
    details
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
