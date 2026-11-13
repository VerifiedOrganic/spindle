use super::*;

impl SqliteSpindleService {
    /// Route-based research lookup that asks the configured research agent
    /// for structured source/note/claim JSON, optionally persists it to the
    /// project-local research library, and records the raw model output in
    /// `research_log`.
    pub async fn research_query(
        &self,
        input: spindle_core::models::ResearchQueryInput,
    ) -> Result<spindle_core::models::ResearchQueryOutput> {
        use crate::ai::ModelRequest;
        use spindle_core::models::SourcePolicy;

        let project = self.repository.get_project(&input.project_id).await?;

        let branch_id = self
            .resolve_research_branch_id(&input.project_id, input.branch_id.as_deref())
            .await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let resolved_branch_id = branch_id.clone().unwrap_or(active_branch.id);

        let mut warnings = Vec::new();
        let input_tags = input
            .tags
            .as_deref()
            .map(normalize_tags)
            .unwrap_or_default();
        let is_explicit_rating = input
            .rating
            .as_deref()
            .is_some_and(|rating| rating.trim().eq_ignore_ascii_case("explicit"));

        // 1. Perform rating and route configurations check
        if is_explicit_rating {
            let bindings = self.repository.model_router().list_route_bindings()?;
            let has_explicit_route = bindings.iter().any(|b| {
                b.route.route_name == "research"
                    && b.rating
                        .as_deref()
                        .is_some_and(|rating| rating.trim().eq_ignore_ascii_case("explicit"))
            });
            if !has_explicit_route {
                anyhow::bail!(
                    "explicit rating requested but no explicit research route configured; add a `[[routing]]` entry with `route = \"research\"` and `rating = \"explicit\"`"
                );
            }

            // Check if query has adult-only boundary language
            let query_lower = input.query.to_lowercase();
            let has_boundary = query_lower.contains("factual")
                || query_lower.contains("historical")
                || query_lower.contains("sociological")
                || query_lower.contains("educational")
                || query_lower.contains("anthropological")
                || query_lower.contains("non-prose")
                || query_lower.contains("academic")
                || query_lower.contains("boundaries")
                || query_lower.contains("consent")
                || query_lower.contains("medical")
                || query_lower.contains("culture")
                || query_lower.contains("history");
            if !has_boundary {
                warnings.push(
                    "adult/explicit query without adult-only boundary language in the prompt"
                        .to_string(),
                );
            }
        }

        // 2. Resolve scene tags if scene_id is provided
        let mut scene_tags = Vec::new();
        if let Some(scene_id) = &input.scene_id {
            let scene = self.repository.get_scene(scene_id).await?;
            if scene.project_id != input.project_id {
                anyhow::bail!("scene does not belong to the requested project");
            }
            let plans = self
                .repository
                .list_chapter_plans_by_project(&input.project_id)
                .await?;
            if let Some(tags) = plans
                .into_iter()
                .find(|p| {
                    p.book_number == scene.book_number && p.chapter_number == scene.chapter_number
                })
                .and_then(|plan| {
                    plan.scenes
                        .into_iter()
                        .find(|s| s.scene_order == scene.scene_order)
                })
                .map(|planned_scene| planned_scene.research_tags)
            {
                scene_tags.extend(tags);
            }
        }
        scene_tags = normalize_tags(&scene_tags);

        // 3. Build the prompt
        let mut user_message = match &input.context_hint {
            Some(hint) => format!(
                "Context: {hint}\n\nQuestion: {}\n\n{}",
                input.query, STRUCTURED_RESEARCH_PROMPT_INSTRUCTIONS
            ),
            None => format!(
                "Question: {}\n\n{}",
                input.query, STRUCTURED_RESEARCH_PROMPT_INSTRUCTIONS
            ),
        };
        if !input_tags.is_empty() || !scene_tags.is_empty() {
            user_message.push_str(&format!(
                "\n\nRequired classification tags: {:?}",
                [&input_tags[..], &scene_tags[..]].concat()
            ));
        }

        let prompt_to_send = if is_explicit_rating {
            format!(
                "{}\n\nSAFETY/BOUNDARY INSTRUCTIONS:\n{}",
                user_message, EXPLICIT_RESEARCH_SYSTEM_APPENDIX
            )
        } else {
            user_message
        };

        let m_request = ModelRequest {
            route: "research".to_string(),
            prompt: prompt_to_send,
            rating: input.rating.clone(),
            context: Some(crate::ai::RequestContext {
                project_id: Some(input.project_id.clone()),
                book_id: None,
                chapter_id: None,
                scene_id: input.scene_id.clone(),
                ..Default::default()
            }),
        };

        // 4. Execute routed model completion
        let complete_res = match self.repository.model_router().complete(&m_request).await {
            Ok(res) => res,
            Err(e) => {
                if e.to_string().contains("unknown model route")
                    || e.to_string().contains("unsupported model adapter")
                {
                    anyhow::bail!(
                        "No research route is configured for Spindle. Please add a `[[routing]]` entry with `route = \"research\"` to your project-local `.spindle/config.toml`."
                    );
                } else {
                    return Err(e);
                }
            }
        };

        let raw_response = complete_res.output;
        let cleaned = clean_json_response(&raw_response);
        let parsed: Result<ResearchQueryOutputJson, _> = serde_json::from_str(cleaned);

        let mut parsed_data = match parsed {
            Ok(data) => data,
            Err(err) => {
                // Live-run bug 5: on parse failure we used to insert a junk
                // "Unparsed Lead" claim whose body was the ENTIRE raw model
                // output (tagged `unparsed`, confidence `uncertain`) whenever
                // source_policy was AllowUnsourcedLeads, and return
                // success-shaped output. That poisoned research_pack_for_scene
                // and every research consumer with unstructured prose. Fail
                // honestly instead — for BOTH source policies — with zero DB
                // writes (this bail precedes every persistence call, and the
                // research-log write below it, so nothing is stored). The caller
                // can re-run; no structured research is fabricated from
                // unparseable output.
                anyhow::bail!(
                    "research model returned unparseable output for query \"{}\": {}. \
                     No research was ingested. Raw response: {}",
                    input.query,
                    err,
                    raw_response
                );
            }
        };
        extend_unique_tags(&mut parsed_data.tags, &input_tags);
        extend_unique_tags(&mut parsed_data.tags, &scene_tags);
        if is_explicit_rating {
            extend_unique_tags(
                &mut parsed_data.tags,
                &["explicit".to_string(), "adult".to_string()],
            );
        }

        // 5. Warnings and validations on parsed data
        if is_explicit_rating {
            let mut total_provenance = false;
            for claim in &parsed_data.claims {
                let has_prov = claim
                    .note_index
                    .and_then(|idx| parsed_data.notes.get(idx))
                    .and_then(|n| n.source_index)
                    .is_some();
                if has_prov {
                    total_provenance = true;
                } else {
                    warnings.push(format!(
                        "explicit research claim contains no source provenance: \"{}\"",
                        claim.claim
                    ));
                }
            }
            if !total_provenance && !parsed_data.claims.is_empty() {
                warnings.push("explicit research result contains no provenance".to_string());
            }
        }

        if is_explicit_rating {
            for claim in &parsed_data.claims {
                if is_prose_like(&claim.claim) {
                    warnings.push(format!(
                        "explicit research attempting to save prose-like material as a research claim: \"{}\"",
                        claim.claim
                    ));
                }
            }
        }

        let classification_tags = [&input_tags[..], &scene_tags[..]].concat();

        let mut source_ids = Vec::new();
        let mut note_ids = Vec::new();

        // 6. Persist to database if store is true
        if input.store {
            for src in &parsed_data.sources {
                let mut src_tags = src.tags.clone();
                extend_unique_tags(&mut src_tags, &classification_tags);
                if is_explicit_rating {
                    if !has_tag(&src_tags, "explicit") {
                        src_tags.push("explicit".to_string());
                    }
                    if !has_tag(&src_tags, "adult") {
                        src_tags.push("adult".to_string());
                    }
                }
                let add_src_out = self
                    .research_add_source(spindle_core::models::ResearchAddSourceInput {
                        project_id: input.project_id.clone(),
                        branch_id: Some(resolved_branch_id.clone()),
                        title: src.title.clone(),
                        source_type: src.source_type.clone(),
                        url: src.url.clone(),
                        file_path: None,
                        author: src.author.clone(),
                        publisher: src.publisher.clone(),
                        published_date: src.published_date.clone(),
                        accessed_at: None,
                        reliability: src.reliability.clone(),
                        tags: src_tags,
                        summary: src.summary.clone(),
                    })
                    .await?;
                source_ids.push(add_src_out.source_id);
            }

            for note in &parsed_data.notes {
                let note_source_id = note
                    .source_index
                    .and_then(|idx| source_ids.get(idx).cloned());
                let mut note_tags = note.tags.clone();
                extend_unique_tags(&mut note_tags, &classification_tags);
                if is_explicit_rating {
                    if !has_tag(&note_tags, "explicit") {
                        note_tags.push("explicit".to_string());
                    }
                    if !has_tag(&note_tags, "adult") {
                        note_tags.push("adult".to_string());
                    }
                }
                let add_note_out = self
                    .research_add_note(spindle_core::models::ResearchAddNoteInput {
                        project_id: input.project_id.clone(),
                        source_id: note_source_id,
                        branch_id: Some(resolved_branch_id.clone()),
                        note: note.note.clone(),
                        quote: note.quote.clone(),
                        locator: note.locator.clone(),
                        tags: note_tags,
                    })
                    .await?;
                note_ids.push(add_note_out.note_id);
            }

            for claim in &parsed_data.claims {
                if is_explicit_rating && is_prose_like(&claim.claim) {
                    warnings.push(format!(
                        "explicit research claim skipped because it appears prose-like: \"{}\"",
                        claim.claim
                    ));
                    continue;
                }
                let linked_note_id = claim.note_index.and_then(|idx| note_ids.get(idx).cloned());
                let linked_source_id = claim
                    .note_index
                    .and_then(|idx| parsed_data.notes.get(idx))
                    .and_then(|n| n.source_index)
                    .and_then(|s_idx| source_ids.get(s_idx).cloned());
                let has_source_provenance = linked_source_id.is_some();
                let should_store = has_source_provenance
                    || input.source_policy.unwrap_or(SourcePolicy::RequireSources)
                        != SourcePolicy::RequireSources;
                if should_store {
                    let mut claim_tags = claim.tags.clone();
                    extend_unique_tags(&mut claim_tags, &classification_tags);
                    if is_explicit_rating {
                        if !has_tag(&claim_tags, "explicit") {
                            claim_tags.push("explicit".to_string());
                        }
                        if !has_tag(&claim_tags, "adult") {
                            claim_tags.push("adult".to_string());
                        }
                    }
                    self.research_add_claim(spindle_core::models::ResearchAddClaimInput {
                        project_id: input.project_id.clone(),
                        source_id: linked_source_id,
                        note_id: linked_note_id,
                        branch_id: Some(resolved_branch_id.clone()),
                        claim: claim.claim.clone(),
                        topic: claim.topic.clone(),
                        time_period: claim.time_period.clone(),
                        location: claim.location.clone(),
                        confidence: claim.confidence.clone(),
                        tags: claim_tags,
                    })
                    .await?;
                } else {
                    warnings.push(format!(
                        "research claim skipped because source_policy=require_sources and no source provenance was returned: \"{}\"",
                        claim.claim
                    ));
                }
            }
        }

        // 7. Write to the research log
        let context_summary = format!(
            "project={} | genre={} | rating={} | store={} | sources={} | notes={} | claims={}",
            project.name,
            project.genre,
            input.rating.as_deref().unwrap_or("none"),
            input.store,
            parsed_data.sources.len(),
            parsed_data.notes.len(),
            parsed_data.claims.len()
        );
        let log_model = complete_res.model_name;
        if let Err(error) = self
            .repository
            .create_research_log(
                &input.project_id,
                &input.query,
                input.context_hint.as_deref(),
                &log_model,
                &raw_response,
                &context_summary,
            )
            .await
        {
            eprintln!("failed to persist research log: {error}");
        }

        Ok(spindle_core::models::ResearchQueryOutput {
            summary: parsed_data.summary,
            sources: parsed_data.sources,
            notes: parsed_data.notes,
            claims: parsed_data.claims,
            tags: parsed_data.tags,
            warnings,
            uncertainty_level: parsed_data.uncertainty_level,
        })
    }

    pub async fn research_ingest_report(
        &self,
        input: spindle_core::models::ResearchIngestReportInput,
    ) -> Result<spindle_core::models::ResearchIngestReportOutput> {
        let prompt = format!(
            "Analyze and parse the following research report titled \"{}\" into the structured JSON research format. \
             Report contents:\n{}\n\nRequired tags: {:?}",
            input.title, input.report, input.tags
        );

        let query_in = spindle_core::models::ResearchQueryInput {
            project_id: input.project_id.clone(),
            query: prompt,
            context_hint: Some(format!("Parse report: {}", input.title)),
            branch_id: input.branch_id.clone(),
            rating: input.rating.clone(),
            tags: Some(input.tags.clone()),
            scene_id: None,
            store: true,
            source_policy: input.source_policy,
        };

        let out = self.research_query(query_in).await?;
        Ok(spindle_core::models::ResearchIngestReportOutput {
            summary: out.summary,
            sources: out.sources,
            notes: out.notes,
            claims: out.claims,
            tags: out.tags,
            warnings: out.warnings,
            uncertainty_level: out.uncertainty_level,
        })
    }

    pub async fn research_plan_for_scene(
        &self,
        input: spindle_core::models::ResearchPlanForSceneInput,
    ) -> Result<spindle_core::models::ResearchPlanForSceneOutput> {
        let scene = self.repository.get_scene(&input.scene_id).await?;
        let project_id = &input.project_id;
        if scene.project_id != *project_id {
            anyhow::bail!("scene does not belong to the requested project");
        }
        let branch_id = scene.branch_id.clone();

        let planned_scene_opt = if let Ok(plans) = self
            .repository
            .list_chapter_plans_by_project(project_id)
            .await
        {
            plans
                .into_iter()
                .find(|p| {
                    p.book_number == scene.book_number && p.chapter_number == scene.chapter_number
                })
                .and_then(|plan| {
                    plan.scenes
                        .into_iter()
                        .find(|s| s.scene_order == scene.scene_order)
                })
        } else {
            None
        };

        let mut research_required = false;
        let mut target_tags = Vec::new();
        let mut explicit_query = None;
        let scene_summary = input
            .planned_scene_summary
            .clone()
            .unwrap_or(scene.summary.clone());

        if let Some(planned_scene) = planned_scene_opt {
            research_required = planned_scene.research_required.unwrap_or(false);
            target_tags = planned_scene.research_tags;
            explicit_query = planned_scene.explicit_query;
        }

        if let Some(in_tags) = &input.tags {
            target_tags.extend(in_tags.clone());
        }
        if input.rating.is_some() {
            research_required = true;
        }

        let pack = self
            .research_pack_for_scene(spindle_core::models::ResearchPackForSceneInput {
                project_id: project_id.clone(),
                branch_id: Some(branch_id),
                scene_summary: Some(scene_summary.clone()),
                scene_location: None,
                character_ids: vec![],
                tags: target_tags.clone(),
                explicit_query: explicit_query.clone(),
                limit: Some(100),
            })
            .await?;

        let research_pack_empty =
            pack.sources.is_empty() && pack.notes.is_empty() && pack.claims.is_empty();

        let mut research_tags_matched = true;
        let mut missing_tags = Vec::new();
        if !target_tags.is_empty() {
            let mut found_any = false;
            for t in &target_tags {
                let t_lower = t.to_lowercase();
                let mut found_this = false;
                for s in &pack.sources {
                    if s.tags.iter().any(|st| st.to_lowercase() == t_lower) {
                        found_this = true;
                        break;
                    }
                }
                if !found_this {
                    for n in &pack.notes {
                        if n.tags.iter().any(|nt| nt.to_lowercase() == t_lower) {
                            found_this = true;
                            break;
                        }
                    }
                }
                if !found_this {
                    for c in &pack.claims {
                        if c.tags.iter().any(|ct| ct.to_lowercase() == t_lower) {
                            found_this = true;
                            break;
                        }
                    }
                }
                if found_this {
                    found_any = true;
                } else {
                    missing_tags.push(t.clone());
                }
            }
            research_tags_matched = found_any;
        }

        let await_research = (research_required && research_pack_empty)
            || (!target_tags.is_empty() && !research_tags_matched);

        let mut suggested_queries = Vec::new();
        if let Some(eq) = explicit_query {
            suggested_queries.push(eq);
        }
        for tag in &missing_tags {
            suggested_queries.push(format!(
                "historical context and factual details for {}",
                tag
            ));
        }
        if suggested_queries.is_empty() && !scene_summary.is_empty() {
            suggested_queries.push(format!(
                "factual research query for scene: {}",
                scene_summary
            ));
        }

        Ok(spindle_core::models::ResearchPlanForSceneOutput {
            suggested_queries,
            missing_tags,
            await_research,
        })
    }

    pub(super) async fn resolve_research_branch_id(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
    ) -> Result<Option<String>> {
        self.repository.get_project(project_id).await?;

        if let Some(branch_id) = branch_id {
            let branch = self
                .repository
                .get_branch(branch_id)
                .await
                .with_context(|| format!("branch not found: {branch_id}"))?;
            if branch.project_id.as_deref() != Some(project_id) {
                anyhow::bail!("branch does not belong to the requested project");
            }
            return Ok(Some(branch.id));
        }

        let active_branch = self.repository.get_active_branch(project_id).await?;
        Ok(Some(active_branch.id))
    }

    pub async fn research_add_source(
        &self,
        input: spindle_core::models::ResearchAddSourceInput,
    ) -> Result<spindle_core::models::ResearchAddSourceOutput> {
        let branch_id = self
            .resolve_research_branch_id(&input.project_id, input.branch_id.as_deref())
            .await?;
        let tags = normalize_tags(&input.tags);

        let accessed_at = match input.accessed_at {
            Some(micros) => match chrono::Utc.timestamp_micros(micros).single() {
                Some(ts) => ts,
                None => chrono::Utc::now(),
            },
            None => chrono::Utc::now(),
        };

        let branch_id_str = branch_id.as_deref();

        let source = self
            .repository
            .create_research_source(
                &input.project_id,
                branch_id_str,
                &input.title,
                &input.source_type,
                input.url.as_deref(),
                input.file_path.as_deref(),
                input.author.as_deref(),
                input.publisher.as_deref(),
                input.published_date.as_deref(),
                accessed_at,
                &input.reliability,
                &tags,
                input.summary.as_deref(),
            )
            .await?;

        Ok(spindle_core::models::ResearchAddSourceOutput {
            source_id: source.id,
        })
    }

    pub async fn research_add_note(
        &self,
        input: spindle_core::models::ResearchAddNoteInput,
    ) -> Result<spindle_core::models::ResearchAddNoteOutput> {
        let source = if let Some(source_id) = &input.source_id {
            let source = self
                .repository
                .get_research_source(source_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("source not found: {}", source_id))?;
            if source.project_id != input.project_id {
                anyhow::bail!("source does not belong to the requested project");
            }
            Some(source)
        } else {
            None
        };

        let tags = normalize_tags(&input.tags);

        let mut branch_id = self
            .resolve_research_branch_id(&input.project_id, input.branch_id.as_deref())
            .await?;
        if input.branch_id.is_none()
            && let Some(source_branch_id) =
                source.as_ref().and_then(|source| source.branch_id.clone())
        {
            branch_id = Some(source_branch_id);
        }
        if let Some(source_branch_id) = source
            .as_ref()
            .and_then(|source| source.branch_id.as_deref())
            && branch_id.as_deref() != Some(source_branch_id)
        {
            anyhow::bail!("source branch does not match the requested branch");
        }
        let branch_id_str = branch_id.as_deref();

        let note = self
            .repository
            .create_research_note(
                &input.project_id,
                input.source_id.as_deref(),
                branch_id_str,
                &input.note,
                input.quote.as_deref(),
                input.locator.as_deref(),
                &tags,
            )
            .await?;

        Ok(spindle_core::models::ResearchAddNoteOutput { note_id: note.id })
    }

    pub async fn research_add_claim(
        &self,
        input: spindle_core::models::ResearchAddClaimInput,
    ) -> Result<spindle_core::models::ResearchAddClaimOutput> {
        let note = if let Some(note_id) = &input.note_id {
            let note = self
                .repository
                .get_research_note(note_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("note not found: {}", note_id))?;
            if note.project_id != input.project_id {
                anyhow::bail!("note does not belong to the requested project");
            }
            Some(note)
        } else {
            None
        };

        if let (Some(note), Some(source_id)) = (&note, &input.source_id)
            && let Some(note_source_id) = note.source_id.as_deref()
            && note_source_id != source_id
        {
            anyhow::bail!("note is linked to a different source");
        }

        let source_id = input
            .source_id
            .clone()
            .or_else(|| note.as_ref().and_then(|note| note.source_id.clone()));

        let source = if let Some(source_id) = &source_id {
            let source = self
                .repository
                .get_research_source(source_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("source not found: {}", source_id))?;
            if source.project_id != input.project_id {
                anyhow::bail!("source does not belong to the requested project");
            }
            Some(source)
        } else {
            None
        };

        let tags = normalize_tags(&input.tags);

        let mut branch_id = self
            .resolve_research_branch_id(&input.project_id, input.branch_id.as_deref())
            .await?;
        if input.branch_id.is_none() {
            if let Some(note_branch_id) = note.as_ref().and_then(|note| note.branch_id.clone()) {
                branch_id = Some(note_branch_id);
            } else if let Some(source_branch_id) =
                source.as_ref().and_then(|source| source.branch_id.clone())
            {
                branch_id = Some(source_branch_id);
            }
        }
        if let Some(note_branch_id) = note.as_ref().and_then(|note| note.branch_id.as_deref())
            && branch_id.as_deref() != Some(note_branch_id)
        {
            anyhow::bail!("note branch does not match the requested branch");
        }
        if let Some(source_branch_id) = source
            .as_ref()
            .and_then(|source| source.branch_id.as_deref())
            && branch_id.as_deref() != Some(source_branch_id)
        {
            anyhow::bail!("source branch does not match the requested branch");
        }
        let branch_id_str = branch_id.as_deref();

        let claim = self
            .repository
            .create_research_claim(
                &input.project_id,
                source_id.as_deref(),
                input.note_id.as_deref(),
                branch_id_str,
                &input.claim,
                input.topic.as_deref(),
                input.time_period.as_deref(),
                input.location.as_deref(),
                &input.confidence,
                &tags,
            )
            .await?;

        Ok(spindle_core::models::ResearchAddClaimOutput { claim_id: claim.id })
    }

    pub async fn research_search(
        &self,
        input: spindle_core::models::ResearchSearchInput,
    ) -> Result<spindle_core::models::ResearchSearchOutput> {
        let branch_id = self
            .resolve_research_branch_id(&input.project_id, input.branch_id.as_deref())
            .await?;
        let tags = normalize_tags(&input.tags);
        let results = self
            .repository
            .search_research(
                &input.project_id,
                branch_id.as_deref(),
                &input.query,
                &tags,
                input.time_period.as_deref(),
                input.location.as_deref(),
                input.limit,
            )
            .await?;

        Ok(spindle_core::models::ResearchSearchOutput { results })
    }

    pub async fn research_pack_for_scene(
        &self,
        input: spindle_core::models::ResearchPackForSceneInput,
    ) -> Result<spindle_core::models::ResearchPackForSceneOutput> {
        let limit = input.limit.unwrap_or(10);
        let project_id = &input.project_id;
        let branch_id = self
            .resolve_research_branch_id(project_id, input.branch_id.as_deref())
            .await?;

        let mut target_tags = normalize_tags(&input.tags);
        if let Some(summary) = &input.scene_summary {
            for word in summary.split_whitespace() {
                let cleaned: String = word.chars().filter(|c| c.is_alphanumeric()).collect();
                if cleaned.len() > 3 {
                    target_tags.push(cleaned.to_lowercase());
                }
            }
        }

        let is_explicit_requested = target_tags.iter().any(|t| {
            let tl = t.to_lowercase();
            tl == "explicit" || tl == "adult"
        }) || input
            .explicit_query
            .as_ref()
            .map(|q| {
                let ql = q.to_lowercase();
                ql.contains("explicit") || ql.contains("adult")
            })
            .unwrap_or(false);

        let all_sources = self
            .repository
            .list_research_sources_by_project(project_id)
            .await?
            .into_iter()
            .filter(|source| {
                research_branch_matches(source.branch_id.as_deref(), branch_id.as_deref())
            })
            .filter(|source| {
                if !is_explicit_requested {
                    !source.tags.iter().any(|t| {
                        let tl = t.to_lowercase();
                        tl == "explicit" || tl == "adult"
                    })
                } else {
                    true
                }
            })
            .collect::<Vec<_>>();
        let all_notes = self
            .repository
            .list_research_notes_by_project(project_id)
            .await?
            .into_iter()
            .filter(|note| research_branch_matches(note.branch_id.as_deref(), branch_id.as_deref()))
            .filter(|note| {
                if !is_explicit_requested {
                    !note.tags.iter().any(|t| {
                        let tl = t.to_lowercase();
                        tl == "explicit" || tl == "adult"
                    })
                } else {
                    true
                }
            })
            .collect::<Vec<_>>();
        let all_claims = self
            .repository
            .list_research_claims_by_project(project_id)
            .await?
            .into_iter()
            .filter(|claim| {
                research_branch_matches(claim.branch_id.as_deref(), branch_id.as_deref())
            })
            .filter(|claim| {
                if !is_explicit_requested {
                    !claim.tags.iter().any(|t| {
                        let tl = t.to_lowercase();
                        tl == "explicit" || tl == "adult"
                    })
                } else {
                    true
                }
            })
            .collect::<Vec<_>>();

        let mut scored_sources = Vec::new();
        for s in all_sources {
            let mut score = 0;
            if let Some(loc) = &input.scene_location {
                let loc_lower = loc.to_lowercase();
                if s.title.to_lowercase().contains(&loc_lower) {
                    score += 5;
                }
                if let Some(sum) = &s.summary
                    && sum.to_lowercase().contains(&loc_lower)
                {
                    score += 3;
                }
            }
            if let Some(q) = &input.explicit_query {
                let q_lower = q.to_lowercase();
                if s.title.to_lowercase().contains(&q_lower) {
                    score += 10;
                }
                if let Some(sum) = &s.summary
                    && sum.to_lowercase().contains(&q_lower)
                {
                    score += 5;
                }
            }
            for t in &target_tags {
                if spindle_core::research_match::research_tag_matches(t, &s.tags) {
                    score += 2;
                }
            }
            if score > 0
                || (input.explicit_query.is_none()
                    && input.scene_location.is_none()
                    && input.tags.is_empty())
            {
                scored_sources.push((s, score));
            }
        }
        scored_sources.sort_by_key(|(_, score)| std::cmp::Reverse(*score));

        let mut scored_notes = Vec::new();
        for n in all_notes {
            let mut score = 0;
            if let Some(loc) = &input.scene_location {
                let loc_lower = loc.to_lowercase();
                if n.note.to_lowercase().contains(&loc_lower) {
                    score += 5;
                }
                if let Some(q) = &n.quote
                    && q.to_lowercase().contains(&loc_lower)
                {
                    score += 3;
                }
            }
            if let Some(q) = &input.explicit_query {
                let q_lower = q.to_lowercase();
                if n.note.to_lowercase().contains(&q_lower) {
                    score += 10;
                }
                if let Some(qt) = &n.quote
                    && qt.to_lowercase().contains(&q_lower)
                {
                    score += 5;
                }
            }
            for t in &target_tags {
                if spindle_core::research_match::research_tag_matches(t, &n.tags) {
                    score += 2;
                }
            }
            if score > 0
                || (input.explicit_query.is_none()
                    && input.scene_location.is_none()
                    && input.tags.is_empty())
            {
                scored_notes.push((n, score));
            }
        }
        scored_notes.sort_by_key(|(_, score)| std::cmp::Reverse(*score));

        let mut scored_claims = Vec::new();
        for c in all_claims {
            let mut score = 0;
            if let Some(loc) = &input.scene_location {
                let loc_lower = loc.to_lowercase();
                if let Some(cloc) = &c.location
                    && cloc.to_lowercase().contains(&loc_lower)
                {
                    score += 8;
                }
                if c.claim.to_lowercase().contains(&loc_lower) {
                    score += 5;
                }
            }
            if let Some(q) = &input.explicit_query {
                let q_lower = q.to_lowercase();
                if c.claim.to_lowercase().contains(&q_lower) {
                    score += 10;
                }
                if let Some(top) = &c.topic
                    && top.to_lowercase().contains(&q_lower)
                {
                    score += 5;
                }
            }
            for t in &target_tags {
                if spindle_core::research_match::research_tag_matches(t, &c.tags) {
                    score += 2;
                }
            }
            if score > 0
                || (input.explicit_query.is_none()
                    && input.scene_location.is_none()
                    && input.tags.is_empty())
            {
                scored_claims.push((c, score));
            }
        }
        scored_claims.sort_by_key(|(_, score)| std::cmp::Reverse(*score));

        let sources: Vec<spindle_core::models::ResearchSource> = scored_sources
            .into_iter()
            .take(limit)
            .map(|(s, _)| map_research_source(s))
            .collect();

        let notes: Vec<spindle_core::models::ResearchNote> = scored_notes
            .into_iter()
            .take(limit)
            .map(|(n, _)| map_research_note(n))
            .collect();

        let claims: Vec<spindle_core::models::ResearchClaim> = scored_claims
            .into_iter()
            .take(limit)
            .map(|(c, _)| map_research_claim(c))
            .collect();

        Ok(spindle_core::models::ResearchPackForSceneOutput {
            sources,
            notes,
            claims,
        })
    }

    pub async fn list_research_usages_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<spindle_core::models::ResearchUsage>> {
        let rows = self
            .repository
            .list_research_usages_by_project(project_id)
            .await?;
        Ok(rows.into_iter().map(map_research_usage_record).collect())
    }

    pub async fn list_research_usages_for_scene(
        &self,
        project_id: &str,
        scene_id: &str,
    ) -> Result<Vec<spindle_core::models::ResearchUsage>> {
        self.repository.get_project(project_id).await?;
        let rows = self
            .repository
            .list_research_usages_for_scene(project_id, scene_id)
            .await?;
        Ok(rows.into_iter().map(map_research_usage_record).collect())
    }

    pub async fn research_usage_for_scene(
        &self,
        input: spindle_core::models::ResearchUsageForSceneInput,
    ) -> Result<spindle_core::models::ResearchUsageForSceneOutput> {
        let usages = self
            .list_research_usages_for_scene(&input.project_id, &input.scene_id)
            .await?;
        Ok(spindle_core::models::ResearchUsageForSceneOutput { usages })
    }
}
