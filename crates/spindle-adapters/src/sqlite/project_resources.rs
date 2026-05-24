//! Helpers for SQLite-backed `read_project_resource` dispatch.
//!
//! These stay private to the SQLite adapter. They keep pagination envelopes
//! and record-to-JSON projections out of the main service implementation while
//! preserving the MCP resource shapes.

use anyhow::Context;

use super::json_records::StoredDualPersonaReviewRound;
use super::records::{
    DualPersonaReview, FutureKnowledge, RelatesTo, TemporalIntervention, TimelineEvent,
};

/// Page size used when the caller omits `<offset>/<limit>` on a paginated
/// project resource. Mirrors the SurrealDB default.
const DEFAULT_PROJECT_RESOURCE_PAGE_SIZE: usize = 50;

/// Hard cap on per-page entries for paginated project resources. Prevents
/// callers from requesting an unbounded slice in a single MCP read.
const MAX_PROJECT_RESOURCE_PAGE_SIZE: usize = 200;

/// Project-scoped resource kinds that paginate over a project-wide entry
/// list. Used by `parse_project_resource_page_request` to identify the
/// `<resource_name>` prefix on a `read_project_resource` path and to project
/// metadata (resource name, order) into the response envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PaginatedProjectResourceKind {
    ResearchLog,
    Conflicts,
    FutureKnowledge,
    TimelineEvents,
    DualPersonaReviews,
    Relationships,
    TemporalInterventions,
}

impl PaginatedProjectResourceKind {
    fn resource_name(self) -> &'static str {
        match self {
            Self::ResearchLog => "research-log",
            Self::Conflicts => "conflicts",
            Self::FutureKnowledge => "future-knowledge",
            Self::TimelineEvents => "timeline-events",
            Self::DualPersonaReviews => "dual-persona-reviews",
            Self::Relationships => "relationships",
            Self::TemporalInterventions => "temporal-interventions",
        }
    }

    fn order(self) -> &'static str {
        match self {
            Self::ResearchLog => "newest_first",
            Self::Conflicts => "normalized_name",
            Self::FutureKnowledge => "created_at",
            Self::TimelineEvents => "story_order",
            Self::DualPersonaReviews => "updated_at_desc",
            Self::Relationships => "relationship_type",
            Self::TemporalInterventions => "created_at",
        }
    }
}

/// Parsed `<resource_name>[/<offset>/<limit>]` request. `offset` and `limit`
/// default to 0 and `DEFAULT_PROJECT_RESOURCE_PAGE_SIZE` for the bare
/// resource form.
#[derive(Debug, Clone, Copy)]
pub(super) struct ProjectResourcePageRequest {
    pub(super) kind: PaginatedProjectResourceKind,
    offset: usize,
    limit: usize,
}

/// Parse a `read_project_resource` path against the known paginated resource
/// kinds. Returns `Ok(None)` when the path isn't paginated (the caller
/// continues into the simple-resource dispatch). Returns `Err` only when the
/// path is structurally invalid for a recognized resource (bad offset/limit,
/// over-size limit, zero limit, etc.).
pub(super) fn parse_project_resource_page_request(
    resource_path: &str,
) -> anyhow::Result<Option<ProjectResourcePageRequest>> {
    for kind in [
        PaginatedProjectResourceKind::ResearchLog,
        PaginatedProjectResourceKind::Conflicts,
        PaginatedProjectResourceKind::FutureKnowledge,
        PaginatedProjectResourceKind::TimelineEvents,
        PaginatedProjectResourceKind::DualPersonaReviews,
        PaginatedProjectResourceKind::Relationships,
        PaginatedProjectResourceKind::TemporalInterventions,
    ] {
        let resource_name = kind.resource_name();
        if resource_path == resource_name {
            return Ok(Some(ProjectResourcePageRequest {
                kind,
                offset: 0,
                limit: DEFAULT_PROJECT_RESOURCE_PAGE_SIZE,
            }));
        }

        let Some(page_path) = resource_path.strip_prefix(&format!("{resource_name}/")) else {
            continue;
        };
        let parts = page_path.split('/').collect::<Vec<_>>();
        if parts.len() != 2 {
            anyhow::bail!(
                "{resource_name} resource path must be {resource_name} or {resource_name}/<offset>/<limit>"
            );
        }

        let offset = parts[0].parse::<usize>().with_context(|| {
            format!(
                "invalid {resource_name} offset in resource path: {}",
                parts[0]
            )
        })?;
        let limit = parts[1].parse::<usize>().with_context(|| {
            format!(
                "invalid {resource_name} page size in resource path: {}",
                parts[1]
            )
        })?;
        if limit == 0 {
            anyhow::bail!("{resource_name} page size must be greater than zero");
        }
        if limit > MAX_PROJECT_RESOURCE_PAGE_SIZE {
            anyhow::bail!("{resource_name} page size must be <= {MAX_PROJECT_RESOURCE_PAGE_SIZE}");
        }

        return Ok(Some(ProjectResourcePageRequest {
            kind,
            offset,
            limit,
        }));
    }

    Ok(None)
}

/// Build the canonical paginated resource URI for `previous_resource` /
/// `next_resource` envelope hints.
fn project_resource_page_uri(
    project_id: &str,
    resource_name: &str,
    offset: usize,
    limit: usize,
) -> String {
    format!("bible://projects/{project_id}/{resource_name}/{offset}/{limit}")
}

/// Build the response JSON envelope for a paginated project resource. The
/// supplied `entries` is the full unfiltered list; pagination slicing happens
/// here. `extra` is merged into the top-level object so callers can thread in
/// extra fields, such as `active_branch_id` on relationships.
pub(super) fn paginated_project_resource_response(
    project_id: &str,
    page: ProjectResourcePageRequest,
    entries: Vec<serde_json::Value>,
    extra: Option<serde_json::Map<String, serde_json::Value>>,
) -> serde_json::Value {
    use serde_json::{Value, json};
    let total = entries.len();
    let paged_entries = entries
        .into_iter()
        .skip(page.offset)
        .take(page.limit)
        .collect::<Vec<_>>();
    let returned = paged_entries.len();
    let next_offset = page.offset.saturating_add(returned);

    let mut response = extra.unwrap_or_default();
    response.insert("entries".to_string(), Value::Array(paged_entries));
    response.insert(
        "pagination".to_string(),
        json!({
            "offset": page.offset,
            "limit": page.limit,
            "returned": returned,
            "total": total,
            "has_more": next_offset < total,
            "next_resource": (next_offset < total).then(|| {
                project_resource_page_uri(
                    project_id,
                    page.kind.resource_name(),
                    next_offset,
                    page.limit,
                )
            }),
            "previous_resource": (page.offset > 0).then(|| {
                project_resource_page_uri(
                    project_id,
                    page.kind.resource_name(),
                    page.offset.saturating_sub(page.limit),
                    page.limit,
                )
            }),
            "order": page.kind.order(),
        }),
    );
    Value::Object(response)
}

pub(super) fn future_knowledge_to_json(fk: FutureKnowledge) -> serde_json::Value {
    serde_json::json!({
        "id": fk.id,
        "character_id": fk.character_id,
        "knowledge_summary": fk.knowledge_summary,
        "source": fk.source,
        "learned_at": fk.learned_at,
        "expires_at": fk.expires_at,
        "notes": fk.notes,
    })
}

pub(super) fn timeline_event_to_json(te: TimelineEvent) -> serde_json::Value {
    serde_json::json!({
        "id": te.id,
        "title": te.title,
        "event_type": te.event_type,
        "placement": te.placement,
        "summary": te.summary,
        "related_entity_ids": te.related_entity_ids,
    })
}

pub(super) fn temporal_intervention_to_json(ti: TemporalIntervention) -> serde_json::Value {
    serde_json::json!({
        "id": ti.id,
        "title": ti.title,
        "intervention_type": ti.intervention_type,
        "source_event_id": ti.source_event_id,
        "target_event_id": ti.target_event_id,
        "summary": ti.summary,
        "consequences": ti.consequences,
        "status": ti.status,
    })
}

pub(super) fn relates_to_json(r: RelatesTo) -> serde_json::Value {
    serde_json::json!({
        "in": r.in_id,
        "out": r.out_id,
        "relationship_type": r.relationship_type,
        "trust": r.trust,
        "tension": r.tension,
        "dynamics": r.dynamics,
        "reason": r.reason,
        "last_scene_id": r.last_scene_id,
    })
}

/// Project a stored `DualPersonaReview` row into the
/// `PersistedDualPersonaReview` shape exposed by `read_project_resource`
/// (`dual-persona-reviews` and its paginated variant). The `review_rounds`
/// JSON column is decoded into typed `DualPersonaReviewRound` instances;
/// legacy `{rounds: [...]}` envelopes are unwrapped transparently to match
/// the SurrealDB-era persisted shape.
pub(super) fn persisted_dual_persona_review(
    review: DualPersonaReview,
) -> anyhow::Result<spindle_core::models::PersistedDualPersonaReview> {
    Ok(spindle_core::models::PersistedDualPersonaReview {
        review_id: review.id,
        scene_id: review.scene_id,
        branch_id: review.branch_id,
        rounds_completed: review.rounds_completed as usize,
        status: review.status,
        review_rounds: stored_review_rounds_into_core(review.review_rounds)?,
    })
}

fn stored_review_rounds_into_core(
    value: serde_json::Value,
) -> anyhow::Result<Vec<spindle_core::models::DualPersonaReviewRound>> {
    let rounds_value = match value {
        serde_json::Value::Null => return Ok(Vec::new()),
        serde_json::Value::Array(items) => serde_json::Value::Array(items),
        serde_json::Value::Object(mut object) => object
            .remove("rounds")
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
        other => anyhow::bail!("invalid dual persona review payload: {other}"),
    };

    let rounds: Vec<StoredDualPersonaReviewRound> = serde_json::from_value(rounds_value)?;
    Ok(rounds
        .into_iter()
        .map(StoredDualPersonaReviewRound::into_core)
        .collect())
}
