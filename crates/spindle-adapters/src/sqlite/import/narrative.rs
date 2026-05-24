//! Narrative-analysis pass.
//!
//! Pure logic: walks normalized segment text plus consolidated mentions and
//! character dossiers to draft plot lines, conflicts, narrative promises,
//! arcs, themes, motifs, reader contract, and pacing hints. The service
//! layer persists the draft via `Repository::upsert_import_narrative_dossier`.
//! Ported from the SurrealDB-era `crate::import::narrative`; SQLite records
//! use plain `String` ids so `record_id_string(&x.id)` becomes `x.id.clone()`.

use std::collections::{BTreeMap, BTreeSet};

use spindle_core::models::{
    CharacterArcMilestone, ImportCharacterDossierSummary, ImportConfidenceLevel,
    ImportConflictCandidate, ImportMotifCandidate, ImportNarrativeDossierSummary,
    ImportNarrativePromiseCandidate, ImportPacingHint, ImportPlotLineCandidate,
    ImportReaderContractDraft, ImportThemeCandidate, ReaderContract, StatedConsequence,
    StoryPlacement, TryFailCycleStep,
};

use crate::sqlite::records::{ImportEntityMention, ImportSegment};

#[derive(Debug, Clone)]
pub struct NarrativeReviewDraft {
    pub title: String,
    pub description: String,
    pub related_segment_ids: Vec<String>,
    pub target_id: Option<String>,
    pub status: Option<String>,
    pub thematic_purpose: Option<String>,
    pub note: Option<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub struct NarrativeExtractionDraft {
    pub plot_lines: Vec<ImportPlotLineCandidate>,
    pub conflicts: Vec<ImportConflictCandidate>,
    pub narrative_promises: Vec<ImportNarrativePromiseCandidate>,
    pub arcs: Vec<spindle_core::models::ImportArcCandidate>,
    pub themes: Vec<ImportThemeCandidate>,
    pub motifs: Vec<ImportMotifCandidate>,
    pub reader_contract: ImportReaderContractDraft,
    pub pacing_hints: Vec<ImportPacingHint>,
    pub review_items: Vec<NarrativeReviewDraft>,
}

pub fn analyze_narrative_dossier(
    segments: &[ImportSegment],
    segment_texts: &BTreeMap<String, String>,
    mentions: &[ImportEntityMention],
    character_dossiers: &[ImportCharacterDossierSummary],
) -> NarrativeExtractionDraft {
    let scenes = scene_contexts(segments, segment_texts);
    let conflict_groups = build_conflict_groups(mentions, &scenes);
    let plot_lines = build_plot_lines(&conflict_groups);
    let conflicts = build_conflicts(&conflict_groups);
    let narrative_promises = build_promises(&conflict_groups);
    let corpus = scenes
        .iter()
        .map(|scene| scene.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let themes = build_themes(&corpus);
    let motifs = build_motifs(&corpus, &themes);
    let reader_contract = build_reader_contract(&corpus, &themes, &conflicts);
    let arcs = build_arcs(character_dossiers, &themes);
    let pacing_hints = build_pacing_hints(&scenes);
    let review_items = build_review_items(&narrative_promises, &themes, &arcs);

    NarrativeExtractionDraft {
        plot_lines,
        conflicts,
        narrative_promises,
        arcs,
        themes,
        motifs,
        reader_contract,
        pacing_hints,
        review_items,
    }
}

pub fn to_narrative_summary(draft: &NarrativeExtractionDraft) -> ImportNarrativeDossierSummary {
    ImportNarrativeDossierSummary {
        plot_lines: draft.plot_lines.clone(),
        conflicts: draft.conflicts.clone(),
        narrative_promises: draft.narrative_promises.clone(),
        arcs: draft.arcs.clone(),
        themes: draft.themes.clone(),
        motifs: draft.motifs.clone(),
        reader_contract: draft.reader_contract.clone(),
        pacing_hints: draft.pacing_hints.clone(),
    }
}

fn scene_contexts<'a>(
    segments: &'a [ImportSegment],
    segment_texts: &'a BTreeMap<String, String>,
) -> Vec<SceneContext<'a>> {
    let mut scenes = segments
        .iter()
        .filter(|segment| segment.segment_type == "scene")
        .filter_map(|segment| {
            let segment_id = segment.id.clone();
            Some(SceneContext {
                segment_id: segment_id.clone(),
                placement: placement_for_segment(segment),
                text: segment_texts.get(&segment_id)?,
            })
        })
        .collect::<Vec<_>>();
    scenes.sort_by_key(|scene| {
        (
            scene.placement.book_number,
            scene.placement.chapter_number,
            scene.placement.scene_order.unwrap_or(0),
        )
    });
    scenes
}

fn build_conflict_groups(
    mentions: &[ImportEntityMention],
    scenes: &[SceneContext<'_>],
) -> Vec<ConflictGroup> {
    let placements = scenes
        .iter()
        .map(|scene| (scene.segment_id.clone(), scene.placement.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut groups = BTreeMap::<String, ConflictGroup>::new();

    for mention in mentions
        .iter()
        .filter(|mention| mention.entity_kind == "conflict")
    {
        let segment_id = mention.segment_id.clone();
        let Some(placement) = placements.get(&segment_id) else {
            continue;
        };
        let key = mention.normalized_name.clone();
        let entry = groups.entry(key.clone()).or_insert_with(|| ConflictGroup {
            normalized_name: key.clone(),
            display_name: title_case(&mention.surface_form),
            mention_count: 0,
            placements: Vec::new(),
            segments: BTreeSet::new(),
            excerpts: Vec::new(),
            avg_confidence: 0.0,
        });
        entry.mention_count += 1;
        entry.avg_confidence += mention.confidence;
        entry.placements.push(placement.clone());
        entry.segments.insert(segment_id);
        if let Some(text) = mention.surrounding_text.as_ref() {
            entry.excerpts.push(text.clone());
        }
    }

    groups
        .into_values()
        .map(|mut group| {
            if group.mention_count > 0 {
                group.avg_confidence /= group.mention_count as f64;
            }
            group.placements.sort_by_key(|placement| {
                (
                    placement.book_number,
                    placement.chapter_number,
                    placement.scene_order.unwrap_or(0),
                )
            });
            group
        })
        .collect()
}

fn build_plot_lines(conflicts: &[ConflictGroup]) -> Vec<ImportPlotLineCandidate> {
    conflicts
        .iter()
        .map(|group| {
            let confidence = (group.avg_confidence
                + if group.mention_count >= 2 { 0.18 } else { 0.05 })
            .clamp(0.0, 0.92);
            ImportPlotLineCandidate {
                plot_line_id: None,
                name: plot_line_name(&group.normalized_name),
                plot_type: plot_type_for_conflict(&group.normalized_name).to_string(),
                summary: group
                    .excerpts
                    .first()
                    .map(|excerpt| summarize(excerpt, 110))
                    .unwrap_or_else(|| {
                        format!(
                            "The imported manuscript keeps returning to {}.",
                            group.display_name.to_ascii_lowercase()
                        )
                    }),
                status: Some(if group.mention_count >= 2 {
                    "active".to_string()
                } else {
                    "introduced".to_string()
                }),
                convergence_points: group.placements.iter().take(3).cloned().collect(),
                confidence,
                confidence_level: confidence_level(confidence),
            }
        })
        .collect()
}

fn build_conflicts(conflicts: &[ConflictGroup]) -> Vec<ImportConflictCandidate> {
    conflicts
        .iter()
        .map(|group| {
            let try_fail_cycles = group
                .placements
                .iter()
                .zip(
                    group
                        .excerpts
                        .iter()
                        .chain(std::iter::repeat(&group.display_name)),
                )
                .enumerate()
                .take(3)
                .map(|(index, (_placement, excerpt))| TryFailCycleStep {
                    attempt_order: index as i32 + 1,
                    label: format!("{} beat {}", group.display_name, index + 1),
                    outcome: summarize(excerpt, 96),
                    cost: (index > 0)
                        .then(|| "pressure escalates after the previous attempt".to_string()),
                    revelation: excerpt
                        .to_ascii_lowercase()
                        .contains("reveal")
                        .then(|| "new information changes the line of conflict".to_string()),
                })
                .collect::<Vec<_>>();
            let stated_consequences = group
                .excerpts
                .iter()
                .filter(|excerpt| consequence_markers(excerpt))
                .take(2)
                .map(|excerpt| StatedConsequence {
                    description: summarize(excerpt, 96),
                    stated_at: group.placements.first().cloned(),
                    must_demonstrate_by: None,
                    delivered: false,
                })
                .collect::<Vec<_>>();
            let confidence = (group.avg_confidence
                + if group.mention_count >= 2 { 0.2 } else { 0.04 })
            .clamp(0.0, 0.9);

            ImportConflictCandidate {
                conflict_id: None,
                name: group.display_name.clone(),
                conflict_type: plot_type_for_conflict(&group.normalized_name).to_string(),
                stakes: group
                    .excerpts
                    .first()
                    .map(|excerpt| summarize(excerpt, 120))
                    .unwrap_or_else(|| {
                        format!(
                            "{} threatens the current story direction.",
                            group.display_name
                        )
                    }),
                escalation_stages: group
                    .excerpts
                    .iter()
                    .take(3)
                    .map(|excerpt| summarize(excerpt, 90))
                    .collect(),
                try_fail_cycles,
                stated_consequences,
                confidence,
                confidence_level: confidence_level(confidence),
            }
        })
        .collect()
}

fn build_promises(conflicts: &[ConflictGroup]) -> Vec<ImportNarrativePromiseCandidate> {
    conflicts
        .iter()
        .filter_map(|group| {
            promise_type_for_conflict(&group.normalized_name).map(|promise_type| {
                let planned_payoff = (group.placements.len() >= 2)
                    .then(|| group.placements.last().cloned())
                    .flatten();
                let status = if planned_payoff.is_some() {
                    "paying_off"
                } else {
                    "active"
                };
                let confidence = (group.avg_confidence
                    + if matches!(group.normalized_name.as_str(), "promise" | "oath") {
                        0.22
                    } else if group.mention_count >= 2 {
                        0.14
                    } else {
                        0.02
                    })
                .clamp(0.0, 0.9);

                ImportNarrativePromiseCandidate {
                    narrative_promise_id: None,
                    promise_type: promise_type.to_string(),
                    description: group
                        .excerpts
                        .first()
                        .map(|excerpt| summarize(excerpt, 110))
                        .unwrap_or_else(|| format!("{} remains unresolved.", group.display_name)),
                    planted_at: group.placements.first().cloned().unwrap_or(StoryPlacement {
                        book_number: 1,
                        chapter_number: 1,
                        scene_order: Some(1),
                        note: None,
                    }),
                    planned_payoff,
                    status: status.to_string(),
                    notes: vec![format!(
                        "inferred from recurring {} language",
                        group.normalized_name
                    )],
                    confidence,
                    confidence_level: confidence_level(confidence),
                }
            })
        })
        .collect()
}

fn build_themes(corpus: &str) -> Vec<ImportThemeCandidate> {
    let seeds: [(&str, &str, &[&str]); 5] = [
        (
            "Duty demands sacrifice",
            "Duty can protect others, but it extracts a personal cost.",
            &[
                "duty",
                "oath",
                "swore",
                "watch",
                "blood",
                "cost",
                "sacrifice",
            ],
        ),
        (
            "Truth destabilizes power",
            "Hidden truths sustain power until revelation tears it open.",
            &["secret", "betrayal", "revelation", "truth", "expose", "lie"],
        ),
        (
            "Loyalty is tested under pressure",
            "Pressure reveals whether loyalty can hold against fear and self-interest.",
            &["loyalty", "betrayal", "trust", "warning", "guard", "watch"],
        ),
        (
            "Control over time comes at a human cost",
            "Attempts to control time promise advantage while deepening loss and instability.",
            &["timeline", "reset", "rewind", "future", "memory", "paradox"],
        ),
        (
            "Scarcity exposes social fracture",
            "Scarcity turns trade, hunger, and power into moral pressure points.",
            &["tariff", "ration", "coin", "market", "scarcity", "hunger"],
        ),
    ];

    let mut themes = seeds
        .into_iter()
        .filter_map(|(statement, thesis_antithesis, markers)| {
            let hits = count_markers(corpus, markers);
            (hits > 0).then(|| {
                let confidence = (0.54 + hits as f64 * 0.08).clamp(0.0, 0.9);
                ImportThemeCandidate {
                    theme_statement: statement.to_string(),
                    thesis_antithesis: thesis_antithesis.to_string(),
                    confidence,
                    confidence_level: confidence_level(confidence),
                }
            })
        })
        .collect::<Vec<_>>();
    themes.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    themes.truncate(3);
    themes
}

fn build_motifs(corpus: &str, themes: &[ImportThemeCandidate]) -> Vec<ImportMotifCandidate> {
    let top_themes = themes
        .iter()
        .map(|theme| theme.theme_statement.clone())
        .collect::<Vec<_>>();
    let motif_seeds = [
        ("blood", "Blood marks sacrifice, oath, and costly power."),
        (
            "gate",
            "Gates and thresholds signal pressure, defense, and transition.",
        ),
        ("fire", "Fire recurs as danger, warning, and purification."),
        (
            "screen",
            "Screens externalize system logic and surveillance.",
        ),
        ("archive", "Archives connect revelation to buried truth."),
        ("watch", "Watch imagery reinforces vigilance and duty."),
    ];

    motif_seeds
        .into_iter()
        .filter_map(|(name, description)| {
            let hits = corpus.matches(name).count();
            (hits >= 1).then(|| {
                let confidence = (0.5 + hits as f64 * 0.12).clamp(0.0, 0.88);
                ImportMotifCandidate {
                    name: title_case(name),
                    description: description.to_string(),
                    connected_theme_statements: top_themes.clone(),
                    confidence,
                    confidence_level: confidence_level(confidence),
                }
            })
        })
        .collect()
}

fn build_reader_contract(
    corpus: &str,
    themes: &[ImportThemeCandidate],
    conflicts: &[ImportConflictCandidate],
) -> ImportReaderContractDraft {
    let promise = if corpus.contains("timeline") || corpus.contains("status screen") {
        "A pressure-driven speculative story where unstable systems intensify human cost."
    } else if themes.iter().any(|theme| {
        theme.theme_statement.contains("sacrifice") || theme.theme_statement.contains("Loyalty")
    }) {
        "A tense story of duty, betrayal, and costly choices under relentless pressure."
    } else if let Some(conflict) = conflicts.first() {
        return ImportReaderContractDraft {
            reader_contract: ReaderContract {
                promise: format!(
                    "A character-driven story where {} keeps escalating the pressure.",
                    conflict.name.to_ascii_lowercase()
                ),
                style_notes: vec![
                    "Preserve forward momentum from scene to scene.".to_string(),
                    "Keep consequences visible in character choices.".to_string(),
                ],
                boundaries: infer_boundaries(corpus),
            },
            confidence: 0.68,
            confidence_level: confidence_level(0.68),
        };
    } else {
        "A character-driven story of pressure, consequence, and escalating choices."
    };

    let confidence = if themes.is_empty() { 0.62 } else { 0.78 };
    ImportReaderContractDraft {
        reader_contract: ReaderContract {
            promise: promise.to_string(),
            style_notes: vec![
                "Preserve forward momentum from scene to scene.".to_string(),
                "Keep revelations grounded in immediate consequences.".to_string(),
            ],
            boundaries: infer_boundaries(corpus),
        },
        confidence,
        confidence_level: confidence_level(confidence),
    }
}

fn build_arcs(
    dossiers: &[ImportCharacterDossierSummary],
    themes: &[ImportThemeCandidate],
) -> Vec<spindle_core::models::ImportArcCandidate> {
    let thematic_purpose = themes
        .first()
        .map(|theme| theme.theme_statement.clone())
        .unwrap_or_else(|| "Pressure reveals what each character will protect.".to_string());

    dossiers
        .iter()
        .take(4)
        .map(|dossier| {
            let arc_type = infer_arc_type(dossier, themes);
            let milestones = dossier
                .state_trajectory
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    *index == 0
                        || *index + 1 == dossier.state_trajectory.len()
                        || (*index == dossier.state_trajectory.len() / 2
                            && dossier.state_trajectory.len() > 2)
                })
                .map(|(index, point)| CharacterArcMilestone {
                    label: if index == 0 {
                        "opening pressure".to_string()
                    } else if index + 1 == dossier.state_trajectory.len() {
                        "current trajectory".to_string()
                    } else {
                        "turning point".to_string()
                    },
                    placement: point.placement.clone(),
                    description: summarize(&point.summary, 96),
                    unlocks: themes
                        .first()
                        .map(|theme| vec![theme.theme_statement.clone()])
                        .unwrap_or_default(),
                })
                .collect::<Vec<_>>();
            let starting_state = dossier
                .state_trajectory
                .first()
                .map(|point| summarize(&point.summary, 72))
                .unwrap_or_else(|| {
                    format!(
                        "{} enters the story under pressure.",
                        dossier.canonical_name
                    )
                });
            let ending_state = dossier
                .state_trajectory
                .last()
                .map(|point| summarize(&point.summary, 72))
                .unwrap_or_else(|| {
                    format!(
                        "{} remains unresolved at import end.",
                        dossier.canonical_name
                    )
                });
            let confidence = (dossier.confidence
                - if dossier.state_trajectory.len() < 2 {
                    0.12
                } else {
                    0.0
                })
            .clamp(0.0, 0.9);

            spindle_core::models::ImportArcCandidate {
                character_cluster_id: dossier.cluster_id.clone(),
                arc_type,
                starting_state,
                ending_state,
                milestones,
                thematic_purpose: thematic_purpose.clone(),
                confidence,
                confidence_level: confidence_level(confidence),
            }
        })
        .collect()
}

fn build_pacing_hints(scenes: &[SceneContext<'_>]) -> Vec<ImportPacingHint> {
    let mut by_book = BTreeMap::<i32, Vec<&SceneContext<'_>>>::new();
    for scene in scenes {
        by_book
            .entry(scene.placement.book_number)
            .or_default()
            .push(scene);
    }

    by_book
        .into_iter()
        .map(|(book_number, book_scenes)| {
            let total = book_scenes.len().max(1) as f64;
            let action = book_scenes
                .iter()
                .filter(|scene| {
                    count_markers(
                        &scene.text.to_ascii_lowercase(),
                        &["attack", "battle", "fire", "escape"],
                    ) > 0
                })
                .count() as f64;
            let revelation = book_scenes
                .iter()
                .filter(|scene| {
                    count_markers(
                        &scene.text.to_ascii_lowercase(),
                        &["secret", "betrayal", "revelation", "truth"],
                    ) > 0
                })
                .count() as f64;
            let dialogue = book_scenes
                .iter()
                .filter(|scene| scene.text.contains('"') || scene.text.contains(" said "))
                .count() as f64;
            let density = BTreeMap::from([
                ("action".to_string(), action / total),
                ("revelation".to_string(), revelation / total),
                ("dialogue".to_string(), dialogue / total),
            ]);

            ImportPacingHint {
                book_number: Some(book_number),
                summary: if action >= revelation {
                    "The imported pacing leans on forward pressure and visible external escalation."
                        .to_string()
                } else {
                    "The imported pacing leans on revelation beats and accumulating tension."
                        .to_string()
                },
                act_breakpoints: BTreeMap::from([
                    ("act_1_end".to_string(), 0.25),
                    ("midpoint".to_string(), 0.5),
                    ("act_2_end".to_string(), 0.75),
                ]),
                scene_type_density: density,
                confidence: 0.72,
                confidence_level: confidence_level(0.72),
            }
        })
        .collect()
}

fn build_review_items(
    promises: &[ImportNarrativePromiseCandidate],
    themes: &[ImportThemeCandidate],
    arcs: &[spindle_core::models::ImportArcCandidate],
) -> Vec<NarrativeReviewDraft> {
    let mut reviews = promises
        .iter()
        .filter(|promise| promise.confidence < 0.65)
        .map(|promise| NarrativeReviewDraft {
            title: format!("Review narrative promise '{}'", promise.description),
            description: "This promise was inferred from weak foreshadowing language and should be confirmed before hydration.".to_string(),
            related_segment_ids: Vec::new(),
            target_id: promise.narrative_promise_id.clone(),
            status: Some(promise.status.clone()),
            thematic_purpose: None,
            note: Some(promise.notes.join(" ")),
            confidence: promise.confidence,
        })
        .collect::<Vec<_>>();

    reviews.extend(themes.iter().filter(|theme| theme.confidence < 0.7).map(|theme| NarrativeReviewDraft {
        title: format!("Review theme '{}'", theme.theme_statement),
        description: "This theme statement was inferred from clustered language and should be checked before canonization.".to_string(),
        related_segment_ids: Vec::new(),
        target_id: None,
        status: None,
        thematic_purpose: Some(theme.theme_statement.clone()),
        note: Some(theme.thesis_antithesis.clone()),
        confidence: theme.confidence,
    }));

    reviews.extend(arcs.iter().filter(|arc| arc.confidence < 0.7).map(|arc| NarrativeReviewDraft {
        title: format!("Review character arc for {}", arc.character_cluster_id),
        description: "This arc type was inferred from limited trajectory evidence and should be reviewed before hydration.".to_string(),
        related_segment_ids: Vec::new(),
        target_id: Some(arc.character_cluster_id.clone()),
        status: None,
        thematic_purpose: Some(arc.thematic_purpose.clone()),
        note: Some(arc.arc_type.clone()),
        confidence: arc.confidence,
    }));

    reviews
}

fn placement_for_segment(segment: &ImportSegment) -> StoryPlacement {
    StoryPlacement {
        book_number: segment.book_number.unwrap_or(1) as i32,
        chapter_number: segment.chapter_number.unwrap_or(1) as i32,
        scene_order: segment.scene_order.map(|value| value as i32),
        note: segment.label.clone(),
    }
}

fn promise_type_for_conflict(normalized: &str) -> Option<&'static str> {
    match normalized {
        "warning" | "attack" | "fire" | "escape" => Some("threat"),
        "betrayal" | "argument" => Some("relationship"),
        "secret" | "revelation" => Some("mystery"),
        "promise" | "oath" => Some("quest"),
        _ => None,
    }
}

fn plot_type_for_conflict(normalized: &str) -> &'static str {
    match normalized {
        "warning" | "attack" | "fire" | "escape" => "external",
        "betrayal" | "argument" => "interpersonal",
        "secret" | "revelation" => "mystery",
        "promise" | "oath" => "character",
        _ => "external",
    }
}

fn plot_line_name(normalized: &str) -> String {
    match normalized {
        "warning" => "Answer the warning".to_string(),
        "betrayal" => "Unmask the betrayal".to_string(),
        "secret" => "Expose the secret".to_string(),
        "fire" => "Contain the fire".to_string(),
        "escape" => "Stage the escape".to_string(),
        other => format!("Track {}", title_case(other)),
    }
}

fn infer_arc_type(
    dossier: &ImportCharacterDossierSummary,
    themes: &[ImportThemeCandidate],
) -> String {
    let triggers = dossier
        .emotional_profile
        .triggers
        .join(" ")
        .to_ascii_lowercase();
    let defenses = dossier
        .emotional_profile
        .defense_mechanisms
        .join(" ")
        .to_ascii_lowercase();
    let decisions = dossier.decision_patterns.join(" ").to_ascii_lowercase();

    if triggers.contains("betrayal")
        || themes
            .iter()
            .any(|theme| theme.theme_statement.contains("Truth"))
    {
        "disillusionment".to_string()
    } else if defenses.contains("discipline")
        || defenses.contains("control")
        || decisions.contains("decisively")
    {
        "duty".to_string()
    } else if themes
        .iter()
        .any(|theme| theme.theme_statement.contains("time"))
    {
        "adaptation".to_string()
    } else {
        "growth".to_string()
    }
}

fn infer_boundaries(corpus: &str) -> Vec<String> {
    let mut boundaries = Vec::new();
    if count_markers(corpus, &["blood", "murder", "fire", "wound"]) > 0 {
        boundaries.push("Violence is present on-page.".to_string());
    }
    boundaries
}

fn consequence_markers(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    [" if ", " before ", " will ", " or ", " must "]
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn count_markers(corpus: &str, markers: &[&str]) -> usize {
    markers
        .iter()
        .filter(|marker| corpus.contains(**marker))
        .count()
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

fn title_case(value: &str) -> String {
    value
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    format!(
                        "{}{}",
                        first.to_ascii_uppercase(),
                        chars.as_str().to_ascii_lowercase()
                    )
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn summarize(input: &str, max_chars: usize) -> String {
    let trimmed = input.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let shortened = trimmed
            .chars()
            .take(max_chars.saturating_sub(3))
            .collect::<String>();
        format!("{}...", shortened)
    }
}

struct SceneContext<'a> {
    segment_id: String,
    placement: StoryPlacement,
    text: &'a String,
}

struct ConflictGroup {
    normalized_name: String,
    display_name: String,
    mention_count: usize,
    placements: Vec<StoryPlacement>,
    #[allow(dead_code)]
    segments: BTreeSet<String>,
    excerpts: Vec<String>,
    avg_confidence: f64,
}
