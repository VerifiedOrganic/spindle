use super::*;
use crate::drafting::{
    DraftTokenizer, PacketBudget, PreparedDraftingPacket, assemble_packet_with_system,
    critique::{
        CriticBudget, assemble_critic_messages, assemble_critic_repair_messages,
        ingest_and_dedup_findings, parse_and_validate_critique_output,
    },
};
use spindle_core::drafting::{
    CriticFindingInput, DraftingPacketV1, EditorialPolicy, IngestedFinding, ShadowCritiqueOutcome,
};

#[derive(Debug, Clone)]
pub struct ShadowCritiqueRunResult {
    pub ingested_findings: Vec<IngestedFinding>,
    pub deduplicated_finding_ids: Vec<String>,
    pub prompt_hashes: Vec<String>,
    pub model_call_ids: Vec<String>,
    pub outcome: ShadowCritiqueOutcome,
}

impl ModelRouter {
    /// Opt-in packet generation; existing complete/legacy package paths are unchanged.
    /// Uses a caller-supplied verified counter or an explicitly calibrated profile.
    pub async fn draft_packet(
        &self,
        policy: EditorialPolicy,
        packet: DraftingPacketV1,
        budget: &PacketBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
    ) -> anyhow::Result<(PreparedDraftingPacket, ModelResponse)> {
        self.draft_packet_before_dispatch(policy, packet, budget, tokenizer, |_| Ok(()))
            .await
    }

    pub(crate) async fn draft_packet_before_dispatch(
        &self,
        policy: EditorialPolicy,
        packet: DraftingPacketV1,
        budget: &PacketBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        before_dispatch: impl FnOnce(&PreparedDraftingPacket) -> anyhow::Result<()> + Send,
    ) -> anyhow::Result<(PreparedDraftingPacket, ModelResponse)> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "packet drafting requires critique_v1 policy; use legacy completion otherwise"
        );
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let rating = packet.scene_card.rating.as_str();
        let mut route = resolve_cleared_route(&runtime, "draft", Some(rating))?.clone();
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: packet framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        anyhow::ensure!(
            agent.config.model == budget.model_id,
            "budget_unknown: resolved model does not match budget"
        );
        let reserve: u32 = budget.output_reserve.try_into()?;
        anyhow::ensure!(reserve > 0, "output reserve must be positive");
        anyhow::ensure!(
            route.max_tokens.is_none_or(|max| max <= reserve),
            "configured output exceeds packet reservation"
        );
        if route.max_tokens.is_none() {
            route.max_tokens = Some(reserve);
        }
        if budget.generation_seed.is_some() {
            route.seed = budget.generation_seed;
        }
        let scope = if agent.config.tokenizer.is_some() || tokenizer.is_none() {
            Some(self.counter_scope(&runtime, &route)?)
        } else {
            None
        };
        if let Some(scope) = &scope {
            anyhow::ensure!(
                budget.context_window <= scope.max_context,
                "packet window exceeds configured model context"
            );
        }
        let configured = scope
            .as_ref()
            .map(|scope| self.configured_tokenizer(scope))
            .transpose()?;
        let tokenizer = configured
            .as_deref()
            .map(|encoder| encoder as &dyn DraftTokenizer)
            .or(tokenizer);
        let system = system_prompt_for_request(&route, Some(rating));
        let prepared = assemble_packet_with_system(packet.clone(), budget, tokenizer, &system)?;
        route.system_prompt = prepared.messages.system.clone();
        let request = ModelRequest {
            route: "draft".into(),
            prompt: prepared.messages.user.clone(),
            rating: Some(rating.into()),
            context: Some(RequestContext {
                project_id: Some(packet.project_id),
                scene_id: packet.scene_id,
                run_id: packet.run_id,
                prompt_template_id: Some("draft.packet".into()),
                prompt_version: Some("v1".into()),
                context_packet_hash: Some(prepared.messages_hash.clone()),
                ..Default::default()
            }),
        };
        before_dispatch(&prepared)?;
        let verified_provider_count = if let (Some(scope), Some(encoder)) = (&scope, &configured)
            && encoder.config().verify_each_prompt
        {
            let receipt = self
                .probe_count(
                    scope,
                    &prepared.messages,
                    Some(rating),
                    request.context.as_ref(),
                )
                .await?;
            encoder.verify_count(&prepared.messages, receipt.input_tokens)?;
            Some(receipt.input_tokens)
        } else {
            None
        };
        #[cfg(any(test, feature = "test-support"))]
        self.record_dispatch(DispatchRecord::Dispatch {
            route: request.route.clone(),
            agent: dispatch_agent_label(&runtime, &route),
            rating: request.rating.clone(),
            prompt: request.prompt.clone(),
        })?;
        let start = std::time::Instant::now();
        let mut response = if let Some(scope) = &scope {
            self.run_ollama_packet(scope, &route, &prepared.messages)
                .await
        } else {
            self.run_http_inner(
                &runtime,
                &route,
                request.rating.as_deref(),
                &request.prompt,
                false,
            )
            .await
        };
        self.record_usage(
            &request.route,
            request.rating.as_deref(),
            request.context.as_ref(),
            &request.prompt,
            false,
            Some(&route),
            start,
            &mut response,
        )
        .await;
        let response = response?;
        if let Some(encoder) = &configured {
            let count = response
                .usage
                .as_ref()
                .and_then(|u| u.input_tokens)
                .context("native packet missing prompt_eval_count; recovery required")?;
            encoder.verify_count(&prepared.messages, count.try_into()?)?;
            if let Some(verified) = verified_provider_count {
                anyhow::ensure!(
                    count == verified as u64,
                    "native packet prompt count differs from verified probe; recovery required"
                );
            }
        }
        Ok((prepared, response))
    }

    /// Resolve the effective `PacketBudget` and transport/sampling metadata for
    /// an evaluation route and rating so both packet and legacy baseline arms
    /// use identical model, context window, output reserve, and sampling policy.
    pub fn eval_draft_budget_and_settings(
        &self,
        route_name: &str,
        rating: &str,
        generation_seed: Option<i64>,
    ) -> anyhow::Result<(PacketBudget, serde_json::Value)> {
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let route = resolve_cleared_route(&runtime, route_name, Some(rating))?.clone();
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: eval framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        let context_window = agent.config.max_context.unwrap_or(32_768);
        let mut budget = PacketBudget::scene_default(agent.config.model.clone(), context_window);
        if let Some(max_tokens) = route.max_tokens {
            budget.output_reserve = budget.output_reserve.max(max_tokens as usize);
        }
        budget.generation_seed = generation_seed.or(route.seed);
        let effective_max_tokens = route
            .max_tokens
            .unwrap_or_else(|| u32::try_from(budget.output_reserve).unwrap_or(3_000));
        let settings = if agent.config.tokenizer.is_some() {
            let scope = self.counter_scope(&runtime, &route)?;
            let encoder = self.configured_tokenizer(&scope)?;
            serde_json::json!({
                "transport": "ollama_native_chat",
                "agent_id": agent.config.id,
                "model": agent.config.model,
                "model_revision": encoder.config().model_revision,
                "tokenizer_id": encoder.id(),
                "tokenizer_version": encoder.version(),
                "context_window": scope.max_context,
                "max_tokens": effective_max_tokens,
                "output_reserve": budget.output_reserve,
                "input_limit": budget.input_limit,
                "framing_reserve": budget.framing_reserve,
                "temperature": route.temperature,
                "top_p": route.top_p,
                "seed": budget.generation_seed,
                "truncate": false,
                "shift": false,
            })
        } else {
            serde_json::json!({
                "transport": "openai_compatible_http",
                "agent_id": agent.config.id,
                "model": agent.config.model,
                "model_revision": serde_json::Value::Null,
                "tokenizer_id": serde_json::Value::Null,
                "tokenizer_version": serde_json::Value::Null,
                "context_window": context_window,
                "max_tokens": effective_max_tokens,
                "output_reserve": budget.output_reserve,
                "input_limit": budget.input_limit,
                "framing_reserve": budget.framing_reserve,
                "temperature": route.temperature,
                "top_p": route.top_p,
                "seed": budget.generation_seed,
                "truncate": false,
                "shift": false,
            })
        };
        Ok((budget, settings))
    }

    /// Derive the effective `CriticBudget` and provider settings metadata for
    /// a `review` route and rating so shadow critique uses the configured
    /// review model, context window, output reserve, and sampling policy.
    pub fn eval_critic_budget_and_settings(
        &self,
        route_name: &str,
        rating: &str,
        generation_seed: Option<i64>,
    ) -> anyhow::Result<(CriticBudget, serde_json::Value)> {
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let route = resolve_cleared_route(&runtime, route_name, Some(rating))?.clone();
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: critic framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        let context_window = agent.config.max_context.unwrap_or(32_768);
        let mut budget = CriticBudget::scene_default(agent.config.model.clone(), context_window);
        if let Some(max_tokens) = route.max_tokens {
            budget.output_reserve = budget.output_reserve.max(max_tokens as usize);
        }
        budget.generation_seed = generation_seed.or(route.seed);
        let effective_max_tokens = route
            .max_tokens
            .unwrap_or_else(|| u32::try_from(budget.output_reserve).unwrap_or(1_500));
        let settings = if agent.config.tokenizer.is_some() {
            let scope = self.counter_scope(&runtime, &route)?;
            let encoder = self.configured_tokenizer(&scope)?;
            serde_json::json!({
                "transport": "ollama_native_chat",
                "agent_id": agent.config.id,
                "model": agent.config.model,
                "model_revision": encoder.config().model_revision,
                "tokenizer_id": encoder.id(),
                "tokenizer_version": encoder.version(),
                "context_window": scope.max_context,
                "max_tokens": effective_max_tokens,
                "output_reserve": budget.output_reserve,
                "input_limit": budget.input_limit,
                "framing_reserve": budget.framing_reserve,
                "temperature": route.temperature,
                "top_p": route.top_p,
                "seed": budget.generation_seed,
                "truncate": false,
                "shift": false,
            })
        } else {
            serde_json::json!({
                "transport": "openai_compatible_http",
                "agent_id": agent.config.id,
                "model": agent.config.model,
                "model_revision": serde_json::Value::Null,
                "tokenizer_id": serde_json::Value::Null,
                "tokenizer_version": serde_json::Value::Null,
                "context_window": context_window,
                "max_tokens": effective_max_tokens,
                "output_reserve": budget.output_reserve,
                "input_limit": budget.input_limit,
                "framing_reserve": budget.framing_reserve,
                "temperature": route.temperature,
                "top_p": route.top_p,
                "seed": budget.generation_seed,
                "truncate": false,
                "shift": false,
            })
        };
        Ok((budget, settings))
    }

    /// Assemble and verify a `DraftingPacketV1` offline against the configured
    /// calibrated tokenizer and route budget without making any network calls.
    pub fn verify_packet_offline(
        &self,
        route_name: &str,
        rating: &str,
        packet: DraftingPacketV1,
        generation_seed: Option<i64>,
    ) -> anyhow::Result<PreparedDraftingPacket> {
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let route = resolve_cleared_route(&runtime, route_name, Some(rating))?.clone();
        let scope = self.counter_scope(&runtime, &route)?;
        let encoder = self.configured_tokenizer(&scope)?;
        let (budget, _) =
            self.eval_draft_budget_and_settings(route_name, rating, generation_seed)?;
        let system = system_prompt_for_request(&route, Some(rating));
        Ok(assemble_packet_with_system(
            packet,
            &budget,
            Some(encoder.as_ref() as &dyn DraftTokenizer),
            &system,
        )?)
    }

    /// Dispatch pre-rendered evaluation messages (such as the frozen legacy
    /// baseline in G) under the exact same route clearance, calibrated
    /// tokenizer, context window, sampling options, and prompt-count
    /// verification as `draft_packet`.
    pub async fn draft_eval_messages(
        &self,
        route_name: &str,
        rating: &str,
        messages: &crate::drafting::DraftMessages,
        budget: &PacketBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
    ) -> anyhow::Result<(String, usize, ModelResponse)> {
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let mut route = resolve_cleared_route(&runtime, route_name, Some(rating))?.clone();
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: eval framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        anyhow::ensure!(
            agent.config.model == budget.model_id,
            "budget_unknown: resolved model does not match budget"
        );
        let reserve: u32 = budget.output_reserve.try_into()?;
        anyhow::ensure!(reserve > 0, "output reserve must be positive");
        anyhow::ensure!(
            route.max_tokens.is_none_or(|max| max <= reserve),
            "configured output exceeds packet reservation"
        );
        if route.max_tokens.is_none() {
            route.max_tokens = Some(reserve);
        }
        if budget.generation_seed.is_some() {
            route.seed = budget.generation_seed;
        }
        let scope = if agent.config.tokenizer.is_some() || tokenizer.is_none() {
            Some(self.counter_scope(&runtime, &route)?)
        } else {
            None
        };
        if let Some(scope) = &scope {
            anyhow::ensure!(
                budget.context_window <= scope.max_context,
                "packet window exceeds configured model context"
            );
        }
        let configured = scope
            .as_ref()
            .map(|scope| self.configured_tokenizer(scope))
            .transpose()?;
        let t = configured
            .as_deref()
            .map(|encoder| encoder as &dyn DraftTokenizer)
            .or(tokenizer)
            .ok_or_else(|| anyhow::anyhow!("budget_unknown: no tokenizer configured"))?;
        anyhow::ensure!(
            !t.id().is_empty() && !t.version().is_empty() && t.supports_model(&budget.model_id),
            "budget_unknown: tokenizer/model mismatch"
        );
        let input_tokens = t
            .count_messages(messages)
            .map_err(|e| anyhow::anyhow!("budget_unknown: {e}"))?;
        anyhow::ensure!(
            input_tokens <= budget.input_limit
                && input_tokens
                    .checked_add(budget.output_reserve)
                    .and_then(|n| n.checked_add(budget.framing_reserve))
                    .is_some_and(|n| n <= budget.context_window),
            "mandatory_overflow: final rendered messages/context window"
        );
        let prompt_hash = crate::tokenization::messages_hash(messages)?;
        route.system_prompt = messages.system.clone();
        let request = ModelRequest {
            route: route_name.into(),
            prompt: messages.user.clone(),
            rating: Some(rating.into()),
            context: Some(RequestContext {
                prompt_template_id: Some("draft.legacy_eval".into()),
                prompt_version: Some("v1".into()),
                context_packet_hash: Some(prompt_hash.clone()),
                ..Default::default()
            }),
        };
        let verified_provider_count = if let (Some(scope), Some(encoder)) = (&scope, &configured)
            && encoder.config().verify_each_prompt
        {
            let receipt = self
                .probe_count(scope, messages, Some(rating), request.context.as_ref())
                .await?;
            encoder.verify_count(messages, receipt.input_tokens)?;
            Some(receipt.input_tokens)
        } else {
            None
        };
        #[cfg(any(test, feature = "test-support"))]
        self.record_dispatch(DispatchRecord::Dispatch {
            route: request.route.clone(),
            agent: dispatch_agent_label(&runtime, &route),
            rating: request.rating.clone(),
            prompt: request.prompt.clone(),
        })?;
        let start = std::time::Instant::now();
        let mut response = if let Some(scope) = &scope {
            self.run_ollama_packet(scope, &route, messages).await
        } else {
            self.run_http_inner(
                &runtime,
                &route,
                request.rating.as_deref(),
                &request.prompt,
                false,
            )
            .await
        };
        self.record_usage(
            &request.route,
            request.rating.as_deref(),
            request.context.as_ref(),
            &request.prompt,
            false,
            Some(&route),
            start,
            &mut response,
        )
        .await;
        let response = response?;
        if let Some(encoder) = &configured {
            let count = response
                .usage
                .as_ref()
                .and_then(|u| u.input_tokens)
                .context("native packet missing prompt_eval_count; recovery required")?;
            encoder.verify_count(messages, count.try_into()?)?;
            if let Some(verified) = verified_provider_count {
                anyhow::ensure!(
                    count == verified as u64,
                    "native packet prompt count differs from verified probe; recovery required"
                );
            }
        }
        Ok((prompt_hash, input_tokens, response))
    }

    /// Execute a shadow-mode scene critic pass over an immutable base draft,
    /// ingesting and deduplicating findings, enforcing rating clearance on the
    /// `review` route, validating UTF-8 byte-span/quote anchors, quarantining
    /// author-lock conflicts, and allowing at most one bounded schema/anchor
    /// repair call within `max_calls` (`1..=2`). Never mutates the base draft.
    #[allow(clippy::too_many_arguments)]
    pub async fn critique_draft_shadow(
        &self,
        policy: EditorialPolicy,
        packet: &DraftingPacketV1,
        expected_packet_hash: &str,
        base_text: &str,
        expected_base_text_hash: &str,
        findings: Vec<CriticFindingInput>,
        budget: &CriticBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        max_calls: u32,
    ) -> anyhow::Result<ShadowCritiqueRunResult> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "shadow critique requires critique_v1 policy"
        );
        anyhow::ensure!(
            (1..=2).contains(&max_calls),
            "shadow critique max_calls must be 1 or 2"
        );
        let actual_base_hash = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(base_text.as_bytes()))
        };
        anyhow::ensure!(
            actual_base_hash == expected_base_text_hash,
            "stale_anchor_hash: base_text SHA-256 ({actual_base_hash}) does not match expected_base_text_hash ({expected_base_text_hash})"
        );

        let (ingested_findings, deduplicated_finding_ids) = ingest_and_dedup_findings(
            findings,
            base_text,
            expected_base_text_hash,
            packet.scene_id.as_deref(),
        )?;

        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let rating = packet.scene_card.rating.as_str();
        let mut route = resolve_cleared_route(&runtime, "review", Some(rating))?.clone();
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: critic framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        anyhow::ensure!(
            agent.config.model == budget.model_id,
            "budget_unknown: resolved review model does not match critic budget"
        );
        let reserve: u32 = budget.output_reserve.try_into()?;
        anyhow::ensure!(reserve > 0, "critic output reserve must be positive");
        anyhow::ensure!(
            route.max_tokens.is_none_or(|max| max <= reserve),
            "configured review output exceeds critic reservation"
        );
        if route.max_tokens.is_none() {
            route.max_tokens = Some(reserve);
        }
        if budget.generation_seed.is_some() {
            route.seed = budget.generation_seed;
        }
        let scope = if agent.config.tokenizer.is_some() || tokenizer.is_none() {
            Some(self.counter_scope(&runtime, &route)?)
        } else {
            None
        };
        if let Some(scope) = &scope {
            anyhow::ensure!(
                budget.context_window <= scope.max_context,
                "critic window exceeds configured review model context"
            );
        }
        let configured = scope
            .as_ref()
            .map(|scope| self.configured_tokenizer(scope))
            .transpose()?;
        let t = configured
            .as_deref()
            .map(|encoder| encoder as &dyn DraftTokenizer)
            .or(tokenizer);

        let route_system = system_prompt_for_request(&route, Some(rating));
        let (initial_messages, initial_hash, _) = assemble_critic_messages(
            packet,
            expected_packet_hash,
            base_text,
            expected_base_text_hash,
            &ingested_findings,
            budget,
            t,
            &route_system,
        )?;

        let mut prompt_hashes = vec![initial_hash.clone()];
        let mut model_call_ids = Vec::new();

        // Dispatch Call 1 (initial critic evaluation)
        route.system_prompt = initial_messages.system.clone();
        let req1 = ModelRequest {
            route: "review".into(),
            prompt: initial_messages.user.clone(),
            rating: Some(rating.into()),
            context: Some(RequestContext {
                project_id: Some(packet.project_id.clone()),
                scene_id: packet.scene_id.clone(),
                run_id: packet.run_id.clone(),
                prompt_template_id: Some("review.critique_v1".into()),
                prompt_version: Some("v1".into()),
                context_packet_hash: Some(initial_hash.clone()),
                ..Default::default()
            }),
        };
        let verified_count_1 = if let (Some(scope), Some(encoder)) = (&scope, &configured)
            && encoder.config().verify_each_prompt
        {
            let receipt = self
                .probe_count(
                    scope,
                    &initial_messages,
                    Some(rating),
                    req1.context.as_ref(),
                )
                .await?;
            encoder.verify_count(&initial_messages, receipt.input_tokens)?;
            Some(receipt.input_tokens)
        } else {
            None
        };
        #[cfg(any(test, feature = "test-support"))]
        self.record_dispatch(DispatchRecord::Dispatch {
            route: req1.route.clone(),
            agent: dispatch_agent_label(&runtime, &route),
            rating: req1.rating.clone(),
            prompt: req1.prompt.clone(),
        })?;
        let start1 = std::time::Instant::now();
        let mut resp1 = if let Some(scope) = &scope {
            self.run_ollama_packet(scope, &route, &initial_messages)
                .await
        } else {
            self.run_http_inner(
                &runtime,
                &route,
                req1.rating.as_deref(),
                &req1.prompt,
                false,
            )
            .await
        };
        self.record_usage(
            &req1.route,
            req1.rating.as_deref(),
            req1.context.as_ref(),
            &req1.prompt,
            false,
            Some(&route),
            start1,
            &mut resp1,
        )
        .await;
        let resp1 = resp1?;
        if let Some(encoder) = &configured {
            let count = resp1
                .usage
                .as_ref()
                .and_then(|u| u.input_tokens)
                .context("native critic missing prompt_eval_count; recovery required")?;
            encoder.verify_count(&initial_messages, count.try_into()?)?;
            if let Some(verified) = verified_count_1 {
                anyhow::ensure!(
                    count == verified as u64,
                    "native critic prompt count differs from verified probe; recovery required"
                );
            }
        }
        if let Some(cid) = resp1.call_id.clone() {
            model_call_ids.push(cid);
        }

        match parse_and_validate_critique_output(
            &resp1.output,
            packet,
            expected_packet_hash,
            base_text,
            expected_base_text_hash,
        ) {
            Ok(valid_critique) => Ok(ShadowCritiqueRunResult {
                ingested_findings,
                deduplicated_finding_ids,
                prompt_hashes,
                model_call_ids,
                outcome: ShadowCritiqueOutcome::Critiqued {
                    critique: Box::new(valid_critique),
                    repair_used: false,
                },
            }),
            Err(first_err) => {
                if max_calls < 2 {
                    return Ok(ShadowCritiqueRunResult {
                        ingested_findings,
                        deduplicated_finding_ids,
                        prompt_hashes,
                        model_call_ids,
                        outcome: ShadowCritiqueOutcome::CriticInvalid {
                            reason: first_err.to_string(),
                            attempts: 1,
                            repair_attempted: false,
                        },
                    });
                }

                // Bounded 1-shot schema/anchor repair inside the call cap
                let (repair_messages, repair_hash, _) = match assemble_critic_repair_messages(
                    &initial_messages,
                    &resp1.output,
                    &first_err,
                    budget,
                    t,
                ) {
                    Ok(v) => v,
                    Err(prep_err) => {
                        return Ok(ShadowCritiqueRunResult {
                            ingested_findings,
                            deduplicated_finding_ids,
                            prompt_hashes,
                            model_call_ids,
                            outcome: ShadowCritiqueOutcome::CriticInvalid {
                                reason: format!("{first_err}; repair assembly failed: {prep_err}"),
                                attempts: 1,
                                repair_attempted: false,
                            },
                        });
                    }
                };
                prompt_hashes.push(repair_hash.clone());
                route.system_prompt = repair_messages.system.clone();
                let req2 = ModelRequest {
                    route: "review".into(),
                    prompt: repair_messages.user.clone(),
                    rating: Some(rating.into()),
                    context: Some(RequestContext {
                        project_id: Some(packet.project_id.clone()),
                        scene_id: packet.scene_id.clone(),
                        run_id: packet.run_id.clone(),
                        prompt_template_id: Some("review.critique_v1.repair".into()),
                        prompt_version: Some("v1".into()),
                        context_packet_hash: Some(repair_hash),
                        ..Default::default()
                    }),
                };
                let verified_count_2 = if let (Some(scope), Some(encoder)) = (&scope, &configured)
                    && encoder.config().verify_each_prompt
                {
                    let receipt = self
                        .probe_count(scope, &repair_messages, Some(rating), req2.context.as_ref())
                        .await?;
                    encoder.verify_count(&repair_messages, receipt.input_tokens)?;
                    Some(receipt.input_tokens)
                } else {
                    None
                };
                #[cfg(any(test, feature = "test-support"))]
                self.record_dispatch(DispatchRecord::Dispatch {
                    route: req2.route.clone(),
                    agent: dispatch_agent_label(&runtime, &route),
                    rating: req2.rating.clone(),
                    prompt: req2.prompt.clone(),
                })?;
                let start2 = std::time::Instant::now();
                let mut resp2 = if let Some(scope) = &scope {
                    self.run_ollama_packet(scope, &route, &repair_messages)
                        .await
                } else {
                    self.run_http_inner(
                        &runtime,
                        &route,
                        req2.rating.as_deref(),
                        &req2.prompt,
                        false,
                    )
                    .await
                };
                self.record_usage(
                    &req2.route,
                    req2.rating.as_deref(),
                    req2.context.as_ref(),
                    &req2.prompt,
                    false,
                    Some(&route),
                    start2,
                    &mut resp2,
                )
                .await;
                let resp2 = resp2?;
                if let Some(encoder) = &configured {
                    let count = resp2.usage.as_ref().and_then(|u| u.input_tokens).context(
                        "native critic repair missing prompt_eval_count; recovery required",
                    )?;
                    encoder.verify_count(&repair_messages, count.try_into()?)?;
                    if let Some(verified) = verified_count_2 {
                        anyhow::ensure!(
                            count == verified as u64,
                            "native critic repair prompt count differs from verified probe; recovery required"
                        );
                    }
                }
                if let Some(cid) = resp2.call_id.clone() {
                    model_call_ids.push(cid);
                }

                match parse_and_validate_critique_output(
                    &resp2.output,
                    packet,
                    expected_packet_hash,
                    base_text,
                    expected_base_text_hash,
                ) {
                    Ok(repaired_critique) => Ok(ShadowCritiqueRunResult {
                        ingested_findings,
                        deduplicated_finding_ids,
                        prompt_hashes,
                        model_call_ids,
                        outcome: ShadowCritiqueOutcome::Critiqued {
                            critique: Box::new(repaired_critique),
                            repair_used: true,
                        },
                    }),
                    Err(repair_err) => Ok(ShadowCritiqueRunResult {
                        ingested_findings,
                        deduplicated_finding_ids,
                        prompt_hashes,
                        model_call_ids,
                        outcome: ShadowCritiqueOutcome::CriticInvalid {
                            reason: repair_err.to_string(),
                            attempts: 2,
                            repair_attempted: true,
                        },
                    }),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{captured_json_body, spawn_http_capture_server};
    use super::*;
    #[tokio::test]
    async fn packet_dispatch_counts_exact_effective_messages_and_preserves_output() {
        use crate::drafting::tests::{VerifiedFixtureTokenizer, budget, fixture};
        use spindle_core::drafting::EditorialPolicy;
        let response_body = serde_json::json!({"choices":[{"message":{"content":" \r\n雪。\r\n "},"finish_reason":"length"}]}).to_string();
        let (endpoint, captured, server) =
            spawn_http_capture_server(vec![(200, response_body.clone()), (200, response_body)]);
        let router = ModelRouter::local_only();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("spindle.toml");
        std::fs::write(
            &path,
            format!(
                r#"
[health_check]
enabled = false
[[agents]]
id = "venice"
name = "Venice"
provider = "venice"
endpoint = "{endpoint}"
model = "venice-model"
ratings = ["general"]
[[routing]]
route = "draft"
agent = "venice"
system_prompt = "Configured system framing."
max_tokens = 123
"#
            ),
        )
        .unwrap();
        router.configure(Some(path.to_str().unwrap())).unwrap();
        let mut b = budget();
        b.model_id = "venice-model".into();
        assert!(
            router
                .draft_packet(
                    EditorialPolicy::Legacy,
                    fixture(),
                    &b,
                    Some(&VerifiedFixtureTokenizer)
                )
                .await
                .is_err()
        );
        assert!(
            router
                .draft_packet(EditorialPolicy::CritiqueV1, fixture(), &b, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("budget_unknown")
        );
        let mut denied = fixture();
        denied.scene_card.rating = spindle_core::models::ContentRating::Explicit;
        assert!(
            router
                .draft_packet(
                    EditorialPolicy::CritiqueV1,
                    denied,
                    &b,
                    Some(&VerifiedFixtureTokenizer)
                )
                .await
                .unwrap_err()
                .downcast_ref::<RouteClearanceError>()
                .is_some()
        );
        let (prepared, response) = router
            .draft_packet(
                EditorialPolicy::CritiqueV1,
                fixture(),
                &b,
                Some(&VerifiedFixtureTokenizer),
            )
            .await
            .unwrap();
        assert_eq!(response.output.as_bytes(), " \r\n雪。\r\n ".as_bytes());
        assert!(response.truncated);
        let pool = crate::sqlite::SqlitePool::open(&temp.path().join("service.db"))
            .await
            .unwrap();
        let repository = crate::sqlite::repository::Repository::with_model_router(
            pool,
            temp.path().join("data"),
            router.clone(),
        );
        let service = crate::sqlite::service::SqliteSpindleService::new(repository.clone());
        let project = service
            .create_project(spindle_core::models::CreateProjectInput {
                name: "Packet test".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: spindle_core::models::ReaderContract {
                    promise: "A crossing".into(),
                    style_notes: vec![],
                    boundaries: vec![],
                },
            })
            .await
            .unwrap();
        let mut packet = fixture();
        packet.project_id = project.project_id.clone();
        packet.branch_id = repository
            .get_active_branch(&project.project_id)
            .await
            .unwrap()
            .id;
        let artifact_path = temp.path().join("draft.json");
        assert!(
            service
                .draft_packet_to_artifact(
                    EditorialPolicy::CritiqueV1,
                    packet.clone(),
                    &b,
                    None,
                    &artifact_path
                )
                .await
                .unwrap_err()
                .to_string()
                .contains("budget_unknown")
        );
        let held_lock = crate::drafting::artifact::lock_artifact(&artifact_path).unwrap();
        assert!(
            service
                .draft_packet_to_artifact(
                    EditorialPolicy::CritiqueV1,
                    packet.clone(),
                    &b,
                    Some(&VerifiedFixtureTokenizer),
                    &artifact_path
                )
                .await
                .unwrap_err()
                .to_string()
                .contains("busy")
        );
        drop(held_lock);
        let artifact = service
            .draft_packet_to_artifact(
                EditorialPolicy::CritiqueV1,
                packet.clone(),
                &b,
                Some(&VerifiedFixtureTokenizer),
                &artifact_path,
            )
            .await
            .unwrap();
        assert_eq!(artifact.text.as_bytes(), response.output.as_bytes());
        let persisted: serde_json::Value =
            serde_json::from_reader(std::fs::File::open(&artifact_path).unwrap()).unwrap();
        assert!(
            persisted
                .get("prepared_packet")
                .is_some_and(|value| !value.is_null())
        );
        assert!(artifact.truncated);
        assert!(artifact.accepted_text_sha256.is_none());
        let resumed = service
            .draft_packet_to_artifact(
                EditorialPolicy::CritiqueV1,
                packet.clone(),
                &b,
                None,
                &artifact_path,
            )
            .await
            .unwrap();
        assert_eq!(
            resumed.receipt.generation_id,
            artifact.receipt.generation_id
        );
        packet.scene_card.goal = "Different goal".into();
        assert!(
            service
                .draft_packet_to_artifact(
                    EditorialPolicy::CritiqueV1,
                    packet,
                    &b,
                    None,
                    &artifact_path
                )
                .await
                .is_err()
        );
        assert_eq!(server.join().unwrap(), 2);
        let body = captured_json_body(&captured);
        assert_eq!(body["messages"][0]["content"], prepared.messages.system);
        assert_eq!(body["messages"][1]["content"], prepared.messages.user);
        assert!(
            prepared
                .messages
                .system
                .contains("Configured system framing.")
        );
        assert!(!prepared.messages.user.contains("SKILL.md"));
        assert_eq!(body["max_tokens"], 123);
        let (failure_endpoint, _, failure_server) =
            spawn_http_capture_server(vec![(500, "{}".into())]);
        let config = std::fs::read_to_string(&path)
            .unwrap()
            .replace(&endpoint, &failure_endpoint);
        std::fs::write(&path, config).unwrap();
        router.configure(Some(path.to_str().unwrap())).unwrap();
        let error = router
            .draft_packet(
                EditorialPolicy::CritiqueV1,
                fixture(),
                &b,
                Some(&VerifiedFixtureTokenizer),
            )
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("500"));
        assert_eq!(failure_server.join().unwrap(), 1);
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let cancel_endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let (received_tx, received_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocked_server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 8192];
            assert!(stream.read(&mut buffer).unwrap() > 0);
            received_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(std::time::Duration::from_secs(10));
        });
        let cancel_config = std::fs::read_to_string(&path)
            .unwrap()
            .replace(&failure_endpoint, &cancel_endpoint);
        std::fs::write(&path, cancel_config).unwrap();
        router.configure(Some(path.to_str().unwrap())).unwrap();
        let service = std::sync::Arc::new(service);
        let mut cancel_packet = fixture();
        cancel_packet.project_id = project.project_id.clone();
        cancel_packet.branch_id = repository
            .get_active_branch(&project.project_id)
            .await
            .unwrap()
            .id;
        let cancel_path = temp.path().join("cancelled.json");
        let spawned_service = service.clone();
        let spawned_packet = cancel_packet.clone();
        let spawned_path = cancel_path.clone();
        let spawned_budget = b.clone();
        let task = tokio::spawn(async move {
            spawned_service
                .draft_packet_to_artifact(
                    EditorialPolicy::CritiqueV1,
                    spawned_packet,
                    &spawned_budget,
                    Some(&VerifiedFixtureTokenizer),
                    &spawned_path,
                )
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), received_rx)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!cancel_path.exists());
        let retry = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            service.draft_packet_to_artifact(
                EditorialPolicy::CritiqueV1,
                cancel_packet,
                &b,
                Some(&VerifiedFixtureTokenizer),
                &cancel_path,
            ),
        )
        .await
        .expect("restart must stop without network");
        assert!(retry.unwrap_err().to_string().contains("busy"));
        release_tx.send(()).unwrap();
        blocked_server.join().unwrap();
    }

    #[tokio::test]
    async fn shadow_critique_enforces_rating_clearance_bounded_repair_and_draft_immutability() {
        use crate::drafting::{
            critique::CriticBudget,
            tests::{VerifiedFixtureTokenizer, budget, fixture, valid_critique_fixture},
        };
        use sha2::{Digest, Sha256};
        use spindle_core::drafting::*;

        let draft_text = "Maë braced against the floodwater as the bridge timber cracked.\nShe reached for the rope.";
        let draft_hash = format!("{:x}", Sha256::digest(draft_text.as_bytes()));

        let temp = tempfile::tempdir().unwrap();
        let pool = crate::sqlite::SqlitePool::open(&temp.path().join("critic.db"))
            .await
            .unwrap();
        let router = ModelRouter::local_only();
        let repository = crate::sqlite::repository::Repository::with_model_router(
            pool.clone(),
            temp.path().join("data"),
            router.clone(),
        );
        let service = crate::sqlite::service::SqliteSpindleService::new(repository.clone());
        let project = service
            .create_project(spindle_core::models::CreateProjectInput {
                name: "CriticShadowTest".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: spindle_core::models::ReaderContract {
                    promise: "A crossing".into(),
                    style_notes: vec![],
                    boundaries: vec![],
                },
            })
            .await
            .unwrap();
        let branch = repository
            .get_active_branch(&project.project_id)
            .await
            .unwrap();

        let mut pkt = fixture();
        pkt.project_id = project.project_id.clone();
        pkt.branch_id = branch.id.clone();
        pkt.scene_id = Some("scene-shadow-1".into());

        let mut draft_budget = budget();
        draft_budget.model_id = "venice-model".into();
        let mut critic_budget = CriticBudget::scene_default("venice-model".into(), 20_000);
        critic_budget.generation_seed = Some(101);

        // Prepare the draft artifact first so we know its exact packet_hash
        let draft_resp = serde_json::json!({
            "choices": [{"message": {"content": draft_text}, "finish_reason": "stop"}]
        })
        .to_string();

        // Temporary server just to obtain the draft artifact
        let (draft_ep, _, draft_srv) = spawn_http_capture_server(vec![(200, draft_resp)]);
        let cfg_path = temp.path().join("spindle_critic.toml");
        std::fs::write(
            &cfg_path,
            format!(
                r#"
[health_check]
enabled = false
[[agents]]
id = "venice"
name = "Venice"
provider = "venice"
endpoint = "{draft_ep}"
model = "venice-model"
ratings = ["general"]
[[routing]]
route = "draft"
agent = "venice"
max_tokens = 300
[[routing]]
route = "review"
agent = "venice"
max_tokens = 500
"#
            ),
        )
        .unwrap();
        router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

        let draft_path = temp.path().join("draft_artifact.json");
        let draft_artifact = service
            .draft_packet_to_artifact(
                EditorialPolicy::CritiqueV1,
                pkt.clone(),
                &draft_budget,
                Some(&VerifiedFixtureTokenizer),
                &draft_path,
            )
            .await
            .unwrap();
        assert_eq!(draft_srv.join().unwrap(), 1);
        assert_eq!(draft_artifact.text_sha256, draft_hash);
        let draft_bytes_snapshot = std::fs::read(&draft_path).unwrap();

        let valid_critique = valid_critique_fixture(
            draft_text,
            &draft_hash,
            &draft_artifact.packet_hash,
            Some("scene-shadow-1".into()),
        );
        let valid_critique_json = serde_json::to_string(&valid_critique).unwrap();
        let invalid_json_resp = serde_json::json!({
            "choices": [{"message": {"content": "{\"schema_version\":\"critique_v1\",\"broken\":true}"}, "finish_reason": "stop"}]
        })
        .to_string();
        let valid_json_resp = serde_json::json!({
            "choices": [{"message": {"content": valid_critique_json}, "finish_reason": "stop"}]
        })
        .to_string();

        // 1. Legacy policy is rejected without any network call
        let critique_path = temp.path().join("shadow_critique.json");
        assert!(
            service
                .critique_artifact_shadow(
                    EditorialPolicy::Legacy,
                    &draft_path,
                    &critique_path,
                    vec![],
                    &critic_budget,
                    Some(&VerifiedFixtureTokenizer),
                    2,
                )
                .await
                .is_err()
        );

        // 2. Rating denial on review route fails closed before any network call
        let mut explicit_pkt = pkt.clone();
        explicit_pkt.scene_card.rating = spindle_core::models::ContentRating::Explicit;
        let rating_err = router
            .critique_draft_shadow(
                EditorialPolicy::CritiqueV1,
                &explicit_pkt,
                &draft_artifact.packet_hash,
                draft_text,
                &draft_hash,
                vec![],
                &critic_budget,
                Some(&VerifiedFixtureTokenizer),
                2,
            )
            .await
            .unwrap_err();
        assert!(rating_err.downcast_ref::<RouteClearanceError>().is_some());

        // 3. Malformed/invalid initial output + bounded 1-shot repair succeeds and leaves draft artifact untouched
        let (critic_ep, captured_reqs, critic_srv) = spawn_http_capture_server(vec![
            (200, invalid_json_resp.clone()),
            (200, valid_json_resp.clone()),
            (200, invalid_json_resp.clone()),
            (200, invalid_json_resp),
        ]);
        let updated_cfg = std::fs::read_to_string(&cfg_path)
            .unwrap()
            .replace(&draft_ep, &critic_ep);
        std::fs::write(&cfg_path, updated_cfg).unwrap();
        router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

        let findings = vec![
            CriticFindingInput {
                finding_id: "f-det-1".into(),
                source: FeedbackSourceKind::Continuity,
                check_type: "timeline_coherence".into(),
                dimension: CriticDimension::Continuity,
                severity: DirectiveSeverity::Major,
                message: "Floodwater timing note on opening line.".into(),
                anchor: None,
                suggested_fix: None,
            },
            CriticFindingInput {
                finding_id: "f-reader-dup".into(),
                source: FeedbackSourceKind::ReaderPersona,
                check_type: "reader_note".into(),
                dimension: CriticDimension::Continuity,
                severity: DirectiveSeverity::Blocker,
                message: "Floodwater timing note on opening line!".into(),
                anchor: None,
                suggested_fix: Some("Clarify flood arrival.".into()),
            },
        ];

        let shadow_art = service
            .critique_artifact_shadow(
                EditorialPolicy::CritiqueV1,
                &draft_path,
                &critique_path,
                findings.clone(),
                &critic_budget,
                Some(&VerifiedFixtureTokenizer),
                2,
            )
            .await
            .unwrap();
        assert_eq!(shadow_art.ingested_findings.len(), 1);
        assert_eq!(shadow_art.deduplicated_finding_ids, vec!["f-reader-dup"]);
        assert_eq!(shadow_art.prompt_hashes.len(), 2);
        match &shadow_art.outcome {
            ShadowCritiqueOutcome::Critiqued {
                critique,
                repair_used,
            } => {
                assert!(*repair_used);
                assert_eq!(critique.base_text_hash, draft_hash);
                assert_eq!(critique.dimensions.len(), 10);
                assert_eq!(critique.directives.len(), 1);
            }
            other => panic!("expected Critiqued outcome after repair, got {other:?}"),
        }

        // Verify the repair prompt included the validation error from attempt 1
        let second_req_body: serde_json::Value =
            serde_json::from_str(&captured_reqs.lock().unwrap()[1]).unwrap();
        let second_user_msg = second_req_body["messages"][1]["content"].as_str().unwrap();
        assert!(second_user_msg.contains("SCHEMA / ANCHOR REPAIR REQUIRED"));

        // Verify shadow mode did NOT mutate the draft artifact on disk
        assert_eq!(std::fs::read(&draft_path).unwrap(), draft_bytes_snapshot);

        // Calling critique_artifact_shadow again with the same inputs returns cached artifact without network calls
        let cached_shadow = service
            .critique_artifact_shadow(
                EditorialPolicy::CritiqueV1,
                &draft_path,
                &critique_path,
                findings,
                &critic_budget,
                Some(&VerifiedFixtureTokenizer),
                2,
            )
            .await
            .unwrap();
        assert_eq!(cached_shadow, shadow_art);

        // 4. When both initial and repair calls return invalid JSON, outcome is CriticInvalid (retaining draft)
        let invalid_critique_path = temp.path().join("shadow_critique_invalid.json");
        let invalid_shadow = service
            .critique_artifact_shadow(
                EditorialPolicy::CritiqueV1,
                &draft_path,
                &invalid_critique_path,
                vec![],
                &critic_budget,
                Some(&VerifiedFixtureTokenizer),
                2,
            )
            .await
            .unwrap();
        match &invalid_shadow.outcome {
            ShadowCritiqueOutcome::CriticInvalid {
                attempts,
                repair_attempted,
                ..
            } => {
                assert_eq!(*attempts, 2);
                assert!(*repair_attempted);
            }
            other => panic!("expected CriticInvalid outcome, got {other:?}"),
        }
        assert_eq!(std::fs::read(&draft_path).unwrap(), draft_bytes_snapshot);
        assert_eq!(critic_srv.join().unwrap(), 4);
    }
}
