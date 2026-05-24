use crate::models::{EstablishedIn, StoryPlacement};
use crate::provenance::{Provenance, RecordId};
use crate::subject::{Subject, SubjectTable};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RenderDepth {
    Minimal,
    Standard,
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SubjectSnapshot {
    subject: Subject,
    display_name: String,
    kind_specific: SubjectKindSpecific,
    #[serde(default)]
    canonical_facts: Vec<CanonicalFactSummary>,
    #[serde(default)]
    knowledge: Vec<KnowledgeFactSummary>,
    #[serde(default)]
    relationships: Vec<RelationshipSummary>,
    #[serde(default)]
    open_promises: Vec<NarrativePromiseSummary>,
    #[serde(default)]
    active_arcs: Vec<CharacterArcSummary>,
    #[serde(default)]
    recent_appearances: Vec<SceneAppearanceSummary>,
    voice_profile: Option<VoiceProfileSummary>,
    current_state: Option<CharacterStateSummary>,
    at_placement: StoryPlacement,
    provenance: Provenance,
}

impl SubjectSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        subject: Subject,
        display_name: String,
        kind_specific: SubjectKindSpecific,
        canonical_facts: Vec<CanonicalFactSummary>,
        knowledge: Vec<KnowledgeFactSummary>,
        relationships: Vec<RelationshipSummary>,
        open_promises: Vec<NarrativePromiseSummary>,
        active_arcs: Vec<CharacterArcSummary>,
        recent_appearances: Vec<SceneAppearanceSummary>,
        voice_profile: Option<VoiceProfileSummary>,
        current_state: Option<CharacterStateSummary>,
        at_placement: StoryPlacement,
        provenance: Provenance,
    ) -> Result<Self, SubjectSnapshotError> {
        let snapshot = Self {
            subject,
            display_name,
            kind_specific,
            canonical_facts,
            knowledge,
            relationships,
            open_promises,
            active_arcs,
            recent_appearances,
            voice_profile,
            current_state,
            at_placement,
            provenance,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn kind_specific(&self) -> &SubjectKindSpecific {
        &self.kind_specific
    }

    pub fn canonical_facts(&self) -> &[CanonicalFactSummary] {
        &self.canonical_facts
    }

    pub fn knowledge(&self) -> &[KnowledgeFactSummary] {
        &self.knowledge
    }

    pub fn relationships(&self) -> &[RelationshipSummary] {
        &self.relationships
    }

    pub fn open_promises(&self) -> &[NarrativePromiseSummary] {
        &self.open_promises
    }

    pub fn active_arcs(&self) -> &[CharacterArcSummary] {
        &self.active_arcs
    }

    pub fn recent_appearances(&self) -> &[SceneAppearanceSummary] {
        &self.recent_appearances
    }

    pub fn voice_profile(&self) -> Option<&VoiceProfileSummary> {
        self.voice_profile.as_ref()
    }

    pub fn current_state(&self) -> Option<&CharacterStateSummary> {
        self.current_state.as_ref()
    }

    pub fn at_placement(&self) -> &StoryPlacement {
        &self.at_placement
    }

    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    pub fn validate(&self) -> Result<(), SubjectSnapshotError> {
        let actual_table = self.subject.table();
        if let Some(expected_table) = self.kind_specific.subject_table() {
            if actual_table != expected_table {
                return Err(SubjectSnapshotError::SubjectKindMismatch {
                    subject_table: actual_table,
                    kind: self.kind_specific.variant_name().to_string(),
                });
            }
        } else if !matches!(
            actual_table,
            SubjectTable::Project
                | SubjectTable::Scene
                | SubjectTable::Chapter
                | SubjectTable::Book
        ) {
            return Err(SubjectSnapshotError::GenericKindNotAllowed {
                subject_table: actual_table,
            });
        }

        let is_character = matches!(self.kind_specific, SubjectKindSpecific::Character(_));
        if !is_character {
            if self.voice_profile.is_some() {
                return Err(SubjectSnapshotError::CharacterOnlyField {
                    field: "voice_profile",
                    kind: self.kind_specific.variant_name().to_string(),
                });
            }
            if self.current_state.is_some() {
                return Err(SubjectSnapshotError::CharacterOnlyField {
                    field: "current_state",
                    kind: self.kind_specific.variant_name().to_string(),
                });
            }
        }

        Ok(())
    }

    pub fn render_markdown(&self, depth: RenderDepth) -> String {
        let mut sections = vec![
            format!("# {}", self.display_name),
            format!("- Subject: `{}`", self.subject),
            format!("- Table: `{}`", self.subject.table()),
            format!(
                "- Placement: {}",
                format_story_placement(&self.at_placement)
            ),
            format!("- Provenance: {}", format_provenance(&self.provenance)),
            format!("- Snapshot kind: {}", self.kind_specific.variant_name()),
        ];

        match depth {
            RenderDepth::Minimal => {
                if let Some(line) = self.kind_specific.concise_line() {
                    sections.push(format!("- Summary: {line}"));
                }
                sections.push(format!(
                    "- Signals: {}, {} knowledge items, {} relationships, {} open promises, {} active arcs, {} appearances",
                    pluralize(self.canonical_facts.len(), "fact", "facts"),
                    self.knowledge.len(),
                    self.relationships.len(),
                    self.open_promises.len(),
                    self.active_arcs.len(),
                    self.recent_appearances.len()
                ));
            }
            RenderDepth::Standard => {
                if let Some(lines) = self.kind_specific.markdown_lines() {
                    sections.push("## Details".to_string());
                    sections.extend(lines.into_iter().map(|line| format!("- {line}")));
                }

                push_summary_preview(
                    &mut sections,
                    "Canonical Facts",
                    self.canonical_facts
                        .iter()
                        .map(CanonicalFactSummary::preview),
                );
                push_summary_preview(
                    &mut sections,
                    "Knowledge",
                    self.knowledge.iter().map(KnowledgeFactSummary::preview),
                );
                push_summary_preview(
                    &mut sections,
                    "Relationships",
                    self.relationships.iter().map(RelationshipSummary::preview),
                );
                push_summary_preview(
                    &mut sections,
                    "Open Promises",
                    self.open_promises
                        .iter()
                        .map(NarrativePromiseSummary::preview),
                );
                push_summary_preview(
                    &mut sections,
                    "Active Arcs",
                    self.active_arcs.iter().map(CharacterArcSummary::preview),
                );
                push_summary_preview(
                    &mut sections,
                    "Recent Appearances",
                    self.recent_appearances
                        .iter()
                        .map(SceneAppearanceSummary::preview),
                );

                if let Some(voice_profile) = &self.voice_profile {
                    sections.push("## Voice Profile".to_string());
                    sections.push(format!("- {}", voice_profile.preview()));
                }
                if let Some(current_state) = &self.current_state {
                    sections.push("## Current State".to_string());
                    sections.push(format!("- {}", current_state.preview()));
                }
            }
            RenderDepth::Full => {
                sections.push("## Snapshot JSON".to_string());
                let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string());
                sections.push("```json".to_string());
                sections.push(json);
                sections.push("```".to_string());
            }
        }

        sections.join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectSnapshotError {
    SubjectKindMismatch {
        subject_table: SubjectTable,
        kind: String,
    },
    GenericKindNotAllowed {
        subject_table: SubjectTable,
    },
    CharacterOnlyField {
        field: &'static str,
        kind: String,
    },
}

impl fmt::Display for SubjectSnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubjectSnapshotError::SubjectKindMismatch {
                subject_table,
                kind,
            } => write!(
                f,
                "subject table `{subject_table}` does not match snapshot kind `{kind}`"
            ),
            SubjectSnapshotError::GenericKindNotAllowed { subject_table } => write!(
                f,
                "generic snapshot kind is not allowed for supported table `{subject_table}`"
            ),
            SubjectSnapshotError::CharacterOnlyField { field, kind } => write!(
                f,
                "field `{field}` is only valid for character snapshots, not `{kind}`"
            ),
        }
    }
}

impl std::error::Error for SubjectSnapshotError {}

impl<'de> Deserialize<'de> for SubjectSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SubjectSnapshotRepr {
            subject: Subject,
            display_name: String,
            kind_specific: SubjectKindSpecific,
            #[serde(default)]
            canonical_facts: Vec<CanonicalFactSummary>,
            #[serde(default)]
            knowledge: Vec<KnowledgeFactSummary>,
            #[serde(default)]
            relationships: Vec<RelationshipSummary>,
            #[serde(default)]
            open_promises: Vec<NarrativePromiseSummary>,
            #[serde(default)]
            active_arcs: Vec<CharacterArcSummary>,
            #[serde(default)]
            recent_appearances: Vec<SceneAppearanceSummary>,
            voice_profile: Option<VoiceProfileSummary>,
            current_state: Option<CharacterStateSummary>,
            at_placement: StoryPlacement,
            provenance: Provenance,
        }

        let repr = SubjectSnapshotRepr::deserialize(deserializer)?;
        SubjectSnapshot::new(
            repr.subject,
            repr.display_name,
            repr.kind_specific,
            repr.canonical_facts,
            repr.knowledge,
            repr.relationships,
            repr.open_promises,
            repr.active_arcs,
            repr.recent_appearances,
            repr.voice_profile,
            repr.current_state,
            repr.at_placement,
            repr.provenance,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn push_summary_preview(
    sections: &mut Vec<String>,
    title: &str,
    previews: impl Iterator<Item = String>,
) {
    let items: Vec<String> = previews.collect();
    if items.is_empty() {
        return;
    }

    sections.push(format!("## {title}"));
    sections.extend(items.into_iter().map(|item| format!("- {item}")));
}

fn pluralize(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

fn format_story_placement(placement: &StoryPlacement) -> String {
    let base = match placement.scene_order {
        Some(scene_order) => format!(
            "book {}, chapter {}, scene {}",
            placement.book_number, placement.chapter_number, scene_order
        ),
        None => format!(
            "book {}, chapter {}",
            placement.book_number, placement.chapter_number
        ),
    };

    match &placement.note {
        Some(note) => format!("{base} (note: {note})"),
        None => base,
    }
}

fn format_provenance(provenance: &Provenance) -> String {
    const MAX_DERIVED_DEPTH: usize = 32;
    let mut current = provenance;
    let mut derived_by = Vec::new();

    loop {
        match current {
            Provenance::Derived { from, by } if derived_by.len() < MAX_DERIVED_DEPTH => {
                derived_by.push(by.as_str());
                current = from;
            }
            Provenance::Derived { .. } => {
                let chain = derived_by.join(" <- ");
                return format!("derived via {chain} <- <truncated>");
            }
            _ => break,
        }
    }

    let base = match current {
        Provenance::Scene {
            scene_id,
            byte_range,
        } => match byte_range {
            Some(range) => format!("{scene_id} bytes {}..{}", range.start, range.end),
            None => scene_id.to_string(),
        },
        Provenance::Chapter { chapter_id } => chapter_id.to_string(),
        Provenance::Book { book_id } => book_id.to_string(),
        Provenance::File { path, byte_range } => match byte_range {
            Some(range) => format!("{path} bytes {}..{}", range.start, range.end),
            None => path.clone(),
        },
        Provenance::AssertedByAuthor { at } => format!("asserted by author at {at}"),
        Provenance::Imported { source_path, at } => format!("imported from {source_path} at {at}"),
        Provenance::Derived { .. } => unreachable!("derived chains are handled iteratively"),
    };

    if derived_by.is_empty() {
        base
    } else {
        format!("derived via {} from {base}", derived_by.join(" <- "))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "details", rename_all = "snake_case")]
pub enum SubjectKindSpecific {
    WorldRule(WorldRuleDetails),
    Character(CharacterDetails),
    Location(LocationDetails),
    Faction(FactionDetails),
    Religion(ReligionDetails),
    Economy(EconomyDetails),
    PlotLine(PlotLineDetails),
    Conflict(ConflictDetails),
    Theme(ThemeDetails),
    Motif(MotifDetails),
    SystemOverlay(SystemOverlayDetails),
    NarrativePromise(NarrativePromiseDetails),
    CharacterArc(CharacterArcDetails),
    Term(TermDetails),
    Relationship(RelationshipDetails),
    TimelineEvent(TimelineEventDetails),
    Generic(Value),
}

impl SubjectKindSpecific {
    pub fn variant_name(&self) -> &'static str {
        match self {
            SubjectKindSpecific::WorldRule(_) => "world_rule",
            SubjectKindSpecific::Character(_) => "character",
            SubjectKindSpecific::Location(_) => "location",
            SubjectKindSpecific::Faction(_) => "faction",
            SubjectKindSpecific::Religion(_) => "religion",
            SubjectKindSpecific::Economy(_) => "economy",
            SubjectKindSpecific::PlotLine(_) => "plot_line",
            SubjectKindSpecific::Conflict(_) => "conflict",
            SubjectKindSpecific::Theme(_) => "theme",
            SubjectKindSpecific::Motif(_) => "motif",
            SubjectKindSpecific::SystemOverlay(_) => "system_overlay",
            SubjectKindSpecific::NarrativePromise(_) => "narrative_promise",
            SubjectKindSpecific::CharacterArc(_) => "character_arc",
            SubjectKindSpecific::Term(_) => "term",
            SubjectKindSpecific::Relationship(_) => "relationship",
            SubjectKindSpecific::TimelineEvent(_) => "timeline_event",
            SubjectKindSpecific::Generic(_) => "generic",
        }
    }

    pub fn subject_table(&self) -> Option<SubjectTable> {
        match self {
            SubjectKindSpecific::WorldRule(_) => Some(SubjectTable::WorldRule),
            SubjectKindSpecific::Character(_) => Some(SubjectTable::Character),
            SubjectKindSpecific::Location(_) => Some(SubjectTable::Location),
            SubjectKindSpecific::Faction(_) => Some(SubjectTable::Faction),
            SubjectKindSpecific::Religion(_) => Some(SubjectTable::Religion),
            SubjectKindSpecific::Economy(_) => Some(SubjectTable::Economy),
            SubjectKindSpecific::PlotLine(_) => Some(SubjectTable::PlotLine),
            SubjectKindSpecific::Conflict(_) => Some(SubjectTable::Conflict),
            SubjectKindSpecific::Theme(_) => Some(SubjectTable::Theme),
            SubjectKindSpecific::Motif(_) => Some(SubjectTable::Motif),
            SubjectKindSpecific::SystemOverlay(_) => Some(SubjectTable::SystemOverlay),
            SubjectKindSpecific::NarrativePromise(_) => Some(SubjectTable::NarrativePromise),
            SubjectKindSpecific::CharacterArc(_) => Some(SubjectTable::CharacterArc),
            SubjectKindSpecific::Term(_) => Some(SubjectTable::Term),
            SubjectKindSpecific::Relationship(_) => Some(SubjectTable::Relationship),
            SubjectKindSpecific::TimelineEvent(_) => Some(SubjectTable::TimelineEvent),
            SubjectKindSpecific::Generic(_) => None,
        }
    }

    fn concise_line(&self) -> Option<String> {
        self.markdown_lines()
            .and_then(|lines| lines.into_iter().next())
    }

    fn markdown_lines(&self) -> Option<Vec<String>> {
        match self {
            SubjectKindSpecific::WorldRule(details) => Some(compact_lines(vec![
                Some(format!("Type: {}", details.rule_type)),
                Some(format!("Description: {}", details.description)),
                details
                    .scan_pattern
                    .as_ref()
                    .map(|pattern| format!("Scan pattern: {pattern}")),
                if details.relevance_tags.is_empty() {
                    None
                } else {
                    Some(format!(
                        "Relevance tags: {}",
                        details.relevance_tags.join(", ")
                    ))
                },
                details.established_in.as_ref().map(|placement| {
                    format!("Established in: {}", format_established_in(placement))
                }),
            ])),
            SubjectKindSpecific::Character(details) => Some(compact_lines(vec![
                Some(format!("Role: {}", details.role)),
                Some(format!("Summary: {}", details.summary)),
                details
                    .realm
                    .as_ref()
                    .map(|realm| format!("Realm: {realm}")),
            ])),
            SubjectKindSpecific::Location(details) => Some(compact_lines(vec![
                Some(format!("Kind: {}", details.kind)),
                Some(format!("Summary: {}", details.summary)),
                details
                    .realm
                    .as_ref()
                    .map(|realm| format!("Realm: {realm}")),
                details
                    .controlling_faction
                    .as_ref()
                    .map(|faction| format!("Controlling faction: {faction}")),
                details
                    .status
                    .as_ref()
                    .map(|status| format!("Status: {status}")),
            ])),
            SubjectKindSpecific::Faction(details) => Some(compact_lines(vec![
                Some(format!("Category: {}", details.category)),
                Some(format!("Summary: {}", details.summary)),
                details
                    .sphere_of_influence
                    .as_ref()
                    .map(|value| format!("Sphere of influence: {value}")),
                if details.goals.is_empty() {
                    None
                } else {
                    Some(format!("Goals: {}", details.goals.join(", ")))
                },
            ])),
            SubjectKindSpecific::Religion(details) => Some(compact_lines(vec![
                Some(format!("Domain: {}", details.domain)),
                Some(format!("Summary: {}", details.summary)),
                if details.core_beliefs.is_empty() {
                    None
                } else {
                    Some(format!("Core beliefs: {}", details.core_beliefs.join(", ")))
                },
                if details.practices.is_empty() {
                    None
                } else {
                    Some(format!("Practices: {}", details.practices.join(", ")))
                },
            ])),
            SubjectKindSpecific::Economy(details) => Some(compact_lines(vec![
                Some(format!("System: {}", details.system)),
                Some(format!("Summary: {}", details.summary)),
                details
                    .currency
                    .as_ref()
                    .map(|currency| format!("Currency: {currency}")),
                if details.trade_goods.is_empty() {
                    None
                } else {
                    Some(format!("Trade goods: {}", details.trade_goods.join(", ")))
                },
            ])),
            SubjectKindSpecific::PlotLine(details) => Some(compact_lines(vec![
                Some(format!("Type: {}", details.plot_type)),
                Some(format!("Summary: {}", details.summary)),
                details
                    .status
                    .as_ref()
                    .map(|status| format!("Status: {status}")),
            ])),
            SubjectKindSpecific::Conflict(details) => Some(compact_lines(vec![
                Some(format!("Type: {}", details.conflict_type)),
                Some(format!("Stakes: {}", details.stakes)),
                details
                    .status
                    .as_ref()
                    .map(|status| format!("Status: {status}")),
                details
                    .escalation_stage
                    .as_ref()
                    .map(|stage| format!("Escalation stage: {stage}")),
            ])),
            SubjectKindSpecific::Theme(details) => Some(compact_lines(vec![
                Some(format!("Statement: {}", details.statement)),
                Some(format!("Thesis/antithesis: {}", details.thesis_antithesis)),
                details
                    .status
                    .as_ref()
                    .map(|status| format!("Status: {status}")),
            ])),
            SubjectKindSpecific::Motif(details) => Some(compact_lines(vec![
                Some(format!("Name: {}", details.name)),
                Some(format!("Description: {}", details.description)),
                if details.thematic_links.is_empty() {
                    None
                } else {
                    Some(format!(
                        "Thematic links: {}",
                        details.thematic_links.join(", ")
                    ))
                },
            ])),
            SubjectKindSpecific::SystemOverlay(details) => Some(compact_lines(vec![
                Some(format!("System: {}", details.system_name)),
                Some(format!("Type: {}", details.system_type)),
                Some(format!("Visibility: {}", details.visibility)),
                Some(format!("Rules: {}", details.rules)),
                if details.stats.is_empty() {
                    None
                } else {
                    Some(format!("Stats: {}", details.stats.join(", ")))
                },
            ])),
            SubjectKindSpecific::NarrativePromise(details) => Some(compact_lines(vec![
                Some(format!("Type: {}", details.promise_type)),
                Some(format!("Description: {}", details.description)),
                Some(format!("Status: {}", details.status)),
                details
                    .planned_payoff
                    .as_ref()
                    .map(|payoff| format!("Planned payoff: {}", format_story_placement(payoff))),
            ])),
            SubjectKindSpecific::CharacterArc(details) => Some(compact_lines(vec![
                Some(format!("Arc type: {}", details.arc_type)),
                Some(format!("Starting state: {}", details.starting_state)),
                Some(format!("Current state: {}", details.current_state)),
                Some(format!("Target state: {}", details.target_state)),
                Some(format!("Thematic purpose: {}", details.thematic_purpose)),
            ])),
            SubjectKindSpecific::Term(details) => Some(compact_lines(vec![
                Some(format!("Term: {}", details.term_text)),
                Some(format!("Definition: {}", details.definition)),
                details
                    .pronunciation
                    .as_ref()
                    .map(|value| format!("Pronunciation: {value}")),
                details
                    .usage_context
                    .as_ref()
                    .map(|value| format!("Usage context: {value}")),
                details
                    .origin
                    .as_ref()
                    .map(|value| format!("Origin: {value}")),
            ])),
            SubjectKindSpecific::Relationship(details) => Some(compact_lines(vec![
                Some(format!("Type: {}", details.relationship_type)),
                Some(format!("Summary: {}", details.summary)),
                Some(format!(
                    "Endpoints: {} -> {}",
                    details.source.display_name, details.target.display_name
                )),
                details.trust.map(|trust| format!("Trust: {trust}")),
                details.tension.map(|tension| format!("Tension: {tension}")),
            ])),
            SubjectKindSpecific::TimelineEvent(details) => Some(compact_lines(vec![
                Some(format!("Title: {}", details.title)),
                Some(format!("Type: {}", details.event_type)),
                Some(format!(
                    "Placement: {}",
                    format_story_placement(&details.placement)
                )),
                Some(format!("Summary: {}", details.summary)),
                if details.related_subjects.is_empty() {
                    None
                } else {
                    Some(format!(
                        "Related subjects: {}",
                        details
                            .related_subjects
                            .iter()
                            .map(|subject| subject.display_name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                },
            ])),
            SubjectKindSpecific::Generic(value) => serde_json::to_string(value)
                .ok()
                .map(|json| vec![format!("JSON: {json}")]),
        }
        .map(|lines| lines.into_iter().filter(|line| !line.is_empty()).collect())
    }
}

fn compact_lines(lines: Vec<Option<String>>) -> Vec<String> {
    lines.into_iter().flatten().collect()
}

fn format_established_in(established_in: &EstablishedIn) -> String {
    let mut rendered = format!(
        "Book {}, Chapter {}",
        established_in.book_number, established_in.chapter_number
    );
    if let Some(note) = established_in
        .note
        .as_ref()
        .filter(|note| !note.trim().is_empty())
    {
        rendered.push_str(" (");
        rendered.push_str(note);
        rendered.push(')');
    }
    rendered
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SubjectLinkSummary {
    pub subject: Subject,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorldRuleDetails {
    pub rule_type: String,
    pub description: String,
    pub scan_pattern: Option<String>,
    #[serde(default)]
    pub relevance_tags: Vec<String>,
    pub established_in: Option<EstablishedIn>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CharacterDetails {
    pub role: String,
    pub summary: String,
    pub realm: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LocationDetails {
    pub kind: String,
    pub summary: String,
    pub realm: Option<String>,
    pub controlling_faction: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FactionDetails {
    pub category: String,
    pub summary: String,
    #[serde(default)]
    pub goals: Vec<String>,
    pub sphere_of_influence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReligionDetails {
    pub domain: String,
    pub summary: String,
    #[serde(default)]
    pub core_beliefs: Vec<String>,
    #[serde(default)]
    pub practices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EconomyDetails {
    pub system: String,
    pub summary: String,
    pub currency: Option<String>,
    #[serde(default)]
    pub trade_goods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlotLineDetails {
    pub plot_type: String,
    pub summary: String,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConflictDetails {
    pub conflict_type: String,
    pub stakes: String,
    pub status: Option<String>,
    pub escalation_stage: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ThemeDetails {
    pub statement: String,
    pub thesis_antithesis: String,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MotifDetails {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub thematic_links: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SystemOverlayDetails {
    pub system_name: String,
    pub system_type: String,
    pub visibility: String,
    pub rules: String,
    #[serde(default)]
    pub stats: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NarrativePromiseDetails {
    pub promise_type: String,
    pub description: String,
    pub status: String,
    pub planned_payoff: Option<StoryPlacement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CharacterArcDetails {
    pub arc_type: String,
    pub starting_state: String,
    pub current_state: String,
    pub target_state: String,
    pub thematic_purpose: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TermDetails {
    pub term_text: String,
    pub pronunciation: Option<String>,
    pub definition: String,
    pub usage_context: Option<String>,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RelationshipDetails {
    pub relationship_type: String,
    pub source: SubjectLinkSummary,
    pub target: SubjectLinkSummary,
    pub trust: Option<i32>,
    pub tension: Option<i32>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TimelineEventDetails {
    pub title: String,
    pub event_type: String,
    pub placement: StoryPlacement,
    pub summary: String,
    #[serde(default)]
    pub related_subjects: Vec<SubjectLinkSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalFactSummary {
    pub fact: String,
    pub source_label: Option<String>,
    pub provenance: Provenance,
}

impl CanonicalFactSummary {
    fn preview(&self) -> String {
        match &self.source_label {
            Some(source) => format!(
                "{} ({source}; {})",
                self.fact,
                format_provenance(&self.provenance)
            ),
            None => format!("{} ({})", self.fact, format_provenance(&self.provenance)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeFactSummary {
    pub fact: String,
    pub scope: Option<String>,
    pub source: Option<String>,
    pub learned_at: Option<StoryPlacement>,
    pub confidence: Option<f64>,
    pub provenance: Provenance,
}

impl KnowledgeFactSummary {
    fn preview(&self) -> String {
        let mut parts = vec![self.fact.clone()];
        if let Some(scope) = &self.scope {
            parts.push(format!("scope: {scope}"));
        }
        if let Some(source) = &self.source {
            parts.push(format!("source: {source}"));
        }
        if let Some(learned_at) = &self.learned_at {
            parts.push(format!(
                "learned at: {}",
                format_story_placement(learned_at)
            ));
        }
        if let Some(confidence) = self.confidence {
            parts.push(format!("confidence: {:.2}", confidence));
        }
        parts.push(format!(
            "provenance: {}",
            format_provenance(&self.provenance)
        ));
        parts.join(" | ")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RelationshipSummary {
    pub relationship_type: String,
    pub counterpart: Option<String>,
    pub summary: String,
    pub trust: Option<i32>,
    pub tension: Option<i32>,
    pub provenance: Provenance,
}

impl RelationshipSummary {
    fn preview(&self) -> String {
        let mut parts = vec![format!("{}: {}", self.relationship_type, self.summary)];
        if let Some(counterpart) = &self.counterpart {
            parts.push(format!("with {counterpart}"));
        }
        if let Some(trust) = self.trust {
            parts.push(format!("trust {trust}"));
        }
        if let Some(tension) = self.tension {
            parts.push(format!("tension {tension}"));
        }
        parts.push(format!("from {}", format_provenance(&self.provenance)));
        parts.join(" | ")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NarrativePromiseSummary {
    pub promise_type: String,
    pub description: String,
    pub status: String,
    pub planned_payoff: Option<StoryPlacement>,
    pub provenance: Provenance,
}

impl NarrativePromiseSummary {
    fn preview(&self) -> String {
        match &self.planned_payoff {
            Some(payoff) => format!(
                "{}: {} [{}; payoff {}]",
                self.promise_type,
                self.description,
                self.status,
                format_story_placement(payoff)
            ),
            None => format!(
                "{}: {} [{}]",
                self.promise_type, self.description, self.status
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CharacterArcSummary {
    pub arc_type: String,
    pub summary: String,
    pub current_phase: Option<String>,
    pub provenance: Provenance,
}

impl CharacterArcSummary {
    fn preview(&self) -> String {
        match &self.current_phase {
            Some(phase) => format!("{}: {} ({phase})", self.arc_type, self.summary),
            None => format!("{}: {}", self.arc_type, self.summary),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SceneAppearanceSummary {
    pub scene_id: RecordId,
    pub placement: Option<StoryPlacement>,
    pub summary: Option<String>,
    pub provenance: Provenance,
}

impl SceneAppearanceSummary {
    fn preview(&self) -> String {
        let label = self
            .placement
            .as_ref()
            .map(format_story_placement)
            .unwrap_or_else(|| self.scene_id.to_string());
        match &self.summary {
            Some(summary) => format!("{label}: {summary}"),
            None => label,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceProfileSummary {
    pub tone: Option<String>,
    #[serde(default)]
    pub vocabulary: Vec<String>,
    #[serde(default)]
    pub sentence_structure: Vec<String>,
    #[serde(default)]
    pub tics: Vec<String>,
    #[serde(default)]
    pub forbidden_words: Vec<String>,
    #[serde(default)]
    pub example_lines: Vec<String>,
    pub established_in_scene_id: Option<RecordId>,
    pub updated_at: Option<String>,
    pub provenance: Provenance,
}

impl VoiceProfileSummary {
    fn preview(&self) -> String {
        let mut parts = Vec::new();
        if let Some(tone) = &self.tone {
            parts.push(format!("tone: {tone}"));
        }
        if !self.vocabulary.is_empty() {
            parts.push(format!("vocabulary: {}", self.vocabulary.join(", ")));
        }
        if !self.tics.is_empty() {
            parts.push(format!("tics: {}", self.tics.join(", ")));
        }
        parts.push(format!("from {}", format_provenance(&self.provenance)));
        parts.join(" | ")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CharacterStateSummary {
    #[serde(default)]
    pub emotional_state: BTreeMap<String, Value>,
    #[serde(default)]
    pub goals: Vec<String>,
    #[serde(default)]
    pub status: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub source_summary: Option<String>,
    pub provenance: Provenance,
}

impl CharacterStateSummary {
    fn preview(&self) -> String {
        let mut parts = Vec::new();
        if !self.goals.is_empty() {
            parts.push(format!("goals: {}", self.goals.join(", ")));
        }
        if !self.status.is_empty() {
            parts.push(format!("status: {}", self.status.join(", ")));
        }
        if let Some(summary) = &self.source_summary {
            parts.push(format!("source: {summary}"));
        }
        parts.push(format!("from {}", format_provenance(&self.provenance)));
        parts.join(" | ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement() -> StoryPlacement {
        StoryPlacement {
            book_number: 1,
            chapter_number: 2,
            scene_order: Some(3),
            note: Some("Act break".to_string()),
        }
    }

    fn provenance() -> Provenance {
        Provenance::scene(RecordId::new("scene:test"), Some(10..20))
    }

    fn subject_link(table: SubjectTable, id: &str, display_name: &str) -> SubjectLinkSummary {
        SubjectLinkSummary {
            subject: Subject::new(table, id).unwrap(),
            display_name: display_name.to_string(),
        }
    }

    fn all_variants() -> Vec<SubjectKindSpecific> {
        vec![
            SubjectKindSpecific::Character(CharacterDetails {
                role: "protagonist".to_string(),
                summary: "Leads the chapter.".to_string(),
                realm: Some("capital".to_string()),
            }),
            SubjectKindSpecific::Location(LocationDetails {
                kind: "city".to_string(),
                summary: "A fortified river city.".to_string(),
                realm: Some("north".to_string()),
                controlling_faction: Some("Wardens".to_string()),
                status: Some("contested".to_string()),
            }),
            SubjectKindSpecific::Faction(FactionDetails {
                category: "military".to_string(),
                summary: "Controls the walls.".to_string(),
                goals: vec!["Hold the line".to_string()],
                sphere_of_influence: Some("river gate".to_string()),
            }),
            SubjectKindSpecific::Religion(ReligionDetails {
                domain: "storm".to_string(),
                summary: "Interprets weather as omen.".to_string(),
                core_beliefs: vec!["Nothing is random".to_string()],
                practices: vec!["bell tolling".to_string()],
            }),
            SubjectKindSpecific::Economy(EconomyDetails {
                system: "rationing".to_string(),
                summary: "Controlled wartime trade.".to_string(),
                currency: Some("marks".to_string()),
                trade_goods: vec!["salt".to_string(), "fuel".to_string()],
            }),
            SubjectKindSpecific::PlotLine(PlotLineDetails {
                plot_type: "mystery".to_string(),
                summary: "Tracks the stolen map.".to_string(),
                status: Some("active".to_string()),
            }),
            SubjectKindSpecific::Conflict(ConflictDetails {
                conflict_type: "interpersonal".to_string(),
                stakes: "Trust inside the team.".to_string(),
                status: Some("escalating".to_string()),
                escalation_stage: Some("accusation".to_string()),
            }),
            SubjectKindSpecific::Theme(ThemeDetails {
                statement: "Mercy costs power.".to_string(),
                thesis_antithesis: "Mercy vs expedience".to_string(),
                status: Some("foregrounded".to_string()),
            }),
            SubjectKindSpecific::Motif(MotifDetails {
                name: "Ash".to_string(),
                description: "Signals aftermath.".to_string(),
                thematic_links: vec!["loss".to_string()],
            }),
            SubjectKindSpecific::SystemOverlay(SystemOverlayDetails {
                system_name: "Resonance".to_string(),
                system_type: "magic".to_string(),
                visibility: "public".to_string(),
                rules: "Cost scales with chorus size.".to_string(),
                stats: vec!["strain".to_string()],
            }),
            SubjectKindSpecific::NarrativePromise(NarrativePromiseDetails {
                promise_type: "mystery_box".to_string(),
                description: "Who betrayed the convoy?".to_string(),
                status: "open".to_string(),
                planned_payoff: Some(placement()),
            }),
            SubjectKindSpecific::CharacterArc(CharacterArcDetails {
                arc_type: "belief_shift".to_string(),
                starting_state: "obedient".to_string(),
                current_state: "doubting".to_string(),
                target_state: "self-directed".to_string(),
                thematic_purpose: "Claim agency.".to_string(),
            }),
            SubjectKindSpecific::Term(TermDetails {
                term_text: "Glasswake".to_string(),
                pronunciation: Some("GLASS-wake".to_string()),
                definition: "Frozen shockwave on the bay.".to_string(),
                usage_context: Some("naval jargon".to_string()),
                origin: Some("sailor slang".to_string()),
            }),
            SubjectKindSpecific::Relationship(RelationshipDetails {
                relationship_type: "allies".to_string(),
                source: subject_link(SubjectTable::Character, "character:a", "Mara"),
                target: subject_link(SubjectTable::Character, "character:b", "Iven"),
                trust: Some(6),
                tension: Some(3),
                summary: "Operational but fraying.".to_string(),
            }),
            SubjectKindSpecific::TimelineEvent(TimelineEventDetails {
                title: "Gate breach".to_string(),
                event_type: "battle".to_string(),
                placement: placement(),
                summary: "The eastern gate fails.".to_string(),
                related_subjects: vec![
                    subject_link(SubjectTable::Character, "character:a", "Mara"),
                    subject_link(SubjectTable::Location, "location:g1", "East Gate"),
                ],
            }),
            SubjectKindSpecific::Generic(serde_json::json!({
                "subject_table": "scene",
                "status": "unsupported"
            })),
        ]
    }

    #[test]
    fn subject_kind_specific_variants_round_trip() {
        for variant in all_variants() {
            let json = serde_json::to_string(&variant).unwrap();
            let back: SubjectKindSpecific = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, back);
        }
    }

    fn character_voice_profile() -> VoiceProfileSummary {
        VoiceProfileSummary {
            tone: Some("clipped".to_string()),
            vocabulary: vec!["breach".to_string()],
            sentence_structure: vec!["short commands".to_string()],
            tics: vec!["counts exits".to_string()],
            forbidden_words: vec!["hope".to_string()],
            example_lines: vec!["Lock it down.".to_string()],
            established_in_scene_id: Some(RecordId::new("scene:test")),
            updated_at: Some("2026-04-08T00:00:00Z".to_string()),
            provenance: provenance(),
        }
    }

    fn character_state() -> CharacterStateSummary {
        CharacterStateSummary {
            emotional_state: BTreeMap::from([("fear".to_string(), serde_json::json!("contained"))]),
            goals: vec!["Hold the wall".to_string()],
            status: vec!["wounded".to_string()],
            notes: vec!["Hiding the severity".to_string()],
            source_summary: Some("Scene aftermath".to_string()),
            provenance: provenance(),
        }
    }

    fn snapshot_for(subject: Subject, kind_specific: SubjectKindSpecific) -> SubjectSnapshot {
        let is_character = matches!(kind_specific, SubjectKindSpecific::Character(_));
        SubjectSnapshot::new(
            subject,
            "Sample".to_string(),
            kind_specific,
            vec![CanonicalFactSummary {
                fact: "Anchor fact.".to_string(),
                source_label: Some("scene 3".to_string()),
                provenance: provenance(),
            }],
            vec![KnowledgeFactSummary {
                fact: "Observed the breach.".to_string(),
                scope: Some("private".to_string()),
                source: Some("letter".to_string()),
                learned_at: Some(placement()),
                confidence: Some(0.8),
                provenance: provenance(),
            }],
            vec![RelationshipSummary {
                relationship_type: "ally".to_string(),
                counterpart: Some("Iven".to_string()),
                summary: "Reliable under fire.".to_string(),
                trust: Some(7),
                tension: Some(2),
                provenance: provenance(),
            }],
            vec![NarrativePromiseSummary {
                promise_type: "mystery_box".to_string(),
                description: "Reveal the saboteur.".to_string(),
                status: "open".to_string(),
                planned_payoff: Some(placement()),
                provenance: provenance(),
            }],
            vec![CharacterArcSummary {
                arc_type: "belief_shift".to_string(),
                summary: "Learns to delegate.".to_string(),
                current_phase: Some("pressure".to_string()),
                provenance: provenance(),
            }],
            vec![SceneAppearanceSummary {
                scene_id: RecordId::new("scene:test"),
                placement: Some(placement()),
                summary: Some("Secures the breach.".to_string()),
                provenance: provenance(),
            }],
            is_character.then(character_voice_profile),
            is_character.then(character_state),
            placement(),
            provenance(),
        )
        .unwrap()
    }

    #[test]
    fn subject_snapshot_round_trip_for_every_variant() {
        let snapshots = vec![
            snapshot_for(
                Subject::new(SubjectTable::Character, "character:mara").unwrap(),
                SubjectKindSpecific::Character(CharacterDetails {
                    role: "protagonist".to_string(),
                    summary: "Holding the line.".to_string(),
                    realm: Some("Northwall".to_string()),
                }),
            ),
            snapshot_for(
                Subject::new(SubjectTable::Location, "location:east-gate").unwrap(),
                SubjectKindSpecific::Location(LocationDetails {
                    kind: "fortress".to_string(),
                    summary: "The breach point.".to_string(),
                    realm: Some("Northwall".to_string()),
                    controlling_faction: Some("Wardens".to_string()),
                    status: Some("under siege".to_string()),
                }),
            ),
            snapshot_for(
                Subject::new(SubjectTable::Faction, "faction:wardens").unwrap(),
                SubjectKindSpecific::Faction(FactionDetails {
                    category: "military".to_string(),
                    summary: "Holds the wall.".to_string(),
                    goals: vec!["Repel the siege".to_string()],
                    sphere_of_influence: Some("city defenses".to_string()),
                }),
            ),
        ];

        for snapshot in snapshots {
            let json = serde_json::to_string(&snapshot).unwrap();
            let back: SubjectSnapshot = serde_json::from_str(&json).unwrap();
            assert_eq!(snapshot, back);
        }
    }

    #[test]
    fn subject_snapshot_rejects_mismatched_subject_and_kind() {
        let err = SubjectSnapshot::new(
            Subject::new(SubjectTable::Location, "location:east-gate").unwrap(),
            "East Gate".to_string(),
            SubjectKindSpecific::Character(CharacterDetails {
                role: "protagonist".to_string(),
                summary: "Wrong kind.".to_string(),
                realm: None,
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(character_voice_profile()),
            Some(character_state()),
            placement(),
            provenance(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            SubjectSnapshotError::SubjectKindMismatch {
                subject_table: SubjectTable::Location,
                ..
            }
        ));
    }

    #[test]
    fn subject_snapshot_rejects_character_only_fields_on_non_character_kind() {
        let err = SubjectSnapshot::new(
            Subject::new(SubjectTable::Location, "location:east-gate").unwrap(),
            "East Gate".to_string(),
            SubjectKindSpecific::Location(LocationDetails {
                kind: "fortress".to_string(),
                summary: "The breach point.".to_string(),
                realm: None,
                controlling_faction: None,
                status: None,
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(character_voice_profile()),
            None,
            placement(),
            provenance(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            SubjectSnapshotError::CharacterOnlyField {
                field: "voice_profile",
                ..
            }
        ));
    }

    #[test]
    fn subject_snapshot_rejects_generic_for_supported_table() {
        let err = SubjectSnapshot::new(
            Subject::new(SubjectTable::Character, "character:mara").unwrap(),
            "Mara".to_string(),
            SubjectKindSpecific::Generic(serde_json::json!({
                "status": "should be typed"
            })),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            placement(),
            provenance(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            SubjectSnapshotError::GenericKindNotAllowed {
                subject_table: SubjectTable::Character
            }
        ));
    }

    #[test]
    fn subject_snapshot_deserialize_rejects_malformed_mixed_kind() {
        let json = serde_json::json!({
            "subject": { "table": "location", "id": "location:east-gate" },
            "display_name": "East Gate",
            "kind_specific": {
                "kind": "character",
                "details": {
                    "role": "protagonist",
                    "summary": "Wrong kind",
                    "realm": null
                }
            },
            "canonical_facts": [],
            "knowledge": [],
            "relationships": [],
            "open_promises": [],
            "active_arcs": [],
            "recent_appearances": [],
            "voice_profile": null,
            "current_state": null,
            "at_placement": {
                "book_number": 1,
                "chapter_number": 2,
                "scene_order": 3,
                "note": "Act break"
            },
            "provenance": {
                "kind": "scene",
                "scene_id": "scene:test",
                "byte_range": { "start": 10, "end": 20 }
            }
        });

        let err = serde_json::from_value::<SubjectSnapshot>(json).unwrap_err();
        assert!(err.to_string().contains("does not match snapshot kind"));
    }

    #[test]
    fn subject_snapshot_deserialize_rejects_generic_for_supported_table() {
        let json = serde_json::json!({
            "subject": { "table": "character", "id": "character:mara" },
            "display_name": "Mara",
            "kind_specific": {
                "kind": "generic",
                "details": {
                    "status": "should be typed"
                }
            },
            "canonical_facts": [],
            "knowledge": [],
            "relationships": [],
            "open_promises": [],
            "active_arcs": [],
            "recent_appearances": [],
            "voice_profile": null,
            "current_state": null,
            "at_placement": {
                "book_number": 1,
                "chapter_number": 2,
                "scene_order": 3,
                "note": "Act break"
            },
            "provenance": {
                "kind": "scene",
                "scene_id": "scene:test",
                "byte_range": { "start": 10, "end": 20 }
            }
        });

        let err = serde_json::from_value::<SubjectSnapshot>(json).unwrap_err();
        assert!(
            err.to_string()
                .contains("generic snapshot kind is not allowed for supported table")
        );
    }

    #[test]
    fn render_markdown_supports_all_depths() {
        let snapshot = SubjectSnapshot::new(
            Subject::new(SubjectTable::Character, "character:mara").unwrap(),
            "Mara".to_string(),
            SubjectKindSpecific::Character(CharacterDetails {
                role: "protagonist".to_string(),
                summary: "Holding the line.".to_string(),
                realm: Some("Northwall".to_string()),
            }),
            vec![CanonicalFactSummary {
                fact: "Mara commands the east watch.".to_string(),
                source_label: Some("scene 3".to_string()),
                provenance: provenance(),
            }],
            vec![KnowledgeFactSummary {
                fact: "Suspects a traitor.".to_string(),
                scope: Some("private".to_string()),
                source: Some("letter".to_string()),
                learned_at: Some(placement()),
                confidence: Some(0.8),
                provenance: provenance(),
            }],
            vec![RelationshipSummary {
                relationship_type: "ally".to_string(),
                counterpart: Some("Iven".to_string()),
                summary: "Reliable under fire.".to_string(),
                trust: Some(7),
                tension: Some(2),
                provenance: provenance(),
            }],
            vec![NarrativePromiseSummary {
                promise_type: "mystery_box".to_string(),
                description: "Reveal the saboteur.".to_string(),
                status: "open".to_string(),
                planned_payoff: Some(placement()),
                provenance: provenance(),
            }],
            vec![CharacterArcSummary {
                arc_type: "belief_shift".to_string(),
                summary: "Learns to delegate.".to_string(),
                current_phase: Some("pressure".to_string()),
                provenance: provenance(),
            }],
            vec![SceneAppearanceSummary {
                scene_id: RecordId::new("scene:test"),
                placement: Some(placement()),
                summary: Some("Secures the breach.".to_string()),
                provenance: provenance(),
            }],
            Some(character_voice_profile()),
            Some(character_state()),
            placement(),
            provenance(),
        )
        .unwrap();

        let minimal = snapshot.render_markdown(RenderDepth::Minimal);
        let standard = snapshot.render_markdown(RenderDepth::Standard);
        let full = snapshot.render_markdown(RenderDepth::Full);

        assert!(minimal.contains("# Mara"));
        assert!(minimal.contains("Signals: 1 fact"));
        assert!(standard.contains("## Details"));
        assert!(standard.contains("## Canonical Facts"));
        assert!(full.contains("## Snapshot JSON"));
        assert!(full.contains("\"display_name\": \"Mara\""));
    }
}
