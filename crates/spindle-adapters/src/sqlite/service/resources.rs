use super::*;

impl SqliteSpindleService {
    // =========================================================================
    // MCP-dispatched tool surface
    // =========================================================================
    //
    // The methods below complete the SqliteSpindleService's MCP-required
    // surface. Phase 6 stub-fill finished — every method has a real
    // implementation and matches the SurrealDB-era behaviour
    // observable through MCP.

    /// Fetch a branch's identifying fields for MCP's session-management
    /// flows. Returns (branch_id, project_id, name). The `project_id` is
    /// always present for SQLite-created branches (per-project main branches
    /// design); the Option in the underlying record is a schema-level legacy
    /// from the SurrealDB singleton era and surfaces here as an error if
    /// somehow encountered.
    pub async fn get_branch_info(&self, branch_id: &str) -> Result<(String, String, String)> {
        let b = self.repository.get_branch(branch_id).await?;
        let project_id = b
            .project_id
            .ok_or_else(|| anyhow::anyhow!("branch {branch_id} has no project_id"))?;
        Ok((b.id, project_id, b.name))
    }

    /// Resolve the active branch id for a project. Used by MCP's session
    /// defaulting (`default_branch_id_for_project`) so callers can omit
    /// `branch_id` and have it auto-filled with the project's current main.
    ///
    /// Per the per-project-main-branch design (Phase 6 reconciliation of the
    /// SurrealDB singleton bible_branch:main divergence), every project has
    /// its own main branch row — there is no shared fallback.
    pub async fn active_branch_id_for_project(&self, project_id: &str) -> Result<String> {
        self.repository.active_branch_id_public(project_id).await
    }

    /// List all project IDs as `project:<ulid>` strings. Drives MCP's
    /// resource enumeration without forcing the full list_projects payload.
    pub async fn list_project_ids(&self) -> Result<Vec<String>> {
        Ok(self
            .repository
            .list_projects()
            .await?
            .into_iter()
            .map(|p| p.id)
            .collect())
    }

    pub(super) async fn continuity_health_resource(
        &self,
        project_id: &str,
    ) -> Result<serde_json::Value> {
        use serde_json::{Value, json};
        use std::collections::{BTreeMap, BTreeSet};

        self.repository.get_project(project_id).await?;
        let active_branch = self.repository.get_active_branch(project_id).await?;
        let branches = self.repository.list_branches_by_project(project_id).await?;
        let branch_by_id = branches
            .iter()
            .map(|branch| (branch.id.clone(), branch))
            .collect::<BTreeMap<_, _>>();

        let mut branch_lineage = Vec::new();
        let mut seen_branch_ids = BTreeSet::new();
        let mut current_branch_id = Some(active_branch.id.clone());
        while let Some(branch_id) = current_branch_id {
            if !seen_branch_ids.insert(branch_id.clone()) {
                break;
            }
            let Some(branch) = branch_by_id.get(&branch_id) else {
                break;
            };
            branch_lineage.push(json!({
                "id": branch.id.clone(),
                "name": branch.name.clone(),
                "parent_branch_id": branch.parent_branch_id.clone(),
                "status": branch.status.clone(),
                "branch_type": branch.branch_type.clone(),
            }));
            current_branch_id = branch.parent_branch_id.clone();
        }
        branch_lineage.reverse();

        let validator_findings = self
            .repository
            .list_validator_findings_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut open_by_validator_and_severity: BTreeMap<String, BTreeMap<String, usize>> =
            BTreeMap::new();
        let mut cache_counts_by_validator: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut total_open_findings = 0usize;
        let mut total_active_cache_rows = 0usize;
        let mut total_resolved_cache_rows = 0usize;

        for finding in &validator_findings {
            if finding.resolved_at.is_none() {
                total_open_findings += 1;
                *open_by_validator_and_severity
                    .entry(finding.validator_id.clone())
                    .or_default()
                    .entry(finding.severity.clone())
                    .or_default() += 1;
            }

            if finding.finding_id == "__cache__" {
                let (active_count, resolved_count) = cache_counts_by_validator
                    .entry(finding.validator_id.clone())
                    .or_insert((0usize, 0usize));
                if finding.resolved_at.is_some() {
                    *resolved_count += 1;
                    total_resolved_cache_rows += 1;
                } else {
                    *active_count += 1;
                    total_active_cache_rows += 1;
                }
            }
        }

        let validator_cache_counts = cache_counts_by_validator
            .into_iter()
            .map(|(validator_id, (active_count, resolved_count))| {
                (
                    validator_id,
                    json!({
                        "active_count": active_count,
                        "resolved_count": resolved_count,
                        "total_count": active_count + resolved_count,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();

        let timeline_events = self
            .repository
            .list_timeline_events_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let timeline_event_ids = timeline_events
            .iter()
            .map(|event| event.id.clone())
            .collect::<BTreeSet<_>>();
        let temporal_interventions = self
            .repository
            .list_temporal_interventions_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut orphaned_temporal_interventions = Vec::new();
        for intervention in &temporal_interventions {
            let mut missing_endpoints = Vec::new();
            match intervention.source_event_id.as_ref() {
                Some(source_event_id) if !timeline_event_ids.contains(source_event_id) => {
                    missing_endpoints.push(json!({
                        "field": "source_event_id",
                        "event_id": source_event_id,
                        "reason": "not_found",
                    }));
                }
                None => missing_endpoints.push(json!({
                    "field": "source_event_id",
                    "event_id": null,
                    "reason": "unset",
                })),
                _ => {}
            }
            match intervention.target_event_id.as_ref() {
                Some(target_event_id) if !timeline_event_ids.contains(target_event_id) => {
                    missing_endpoints.push(json!({
                        "field": "target_event_id",
                        "event_id": target_event_id,
                        "reason": "not_found",
                    }));
                }
                None => missing_endpoints.push(json!({
                    "field": "target_event_id",
                    "event_id": null,
                    "reason": "unset",
                })),
                _ => {}
            }

            if !missing_endpoints.is_empty() {
                orphaned_temporal_interventions.push(json!({
                    "id": intervention.id.clone(),
                    "title": intervention.title.clone(),
                    "source_event_id": intervention.source_event_id.clone(),
                    "target_event_id": intervention.target_event_id.clone(),
                    "missing_endpoints": missing_endpoints,
                }));
            }
        }

        let canonical_facts = self
            .repository
            .list_active_canonical_facts_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut facts_by_key: BTreeMap<(String, Option<String>, String), Vec<_>> = BTreeMap::new();
        for fact in &canonical_facts {
            facts_by_key
                .entry((
                    fact.subject_table.clone(),
                    fact.subject_id.clone(),
                    fact.predicate.clone(),
                ))
                .or_default()
                .push(fact);
        }
        let duplicate_active_canonical_facts = facts_by_key
            .into_iter()
            .filter_map(|((subject_table, subject_id, predicate), facts)| {
                if facts.len() < 2 {
                    return None;
                }
                let values = facts
                    .iter()
                    .map(|fact| canonical_fact_value_for_check(fact))
                    .collect::<Vec<_>>();
                let unique_values = values
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                Some(json!({
                    "subject_table": subject_table,
                    "subject_id": subject_id,
                    "predicate": predicate,
                    "fact_ids": facts.iter().map(|fact| fact.id.clone()).collect::<Vec<_>>(),
                    "values": values,
                    "unique_values": unique_values,
                }))
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "project_id": project_id,
            "active_branch_id": active_branch.id,
            "branch_count": branches.len(),
            "branch_lineage": branch_lineage,
            "validator_findings": {
                "total_open": total_open_findings,
                "open_by_validator_and_severity": open_by_validator_and_severity,
            },
            "validator_cache_counts": validator_cache_counts,
            "validator_cache_totals": {
                "active_count": total_active_cache_rows,
                "resolved_count": total_resolved_cache_rows,
                "total_count": total_active_cache_rows + total_resolved_cache_rows,
            },
            "last_check_consistency_at": Value::Null,
            "timeline": {
                "timeline_event_count": timeline_events.len(),
                "temporal_intervention_count": temporal_interventions.len(),
            },
            "orphaned_temporal_interventions": orphaned_temporal_interventions,
            "canonical_facts": {
                "active_count": canonical_facts.len(),
            },
            "duplicate_active_canonical_facts": duplicate_active_canonical_facts,
        }))
    }

    pub async fn timeline_graph_mermaid_resource(&self, project_id: &str) -> Result<String> {
        use std::collections::BTreeMap;

        self.repository.get_project(project_id).await?;
        let active_branch = self.repository.get_active_branch(project_id).await?;

        let mut branches = self.repository.list_branches_by_project(project_id).await?;
        branches.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

        let mut save_points = self.repository.list_all_save_points(project_id).await?;
        save_points.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

        let mut timeline_events = self.repository.list_all_timeline_events(project_id).await?;
        timeline_events.sort_by(|a, b| {
            a.branch_id
                .cmp(&b.branch_id)
                .then(a.placement.book_number.cmp(&b.placement.book_number))
                .then(a.placement.chapter_number.cmp(&b.placement.chapter_number))
                .then(
                    a.placement
                        .scene_order
                        .unwrap_or(0)
                        .cmp(&b.placement.scene_order.unwrap_or(0)),
                )
                .then(a.title.cmp(&b.title))
                .then(a.id.cmp(&b.id))
        });

        let mut temporal_interventions = self
            .repository
            .list_all_temporal_interventions(project_id)
            .await?;
        temporal_interventions.sort_by(|a, b| {
            a.branch_id
                .cmp(&b.branch_id)
                .then(a.title.cmp(&b.title))
                .then(a.id.cmp(&b.id))
        });

        let branch_node_ids = branches
            .iter()
            .map(|branch| (branch.id.clone(), mermaid_node_id("B", &branch.id)))
            .collect::<BTreeMap<_, _>>();
        let save_point_node_ids = save_points
            .iter()
            .map(|save_point| (save_point.id.clone(), mermaid_node_id("SP", &save_point.id)))
            .collect::<BTreeMap<_, _>>();
        let event_node_ids = timeline_events
            .iter()
            .map(|event| (event.id.clone(), mermaid_node_id("E", &event.id)))
            .collect::<BTreeMap<_, _>>();

        let mut out = String::new();
        out.push_str("```mermaid\n");
        out.push_str("flowchart LR\n");
        out.push_str("  classDef branch fill:#eef,stroke:#557\n");
        out.push_str("  classDef savepoint fill:#efe,stroke:#575\n");
        out.push_str("  classDef event fill:#fff,stroke:#777\n");
        out.push_str("  classDef warning fill:#fee,stroke:#a44\n\n");

        for branch in &branches {
            let node_id = &branch_node_ids[&branch.id];
            let active_suffix = if branch.id == active_branch.id {
                " (active)"
            } else {
                ""
            };
            out.push_str(&format!(
                "  {node_id}[\"{}\"]:::branch\n",
                mermaid_escape_label(&format!("{}{}", branch.name, active_suffix))
            ));
        }
        for save_point in &save_points {
            let node_id = &save_point_node_ids[&save_point.id];
            out.push_str(&format!(
                "  {node_id}[\"{}\"]:::savepoint\n",
                mermaid_escape_label(&format!("save: {}", save_point.name))
            ));
        }
        if !branches.is_empty() || !save_points.is_empty() {
            out.push('\n');
        }

        for branch in &branches {
            if let Some(parent_branch_id) = branch.parent_branch_id.as_ref()
                && let Some(parent_node_id) = branch_node_ids.get(parent_branch_id)
            {
                let branch_node_id = &branch_node_ids[&branch.id];
                out.push_str(&format!("  {parent_node_id} --> {branch_node_id}\n"));
            }
            if let Some(save_point_id) = branch.created_from_save_point_id.as_ref()
                && let Some(save_point_node_id) = save_point_node_ids.get(save_point_id)
            {
                let branch_node_id = &branch_node_ids[&branch.id];
                out.push_str(&format!("  {save_point_node_id} --> {branch_node_id}\n"));
            }
        }
        for save_point in &save_points {
            if let Some(branch_node_id) = branch_node_ids.get(&save_point.branch_id) {
                let save_point_node_id = &save_point_node_ids[&save_point.id];
                out.push_str(&format!("  {branch_node_id} --> {save_point_node_id}\n"));
            }
        }
        if !branches.is_empty() || !save_points.is_empty() {
            out.push('\n');
        }

        let interventions_by_branch = temporal_interventions.iter().fold(
            BTreeMap::<String, Vec<_>>::new(),
            |mut acc, intervention| {
                acc.entry(intervention.branch_id.clone())
                    .or_default()
                    .push(intervention);
                acc
            },
        );
        let events_by_branch =
            timeline_events
                .iter()
                .fold(BTreeMap::<String, Vec<_>>::new(), |mut acc, event| {
                    acc.entry(event.branch_id.clone()).or_default().push(event);
                    acc
                });

        for branch in &branches {
            let branch_events = events_by_branch
                .get(&branch.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let branch_interventions = interventions_by_branch
                .get(&branch.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let warning_interventions = branch_interventions
                .iter()
                .filter(|intervention| {
                    let source_visible = intervention
                        .source_event_id
                        .as_ref()
                        .is_some_and(|id| event_node_ids.contains_key(id));
                    let target_visible = intervention
                        .target_event_id
                        .as_ref()
                        .is_some_and(|id| event_node_ids.contains_key(id));
                    !(source_visible && target_visible)
                })
                .collect::<Vec<_>>();

            if branch_events.is_empty() && warning_interventions.is_empty() {
                continue;
            }

            let subgraph_id = mermaid_node_id("SG", &branch.id);
            out.push_str(&format!(
                "  subgraph {subgraph_id}[\"{}\"]\n",
                mermaid_escape_label(&format!("{} timeline", branch.name))
            ));
            for event in branch_events {
                let event_node_id = &event_node_ids[&event.id];
                out.push_str(&format!(
                    "    {event_node_id}[\"{}\"]:::event\n",
                    mermaid_escape_label(&timeline_event_graph_label(event))
                ));
            }
            for window in branch_events.windows(2) {
                let from = &event_node_ids[&window[0].id];
                let to = &event_node_ids[&window[1].id];
                out.push_str(&format!("    {from} --> {to}\n"));
            }
            for intervention in warning_interventions {
                let warning_node_id = mermaid_node_id("W", &intervention.id);
                out.push_str(&format!(
                    "    {warning_node_id}[\"{}\"]:::warning\n",
                    mermaid_escape_label(&format!(
                        "missing {}: {}",
                        mermaid_missing_endpoint_summary(intervention, &event_node_ids),
                        intervention.title
                    ))
                ));
            }
            out.push_str("  end\n\n");
        }

        for intervention in &temporal_interventions {
            let Some(source_node_id) = intervention
                .source_event_id
                .as_ref()
                .and_then(|id| event_node_ids.get(id))
            else {
                continue;
            };
            let Some(target_node_id) = intervention
                .target_event_id
                .as_ref()
                .and_then(|id| event_node_ids.get(id))
            else {
                continue;
            };
            out.push_str(&format!(
                "  {source_node_id} -. \"{}\" .-> {target_node_id}\n",
                mermaid_escape_label(&format!("intervention: {}", intervention.title))
            ));
        }

        out.push_str("```\n");
        Ok(out)
    }

    /// Read a project-scoped resource by path (e.g., `"books"`,
    /// `"chapters/1/2/scenes"`, `"research-log"`, `"conflicts/0/50"`). Drives
    /// the `bible://projects/{id}/...` MCP resource layer. Dispatches across:
    ///
    /// * Paginated resources (research-log, conflicts, future-knowledge,
    ///   timeline-events, dual-persona-reviews, relationships,
    ///   temporal-interventions) — both bare and `<resource>/<offset>/<limit>`
    ///   shapes.
    /// * The `chapters/<book>/<chapter>/scenes` reader.
    /// * `imports` and `imports/<session_id>[/<nested_path>]`.
    /// * 20+ simple list resources (books, chapters, characters, locations,
    ///   world-rules, factions, plot-lines, conflicts, themes, motifs,
    ///   narrative-promises, pacing/overview, chapter-summaries,
    ///   reader-contract, branches, future-knowledge, timeline-events,
    ///   temporal-interventions, system-overlays, dual-persona-reviews,
    ///   religions, economies, terms, character-arcs, relationships).
    ///
    /// `scene-move-impact/<from_book>/<from_chapter>/<from_scene>/<to_book>/<to_chapter>/<to_scene>`
    /// and `scene-delete-impact/<book>/<chapter>/<scene>` parse the path
    /// tail into the natural-key coordinates of the impact methods and
    /// return the structured `GetScene*ImpactOutput` payload as JSON.
    pub async fn read_project_resource(
        &self,
        project_id: &str,
        resource_path: &str,
    ) -> Result<serde_json::Value> {
        use serde_json::{Value, json};
        let project_id = project_id.to_string();
        if let Some(page) = parse_project_resource_page_request(resource_path)? {
            return Ok(match page.kind {
                PaginatedProjectResourceKind::ResearchLog => {
                    let entries = self
                        .repository
                        .list_research_logs_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(|entry| {
                            json!({
                                "id": entry.id,
                                "project_id": entry.project_id,
                                "query": entry.query,
                                "context_hint": entry.context_hint,
                                "model": entry.model,
                                "response": entry.response,
                                "context_summary": entry.context_summary,
                                "created_at": entry.created_at.to_rfc3339(),
                            })
                        })
                        .collect::<Vec<_>>();
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
                PaginatedProjectResourceKind::Conflicts => {
                    let entries = self
                        .repository
                        .list_conflicts_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "project_id": c.project_id,
                                "name": c.name,
                                "conflict_type": c.conflict_type,
                                "stakes": c.stakes,
                                "escalation_stages": c.escalation_stages,
                                "expected_total_cycles": c.expected_total_cycles,
                                "try_fail_cycles": c.try_fail_cycles,
                                "stated_consequences": c.stated_consequences,
                                "resolution_summary": c.resolution_summary,
                                "notes": c.notes,
                            })
                        })
                        .collect::<Vec<_>>();
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
                PaginatedProjectResourceKind::FutureKnowledge => {
                    let entries = self
                        .repository
                        .list_future_knowledge_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(future_knowledge_to_json)
                        .collect::<Vec<_>>();
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
                PaginatedProjectResourceKind::TimelineEvents => {
                    let entries = self
                        .repository
                        .list_timeline_events_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(timeline_event_to_json)
                        .collect::<Vec<_>>();
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
                PaginatedProjectResourceKind::DualPersonaReviews => {
                    let entries = self
                        .repository
                        .list_dual_persona_reviews_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(persisted_dual_persona_review)
                        .collect::<anyhow::Result<Vec<_>>>()?
                        .into_iter()
                        .map(serde_json::to_value)
                        .collect::<Result<Vec<_>, _>>()?;
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
                PaginatedProjectResourceKind::Relationships => {
                    let active_branch = self.repository.get_active_branch(&project_id).await?;
                    let entries = self
                        .repository
                        .list_relationships_by_branch(&active_branch.id)
                        .await?
                        .into_iter()
                        .map(relates_to_json)
                        .collect::<Vec<_>>();
                    let mut extra = serde_json::Map::new();
                    extra.insert("active_branch_id".to_string(), json!(active_branch.id));
                    paginated_project_resource_response(&project_id, page, entries, Some(extra))
                }
                PaginatedProjectResourceKind::TemporalInterventions => {
                    let entries = self
                        .repository
                        .list_temporal_interventions_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(temporal_intervention_to_json)
                        .collect::<Vec<_>>();
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
            });
        }
        if resource_path == "style-profiles" {
            let profiles = self.repository.list_style_profiles(&project_id).await?;
            let summaries: Vec<serde_json::Value> = profiles
                .into_iter()
                .map(|p| {
                    serde_json::json!({
                        "profile_id": p.profile_id,
                        "project_id": p.project_id,
                        "name": p.name,
                        "status": p.status,
                        "created_at": p.created_at,
                        "updated_at": p.updated_at,
                        "corpus": {
                            "source_count": p.corpus.source_count,
                            "analyzed_source_count": p.corpus.analyzed_source_count,
                            "skipped_source_count": p.corpus.skipped_source_count,
                            "total_words": p.corpus.total_words,
                            "total_characters": p.corpus.total_characters,
                            "chunk_count": p.corpus.chunk_count,
                            "warnings": p.corpus.warnings,
                        },
                        "metrics": p.metrics,
                        "guidance": p.guidance,
                        "source_policy": p.source_policy,
                        "quality": p.quality,
                        "archived_at": p.archived_at,
                    })
                })
                .collect();
            return Ok(serde_json::to_value(summaries)?);
        }
        if resource_path == "style-profile-applications" {
            let apps = self
                .repository
                .list_style_profile_applications(&project_id)
                .await?;
            return Ok(serde_json::to_value(apps)?);
        }
        if resource_path == "style-revision-patch-audits" {
            let audits = self
                .repository
                .list_style_revision_patch_audits(&project_id)
                .await?;
            return Ok(serde_json::to_value(audits)?);
        }
        if let Some(rest) = resource_path.strip_prefix("style-profiles/") {
            if let Some((profile_id, "sources")) = rest.split_once('/') {
                let profile = self
                    .repository
                    .get_style_profile(&project_id, profile_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("style profile not found: {}", profile_id))?;
                return Ok(serde_json::to_value(profile.corpus.source_refs)?);
            } else if let Some((profile_id, "refresh-preview")) = rest.split_once('/') {
                let out = self
                    .refresh_style_profile(spindle_core::style::RefreshStyleProfileInput {
                        project_id: project_id.clone(),
                        profile_id: profile_id.to_string(),
                        apply_after_refresh: None,
                        force_apply: None,
                        metrics_only: None,
                        dismiss_candidate_ids: Vec::new(),
                        dry_run: true,
                    })
                    .await?;
                return Ok(serde_json::to_value(out.preview)?);
            } else {
                let profile = self
                    .repository
                    .get_style_profile(&project_id, rest)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("style profile not found: {}", rest))?;
                return Ok(serde_json::to_value(profile)?);
            }
        }
        if resource_path == "continuity/health" {
            return self.continuity_health_resource(&project_id).await;
        }
        if let Some(chapter_path) = resource_path.strip_prefix("chapters/") {
            let parts = chapter_path.split('/').collect::<Vec<_>>();
            if parts.len() == 3 && parts[2] == "scenes" {
                let book_number = parts[0].parse::<i32>().with_context(|| {
                    format!("invalid book number in resource path: {}", parts[0])
                })?;
                let chapter_number = parts[1].parse::<i32>().with_context(|| {
                    format!("invalid chapter number in resource path: {}", parts[1])
                })?;
                let active_branch = self.repository.get_active_branch(&project_id).await?;
                let chapter = self
                    .repository
                    .get_chapter_by_number(&project_id, book_number, chapter_number)
                    .await?;
                let scenes = self
                    .repository
                    .list_scenes_by_project_and_branch(&project_id, &active_branch.id)
                    .await?
                    .into_iter()
                    .filter(|scene| {
                        scene.book_number == book_number && scene.chapter_number == chapter_number
                    })
                    .map(|scene| {
                        json!({
                            "id": scene.id,
                            "project_id": scene.project_id,
                            "branch_id": scene.branch_id,
                            "book_id": scene.book_id,
                            "chapter_id": scene.chapter_id,
                            "book_number": scene.book_number,
                            "chapter_number": scene.chapter_number,
                            "scene_order": scene.scene_order,
                            "summary": scene.summary,
                            "content_rating": scene.content_rating,
                            "tone": scene.tone,
                        })
                    })
                    .collect::<Vec<_>>();
                return Ok(json!({
                    "active_branch_id": active_branch.id,
                    "book_number": book_number,
                    "chapter_number": chapter_number,
                    "chapter_id": chapter.id,
                    "title": chapter.title.as_deref().unwrap_or(""),
                    "scenes": scenes,
                }));
            }
            anyhow::bail!(
                "chapter resource path must be chapters/<book_number>/<chapter_number>/scenes"
            );
        }
        if let Some(scene_path) = resource_path.strip_prefix("scene-move-impact/") {
            let parts = scene_path.split('/').collect::<Vec<_>>();
            if parts.len() != 6 {
                anyhow::bail!(
                    "scene move impact resource path must be from_book/from_chapter/from_scene/to_book/to_chapter/to_scene"
                );
            }
            let from_book_number = parts[0].parse::<i32>().with_context(|| {
                format!("invalid source book number in resource path: {}", parts[0])
            })?;
            let from_chapter_number = parts[1].parse::<i32>().with_context(|| {
                format!(
                    "invalid source chapter number in resource path: {}",
                    parts[1]
                )
            })?;
            let from_scene_order = parts[2].parse::<i32>().with_context(|| {
                format!("invalid source scene order in resource path: {}", parts[2])
            })?;
            let to_book_number = parts[3].parse::<i32>().with_context(|| {
                format!(
                    "invalid destination book number in resource path: {}",
                    parts[3]
                )
            })?;
            let to_chapter_number = parts[4].parse::<i32>().with_context(|| {
                format!(
                    "invalid destination chapter number in resource path: {}",
                    parts[4]
                )
            })?;
            let to_scene_order = parts[5].parse::<i32>().with_context(|| {
                format!(
                    "invalid destination scene order in resource path: {}",
                    parts[5]
                )
            })?;
            let impact = self
                .get_scene_move_impact(GetSceneMoveImpactInput {
                    project_id: project_id.clone(),
                    from_book_number,
                    from_chapter_number,
                    from_scene_order,
                    to_book_number,
                    to_chapter_number,
                    to_scene_order,
                })
                .await?;
            return Ok(serde_json::to_value(impact)?);
        }
        if let Some(scene_path) = resource_path.strip_prefix("scene-delete-impact/") {
            let parts = scene_path.split('/').collect::<Vec<_>>();
            if parts.len() != 3 {
                anyhow::bail!("scene delete impact resource path must be book/chapter/scene");
            }
            let book_number = parts[0]
                .parse::<i32>()
                .with_context(|| format!("invalid book number in resource path: {}", parts[0]))?;
            let chapter_number = parts[1].parse::<i32>().with_context(|| {
                format!("invalid chapter number in resource path: {}", parts[1])
            })?;
            let scene_order = parts[2]
                .parse::<i32>()
                .with_context(|| format!("invalid scene order in resource path: {}", parts[2]))?;
            let impact = self
                .get_scene_delete_impact(GetSceneDeleteImpactInput {
                    project_id: project_id.clone(),
                    book_number,
                    chapter_number,
                    scene_order,
                })
                .await?;
            return Ok(serde_json::to_value(impact)?);
        }
        if resource_path == "imports" {
            let sessions = self
                .repository
                .list_import_sessions_by_project(&project_id)
                .await?
                .into_iter()
                .map(|session| crate::sqlite::import_service::import_session_summary(&session))
                .collect::<anyhow::Result<Vec<_>>>()?;
            return Ok(serde_json::to_value(sessions)?);
        }
        if let Some(session_path) = resource_path.strip_prefix("imports/") {
            use spindle_core::models::ImportStatusInput;
            if !session_path.contains('/') {
                let status = self
                    .import_status(ImportStatusInput {
                        project_id: project_id.clone(),
                        session_id: session_path.to_string(),
                    })
                    .await?;
                return Ok(serde_json::to_value(status.session)?);
            }
            let (session_id, nested_path) = session_path
                .split_once('/')
                .context("import resource is missing a nested path")?;
            let status = self
                .import_status(ImportStatusInput {
                    project_id: project_id.clone(),
                    session_id: session_id.to_string(),
                })
                .await?;
            return Ok(match nested_path {
                "summary" => serde_json::to_value(status.session)?,
                "structure" => serde_json::to_value(status.structure)?,
                "entity-extraction" => serde_json::to_value(status.entity_extraction)?,
                "entity-consolidation" => serde_json::to_value(status.entity_consolidation)?,
                "characters" => serde_json::to_value(status.characters)?,
                "world" => serde_json::to_value(status.world)?,
                "narrative" => serde_json::to_value(status.narrative)?,
                "resume-snapshot" => serde_json::to_value(status.final_state)?,
                "review-items" => serde_json::to_value(status.review_items)?,
                "hydration-report" => serde_json::to_value(status.hydration_report)?,
                _ => anyhow::bail!("unknown import resource path: {resource_path}"),
            });
        }
        if let Some(scene_id) = resource_path.strip_prefix("research/usage/") {
            let usages = self
                .list_research_usages_for_scene(&project_id, scene_id)
                .await?;
            return Ok(serde_json::to_value(usages)?);
        }
        let active_branch = self.repository.get_active_branch(&project_id).await?;
        Ok(match resource_path {
            "books" => {
                let books = self.repository.list_books_by_project(&project_id).await?;
                let scenes = self
                    .repository
                    .list_scenes_by_project_and_branch(&project_id, &active_branch.id)
                    .await?;
                let mut items = Vec::new();
                for book in books {
                    let chapters = self
                        .repository
                        .list_chapters_by_book_number(&project_id, book.book_number)
                        .await?;
                    let scene_count = scenes
                        .iter()
                        .filter(|scene| scene.book_number == book.book_number)
                        .count();
                    items.push(json!({
                        "id": book.id,
                        "project_id": book.project_id,
                        "book_number": book.book_number,
                        "title": book.title,
                        "chapter_count": chapters.len(),
                        "scene_count": scene_count,
                    }));
                }
                Value::Array(items)
            }
            "chapters" => {
                let books = self.repository.list_books_by_project(&project_id).await?;
                let scenes = self
                    .repository
                    .list_scenes_by_project_and_branch(&project_id, &active_branch.id)
                    .await?;
                let mut chapters_out = Vec::new();
                for book in books {
                    for chapter in self
                        .repository
                        .list_chapters_by_book_number(&project_id, book.book_number)
                        .await?
                    {
                        let chapter_scenes = scenes
                            .iter()
                            .filter(|scene| {
                                scene.book_number == chapter.book_number
                                    && scene.chapter_number == chapter.chapter_number
                            })
                            .collect::<Vec<_>>();
                        chapters_out.push(json!({
                            "id": chapter.id,
                            "project_id": chapter.project_id,
                            "book_id": chapter.book_id,
                            "book_number": chapter.book_number,
                            "chapter_number": chapter.chapter_number,
                            "title": chapter.title,
                            "scene_count": chapter_scenes.len(),
                            "scene_orders": chapter_scenes
                                .iter()
                                .map(|scene| scene.scene_order)
                                .collect::<Vec<_>>(),
                        }));
                    }
                }
                Value::Array(chapters_out)
            }
            "characters" => {
                let records = self
                    .repository
                    .list_characters_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "project_id": c.project_id,
                                "branch_id": c.branch_id,
                                "name": c.name,
                                "role": c.role,
                                "summary": c.summary,
                                "realm": c.realm,
                                "appearance": c.appearance,
                                "notes": c.notes,
                            })
                        })
                        .collect(),
                )
            }
            "locations" => {
                let records = self
                    .repository
                    .list_locations_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|l| {
                            json!({
                                "id": l.id,
                                "project_id": l.project_id,
                                "branch_id": l.branch_id,
                                "name": l.name,
                                "kind": l.kind,
                                "realm": l.realm,
                                "summary": l.summary,
                                "notes": l.notes,
                            })
                        })
                        .collect(),
                )
            }
            "world-rules" => {
                let records = self
                    .repository
                    .list_world_rules_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|r| {
                            json!({
                                "id": r.id,
                                "project_id": r.project_id,
                                "rule_name": r.rule_name,
                                "rule_type": r.rule_type,
                                "description": r.description,
                                "scan_pattern": r.scan_pattern,
                                "relevance_tags": r.relevance_tags.unwrap_or_default(),
                                "established_in": r.established_in.map(|e| json!({
                                    "book_number": e.book_number,
                                    "chapter_number": e.chapter_number,
                                    "note": e.note,
                                })),
                                "notes": r.notes,
                            })
                        })
                        .collect(),
                )
            }
            "factions" => {
                let records = self
                    .repository
                    .list_factions_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|f| {
                            json!({
                                "id": f.id,
                                "project_id": f.project_id,
                                "name": f.name,
                                "faction_type": f.faction_type,
                                "realm": f.realm,
                                "summary": f.summary,
                                "tags": f.tags,
                                "notes": f.notes,
                            })
                        })
                        .collect(),
                )
            }
            "plot-lines" => {
                let records = self
                    .repository
                    .list_plot_lines_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|p| {
                            json!({
                                "id": p.id,
                                "project_id": p.project_id,
                                "name": p.name,
                                "plot_type": p.plot_type,
                                "summary": p.summary,
                                "status": p.status,
                                "convergence_points": p.convergence_points,
                                "notes": p.notes,
                            })
                        })
                        .collect(),
                )
            }
            "conflicts" => {
                let records = self
                    .repository
                    .list_conflicts_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "project_id": c.project_id,
                                "name": c.name,
                                "conflict_type": c.conflict_type,
                                "stakes": c.stakes,
                                "escalation_stages": c.escalation_stages,
                                "expected_total_cycles": c.expected_total_cycles,
                                "try_fail_cycles": c.try_fail_cycles,
                                "stated_consequences": c.stated_consequences,
                                "resolution_summary": c.resolution_summary,
                                "notes": c.notes,
                            })
                        })
                        .collect(),
                )
            }
            "themes" => {
                let records = self.repository.list_themes_by_project(&project_id).await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "project_id": t.project_id,
                                "theme_statement": t.theme_statement,
                                "thesis_antithesis": t.thesis_antithesis,
                                "introduction_point": t.introduction_point,
                                "resolution_point": t.resolution_point,
                                "notes": t.notes,
                            })
                        })
                        .collect(),
                )
            }
            "motifs" => {
                let records = self.repository.list_motifs_by_project(&project_id).await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|m| {
                            json!({
                                "id": m.id,
                                "project_id": m.project_id,
                                "name": m.name,
                                "description": m.description,
                                "max_uses_per_chapter": m.max_uses_per_chapter,
                                "connected_theme_ids": m.connected_theme_ids,
                                "notes": m.notes,
                            })
                        })
                        .collect(),
                )
            }
            "narrative-promises" => {
                let records = self
                    .repository
                    .list_narrative_promises_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|p| {
                            json!({
                                "id": p.id,
                                "project_id": p.project_id,
                                "promise_type": p.promise_type,
                                "description": p.description,
                                "status": p.status,
                                "planted_at": p.planted_at,
                                "planned_payoff": p.planned_payoff,
                                "status_history": p.status_history,
                                "notes": p.notes,
                            })
                        })
                        .collect(),
                )
            }
            "pacing/overview" => {
                let records = self
                    .repository
                    .list_pacing_trackers_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|p| {
                            json!({
                                "id": p.id,
                                "project_id": p.project_id,
                                "character_arc_id": p.character_arc_id,
                                "per_book_budget": p.per_book_budget,
                                "max_progress_per_chapter": p.max_progress_per_chapter,
                                "milestone_spacing": p.milestone_spacing,
                                "sprint_allowance": p.sprint_allowance,
                                "regression_budget": p.regression_budget,
                                "current_progress": p.current_progress,
                                "budget_remaining": p.budget_remaining,
                                "velocity": p.velocity,
                                "status": p.status,
                                "next_milestone": p.next_milestone,
                                "warnings": p.warnings,
                            })
                        })
                        .collect(),
                )
            }
            "chapter-summaries" => {
                let records = self
                    .repository
                    .list_chapter_summaries_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|summary| {
                            json!({
                                "id": summary.id,
                                "project_id": summary.project_id,
                                "branch_id": summary.branch_id,
                                "book_number": summary.book_number,
                                "chapter_number": summary.chapter_number,
                                "summary": summary.summary,
                                "key_events": summary.key_events,
                                "character_changes": summary.character_changes,
                                "relationship_shifts": summary.relationship_shifts,
                                "arc_advances": summary.arc_advances,
                                "promise_events": summary.promise_events,
                            })
                        })
                        .collect(),
                )
            }
            "reader-contract" => serde_json::to_value(
                self.repository
                    .get_project(&project_id)
                    .await?
                    .reader_contract
                    .into_core(),
            )?,
            "branches" => {
                let project = self.repository.get_project(&project_id).await?;
                let active_branch_id = project.active_branch_id;
                let branches = self
                    .repository
                    .list_branches_by_project(&project_id)
                    .await?
                    .into_iter()
                    .map(|branch| {
                        crate::format::branch_summary(&branch, active_branch_id.as_deref())
                    })
                    .collect::<Vec<_>>();
                serde_json::to_value(branches)?
            }
            "future-knowledge" => {
                let records = self
                    .repository
                    .list_future_knowledge_by_project(&project_id)
                    .await?;
                Value::Array(records.into_iter().map(future_knowledge_to_json).collect())
            }
            "timeline-events" => {
                let records = self
                    .repository
                    .list_timeline_events_by_project(&project_id)
                    .await?;
                Value::Array(records.into_iter().map(timeline_event_to_json).collect())
            }
            "temporal-interventions" => {
                let records = self
                    .repository
                    .list_temporal_interventions_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(temporal_intervention_to_json)
                        .collect(),
                )
            }
            "system-overlays" => {
                let records = self
                    .repository
                    .list_system_overlays_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|s| {
                            json!({
                                "id": s.id,
                                "system_name": s.system_name,
                                "system_type": s.system_type,
                                "rules": s.rules,
                                "visibility": s.visibility,
                                "progression_currency": s.progression_currency,
                                "stats": s.stats,
                                "advancement_tiers": s.advancement_tiers,
                            })
                        })
                        .collect(),
                )
            }
            "dual-persona-reviews" => serde_json::to_value(
                self.repository
                    .list_dual_persona_reviews_by_project(&project_id)
                    .await?
                    .into_iter()
                    .map(persisted_dual_persona_review)
                    .collect::<anyhow::Result<Vec<_>>>()?,
            )?,
            "religions" => {
                let records = self
                    .repository
                    .list_religions_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|r| {
                            json!({
                                "id": r.id,
                                "project_id": r.project_id,
                                "name": r.name,
                                "deity_or_principle": r.deity_or_principle,
                                "summary": r.summary,
                                "tags": r.tags,
                                "notes": r.notes,
                            })
                        })
                        .collect(),
                )
            }
            "economies" => {
                let records = self
                    .repository
                    .list_economies_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|e| {
                            json!({
                                "id": e.id,
                                "project_id": e.project_id,
                                "name": e.name,
                                "realm": e.realm,
                                "summary": e.summary,
                                "scarce_resources": e.scarce_resources,
                                "trade_goods": e.trade_goods,
                                "currency": e.currency,
                                "notes": e.notes,
                            })
                        })
                        .collect(),
                )
            }
            "terms" => {
                let records = self.repository.list_terms_by_project(&project_id).await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "project_id": t.project_id,
                                "term_text": t.term_text,
                                "pronunciation": t.pronunciation,
                                "definition": t.definition,
                                "usage_context": t.usage_context,
                                "origin": t.origin,
                                "notes": t.notes,
                            })
                        })
                        .collect(),
                )
            }
            "character-arcs" => {
                let records = self
                    .repository
                    .list_character_arcs_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|a| {
                            json!({
                                "id": a.id,
                                "project_id": a.project_id,
                                "character_id": a.character_id,
                                "arc_type": a.arc_type,
                                "starting_state": a.starting_state,
                                "ending_state": a.ending_state,
                                "milestones": a.milestones,
                                "thematic_purpose": a.thematic_purpose,
                                "connected_theme_ids": a.connected_theme_ids,
                                "status": a.status,
                                "progress": a.progress,
                                "notes": a.notes,
                            })
                        })
                        .collect(),
                )
            }
            "relationships" => {
                let branch = self.repository.get_active_branch(&project_id).await?;
                let records = self
                    .repository
                    .list_relationships_by_branch(&branch.id)
                    .await?;
                Value::Array(records.into_iter().map(relates_to_json).collect())
            }
            "research" => {
                let sources = self
                    .repository
                    .list_research_sources_by_project(&project_id)
                    .await?;
                let notes = self
                    .repository
                    .list_research_notes_by_project(&project_id)
                    .await?;
                let claims = self
                    .repository
                    .list_research_claims_by_project(&project_id)
                    .await?;
                json!({
                    "sources": sources.into_iter().map(|s| {
                        json!({
                            "id": s.id,
                            "project_id": s.project_id,
                            "branch_id": s.branch_id,
                            "title": s.title,
                            "source_type": s.source_type,
                            "url": s.url,
                            "file_path": s.file_path,
                            "author": s.author,
                            "publisher": s.publisher,
                            "published_date": s.published_date,
                            "accessed_at": crate::sqlite::row::timestamp_to_micros(s.accessed_at),
                            "reliability": s.reliability,
                            "tags": s.tags,
                            "summary": s.summary,
                            "created_at": crate::sqlite::row::timestamp_to_micros(s.created_at),
                            "updated_at": crate::sqlite::row::timestamp_to_micros(s.updated_at),
                        })
                    }).collect::<Vec<_>>(),
                    "notes": notes.into_iter().map(|n| {
                        json!({
                            "id": n.id,
                            "project_id": n.project_id,
                            "source_id": n.source_id,
                            "branch_id": n.branch_id,
                            "note": n.note,
                            "quote": n.quote,
                            "locator": n.locator,
                            "tags": n.tags,
                            "created_at": crate::sqlite::row::timestamp_to_micros(n.created_at),
                            "updated_at": crate::sqlite::row::timestamp_to_micros(n.updated_at),
                        })
                    }).collect::<Vec<_>>(),
                    "claims": claims.into_iter().map(|c| {
                        json!({
                            "id": c.id,
                            "project_id": c.project_id,
                            "source_id": c.source_id,
                            "note_id": c.note_id,
                            "branch_id": c.branch_id,
                            "claim": c.claim,
                            "topic": c.topic,
                            "time_period": c.time_period,
                            "location": c.location,
                            "confidence": c.confidence,
                            "tags": c.tags,
                            "created_at": crate::sqlite::row::timestamp_to_micros(c.created_at),
                            "updated_at": crate::sqlite::row::timestamp_to_micros(c.updated_at),
                        })
                    }).collect::<Vec<_>>(),
                })
            }
            "research/tags" => {
                let tags = self
                    .repository
                    .research_tags_by_project(&project_id)
                    .await?;
                json!(tags)
            }
            "research/usage" => {
                let usages = self.list_research_usages_by_project(&project_id).await?;
                serde_json::to_value(usages)?
            }
            _ => anyhow::bail!("unknown project resource path: {resource_path}"),
        })
    }

    /// Polymorphic read by record id. Most entity types return a minimal JSON
    /// envelope with `{id, table}` if the entity exists, errors otherwise.
    /// Scene reads are intentionally richer because authoring agents use
    /// `bible://scene:<id>` to continue from already-saved prose.
    pub async fn read_entity_by_id(&self, record_id_str: &str) -> Result<serde_json::Value> {
        let (table, _key) = record_id_str
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("expected `table:id` format, got `{record_id_str}`"))?;
        if table == "scene" {
            let scene = self.repository.get_scene(record_id_str).await?;
            let word_count = scene.full_text.split_whitespace().count();
            return Ok(serde_json::json!({
                "id": scene.id,
                "table": "scene",
                "project_id": scene.project_id,
                "branch_id": scene.branch_id,
                "book_id": scene.book_id,
                "chapter_id": scene.chapter_id,
                "book_number": scene.book_number,
                "chapter_number": scene.chapter_number,
                "scene_order": scene.scene_order,
                "full_text": scene.full_text,
                "summary": scene.summary,
                "content_rating": scene.content_rating,
                "tone": scene.tone,
                "draft_origin": scene.draft_origin,
                "word_count": word_count,
                "created_at": scene.created_at.to_rfc3339(),
                "updated_at": scene.updated_at.to_rfc3339(),
            }));
        }
        let not_found = || anyhow::anyhow!("entity {record_id_str} not found");
        let project_id = match table {
            "project" => {
                self.repository
                    .get_project(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .id
            }
            "book" => {
                self.repository
                    .get_book(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "chapter" => {
                self.repository
                    .get_chapter(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "character" => {
                self.repository
                    .get_character(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            // Keyed 1:1 by character_id with no project_id column of its own;
            // resolve the project through the owning character so update_entity
            // locks the owning project rather than the profile id.
            "character_emotional_profile" => {
                let profile = self
                    .repository
                    .get_character_emotional_profile_by_id(record_id_str)
                    .await
                    .map_err(|_| not_found())?;
                self.repository
                    .get_character(&profile.character_id)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "location" => {
                self.repository
                    .get_location(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "bible_branch" => self
                .repository
                .get_branch(record_id_str)
                .await
                .map_err(|_| not_found())?
                .project_id
                .ok_or_else(not_found)?,
            "faction" => {
                self.repository
                    .get_faction(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "religion" => {
                self.repository
                    .get_religion(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "economy" => {
                self.repository
                    .get_economy(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "term" => {
                self.repository
                    .get_term(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "plot_line" => {
                self.repository
                    .get_plot_line(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "conflict" => {
                self.repository
                    .get_conflict(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "theme" => {
                self.repository
                    .get_theme(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "motif" => {
                self.repository
                    .get_motif(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "narrative_promise" => {
                self.repository
                    .get_narrative_promise(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "world_rule" => {
                self.repository
                    .get_world_rule(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "character_arc" => {
                self.repository
                    .get_character_arc(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "save_point" => {
                self.repository
                    .get_save_point(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "canonical_fact" => {
                self.repository
                    .get_canonical_fact(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "knowledge_fact" => {
                self.repository
                    .get_knowledge_fact(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "system_overlay" => {
                self.repository
                    .get_system_overlay(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "timeline_event" => {
                self.repository
                    .get_timeline_event(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "temporal_intervention" => {
                self.repository
                    .get_temporal_intervention(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "future_knowledge" => {
                self.repository
                    .get_future_knowledge(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "revision_marker" => {
                self.repository
                    .get_revision_marker(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .project_id
            }
            "research_source" => {
                self.repository
                    .get_research_source(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .ok_or_else(not_found)?
                    .project_id
            }
            "research_note" => {
                self.repository
                    .get_research_note(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .ok_or_else(not_found)?
                    .project_id
            }
            "research_claim" => {
                self.repository
                    .get_research_claim(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .ok_or_else(not_found)?
                    .project_id
            }
            "research_usage" => {
                self.repository
                    .get_research_usage(record_id_str)
                    .await
                    .map_err(|_| not_found())?
                    .ok_or_else(not_found)?
                    .project_id
            }
            other => anyhow::bail!("read_entity_by_id does not support table `{other}`"),
        };
        Ok(serde_json::json!({
            "id": record_id_str,
            "table": table,
            "project_id": project_id,
        }))
    }
}
