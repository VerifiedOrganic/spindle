use super::*;
use crate::drafting::{
    DraftTokenizer, PacketBudget, PreparedDraftingPacket, assemble_packet_with_system,
    chapter::{
        ChapterCriticBudget, assemble_chapter_critic_messages,
        parse_and_validate_chapter_critique_output,
    },
    critique::{
        CriticBudget, assemble_critic_messages, assemble_critic_repair_messages,
        ingest_and_dedup_findings, parse_and_validate_critique_output,
    },
    line_edit::{LineEditBudget, assemble_line_edit_messages},
    revision::{
        JudgeBudget, RevisionBudget, assemble_pairwise_judge_messages, assemble_revision_messages,
        parse_and_validate_single_judge_pass,
    },
};
use spindle_core::drafting::{
    ChapterCoverageV1, ChapterCritiqueV1, CriticFindingInput, CritiqueDirective, DeblindedArm,
    DraftingPacketV1, EditorialPolicy, EditorialRouteAvailability, IngestedFinding,
    ShadowCritiqueOutcome, SinglePassJudgeRecord,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ChapterCriticRunResult {
    pub critique: ChapterCritiqueV1,
    pub prompt_hash: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub model_call_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ShadowCritiqueRunResult {
    pub ingested_findings: Vec<IngestedFinding>,
    pub deduplicated_finding_ids: Vec<String>,
    pub prompt_hashes: Vec<String>,
    pub model_call_ids: Vec<String>,
    pub tokens_used: usize,
    pub outcome: ShadowCritiqueOutcome,
}

#[derive(Debug, Clone)]
pub struct RevisionCandidateRunResult {
    pub route_used: String,
    pub prompt_hash: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub response: ModelResponse,
}

#[derive(Debug, Clone)]
pub struct SinglePassJudgeRunResult {
    pub prompt_hash: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub record: SinglePassJudgeRecord,
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

    /// Observe route/tokenizer availability for the editorial controller without
    /// dispatching a model call. A missing or uncleared route reports `false`.
    pub fn editorial_route_availability(&self, rating: &str) -> EditorialRouteAvailability {
        let Ok(runtime) = self.runtime.read() else {
            return EditorialRouteAvailability::default();
        };
        let route_ok = |name: &str| resolve_cleared_route(&runtime, name, Some(rating)).is_ok();
        let mut availability = EditorialRouteAvailability {
            draft: route_ok("draft"),
            review: route_ok("review"),
            line_edit: route_ok("line_edit"),
            extract: route_ok("extract"),
            verified_tokenizer: false,
        };
        if let Ok(route) = resolve_cleared_route(&runtime, "draft", Some(rating))
            && let Ok(agent) = http_agent_for_route(&runtime, route)
        {
            availability.verified_tokenizer =
                agent.config.tokenizer.is_some() && self.counter_scope(&runtime, route).is_ok();
        }
        availability
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
        let (initial_messages, initial_hash, initial_input_tokens) = assemble_critic_messages(
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
        let resp1_output_tokens = resp1
            .usage
            .as_ref()
            .and_then(|u| u.output_tokens)
            .map(|n| n as usize)
            .unwrap_or_else(|| {
                t.and_then(|tok| tok.count_text(&resp1.output).ok())
                    .unwrap_or(0)
            });
        let call1_tokens = initial_input_tokens.saturating_add(resp1_output_tokens);

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
                tokens_used: call1_tokens,
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
                        tokens_used: call1_tokens,
                        outcome: ShadowCritiqueOutcome::CriticInvalid {
                            reason: first_err.to_string(),
                            attempts: 1,
                            repair_attempted: false,
                        },
                    });
                }

                // Bounded 1-shot schema/anchor repair inside the call cap
                let (repair_messages, repair_hash, repair_input_tokens) =
                    match assemble_critic_repair_messages(
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
                                tokens_used: call1_tokens,
                                outcome: ShadowCritiqueOutcome::CriticInvalid {
                                    reason: format!(
                                        "{first_err}; repair assembly failed: {prep_err}"
                                    ),
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
                let resp2_output_tokens = resp2
                    .usage
                    .as_ref()
                    .and_then(|u| u.output_tokens)
                    .map(|n| n as usize)
                    .unwrap_or_else(|| {
                        t.and_then(|tok| tok.count_text(&resp2.output).ok())
                            .unwrap_or(0)
                    });
                let total_critic_tokens = call1_tokens
                    .saturating_add(repair_input_tokens)
                    .saturating_add(resp2_output_tokens);

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
                        tokens_used: total_critic_tokens,
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
                        tokens_used: total_critic_tokens,
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

    /// Derive the effective `RevisionBudget` (`18,000` input / `3,000` output / `512` framing)
    /// and provider settings metadata for prior-draft revision (`revise` route falling back to `draft`).
    pub fn eval_revision_budget_and_settings(
        &self,
        rating: &str,
        generation_seed: Option<i64>,
    ) -> anyhow::Result<(RevisionBudget, serde_json::Value)> {
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let (route_name, route) = match resolve_cleared_route(&runtime, "revise", Some(rating)) {
            Ok(r) => ("revise", r.clone()),
            Err(_) => (
                "draft",
                resolve_cleared_route(&runtime, "draft", Some(rating))?.clone(),
            ),
        };
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: revision framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        let context_window = agent.config.max_context.unwrap_or(32_768);
        let mut budget = RevisionBudget::scene_default(agent.config.model.clone(), context_window);
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
                "route": route_name,
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
                "route": route_name,
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

    /// Derive the effective `JudgeBudget` (`16,000` input / `1,000` output / `512` framing)
    /// and provider settings metadata for the `review` route pairwise acceptance judge.
    pub fn eval_judge_budget_and_settings(
        &self,
        route_name: &str,
        rating: &str,
        generation_seed: Option<i64>,
    ) -> anyhow::Result<(JudgeBudget, serde_json::Value)> {
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let route = resolve_cleared_route(&runtime, route_name, Some(rating))?.clone();
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: judge framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        let context_window = agent.config.max_context.unwrap_or(32_768);
        let mut budget = JudgeBudget::scene_default(agent.config.model.clone(), context_window);
        if let Some(max_tokens) = route.max_tokens {
            budget.output_reserve = budget.output_reserve.max(max_tokens as usize);
        }
        budget.generation_seed = generation_seed.or(route.seed);
        let effective_max_tokens = route
            .max_tokens
            .unwrap_or_else(|| u32::try_from(budget.output_reserve).unwrap_or(1_000));
        let settings = if agent.config.tokenizer.is_some() {
            let scope = self.counter_scope(&runtime, &route)?;
            let encoder = self.configured_tokenizer(&scope)?;
            serde_json::json!({
                "route": route_name,
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
                "route": route_name,
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

    /// Execute a prior-draft revision pass over `incumbent_text` using the selected
    /// actionable `directives` and `strengths_to_preserve` on the `revise` route
    /// (falling back to `draft` when `revise` is not separately configured).
    #[allow(clippy::too_many_arguments)]
    pub async fn revise_draft_candidate(
        &self,
        policy: EditorialPolicy,
        packet: &DraftingPacketV1,
        incumbent_text: &str,
        incumbent_text_hash: &str,
        directives: &[CritiqueDirective],
        strengths_to_preserve: &[String],
        budget: &RevisionBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
    ) -> anyhow::Result<RevisionCandidateRunResult> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "prior-draft revision requires critique_v1 policy"
        );
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let rating = packet.scene_card.rating.as_str();
        let (route_name, mut route) = match resolve_cleared_route(&runtime, "revise", Some(rating))
        {
            Ok(r) => ("revise", r.clone()),
            Err(_) => (
                "draft",
                resolve_cleared_route(&runtime, "draft", Some(rating))?.clone(),
            ),
        };
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: revision framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        anyhow::ensure!(
            agent.config.model == budget.model_id,
            "budget_unknown: resolved revision model does not match revision budget"
        );
        let reserve: u32 = budget.output_reserve.try_into()?;
        anyhow::ensure!(reserve > 0, "revision output reserve must be positive");
        anyhow::ensure!(
            route.max_tokens.is_none_or(|max| max <= reserve),
            "configured revision output exceeds revision reservation"
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
                "revision window exceeds configured model context"
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
        let (messages, prompt_hash, input_tokens) = assemble_revision_messages(
            packet,
            incumbent_text,
            incumbent_text_hash,
            directives,
            strengths_to_preserve,
            budget,
            t,
            &route_system,
        )?;
        route.system_prompt = messages.system.clone();
        let req = ModelRequest {
            route: route_name.into(),
            prompt: messages.user.clone(),
            rating: Some(rating.into()),
            context: Some(RequestContext {
                project_id: Some(packet.project_id.clone()),
                scene_id: packet.scene_id.clone(),
                run_id: packet.run_id.clone(),
                prompt_template_id: Some("draft.revise_v1".into()),
                prompt_version: Some("v1".into()),
                context_packet_hash: Some(prompt_hash.clone()),
                ..Default::default()
            }),
        };
        let verified_count = if let (Some(scope), Some(encoder)) = (&scope, &configured)
            && encoder.config().verify_each_prompt
        {
            let receipt = self
                .probe_count(scope, &messages, Some(rating), req.context.as_ref())
                .await?;
            encoder.verify_count(&messages, receipt.input_tokens)?;
            Some(receipt.input_tokens)
        } else {
            None
        };
        #[cfg(any(test, feature = "test-support"))]
        self.record_dispatch(DispatchRecord::Dispatch {
            route: req.route.clone(),
            agent: dispatch_agent_label(&runtime, &route),
            rating: req.rating.clone(),
            prompt: req.prompt.clone(),
        })?;
        let start = std::time::Instant::now();
        let mut resp = if let Some(scope) = &scope {
            self.run_ollama_packet(scope, &route, &messages).await
        } else {
            self.run_http_inner(&runtime, &route, req.rating.as_deref(), &req.prompt, false)
                .await
        };
        self.record_usage(
            &req.route,
            req.rating.as_deref(),
            req.context.as_ref(),
            &req.prompt,
            false,
            Some(&route),
            start,
            &mut resp,
        )
        .await;
        let resp = resp?;
        if let Some(encoder) = &configured {
            let count = resp
                .usage
                .as_ref()
                .and_then(|u| u.input_tokens)
                .context("native revision missing prompt_eval_count; recovery required")?;
            encoder.verify_count(&messages, count.try_into()?)?;
            if let Some(verified) = verified_count {
                anyhow::ensure!(
                    count == verified as u64,
                    "native revision prompt count differs from verified probe; recovery required"
                );
            }
        }
        let output_tokens = resp
            .usage
            .as_ref()
            .and_then(|u| u.output_tokens)
            .map(|n| n as usize)
            .unwrap_or_else(|| {
                t.and_then(|tok| tok.count_text(&resp.output).ok())
                    .unwrap_or(0)
            });

        Ok(RevisionCandidateRunResult {
            route_used: route_name.into(),
            prompt_hash,
            input_tokens,
            output_tokens,
            response: resp,
        })
    }

    /// Run one chapter advisory critic over a deterministic chapter index/window.
    /// The returned critique is validated against exact scene text; it never
    /// mutates prose or applies an edit.
    #[allow(clippy::too_many_arguments)]
    pub async fn chapter_critic(
        &self,
        policy: EditorialPolicy,
        rating: &str,
        coverage: &ChapterCoverageV1,
        scene_texts: &BTreeMap<String, String>,
        feedback: &[CriticFindingInput],
        budget: &ChapterCriticBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
    ) -> anyhow::Result<ChapterCriticRunResult> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "chapter critic requires critique_v1 policy"
        );
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let chapter_text_hash = chapter_aggregate_hash(coverage, scene_texts);
        let mut route = resolve_cleared_route(&runtime, "review", Some(rating))?.clone();
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: chapter critic framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        anyhow::ensure!(
            agent.config.model == budget.model_id,
            "budget_unknown: resolved chapter-critic model does not match budget"
        );
        let reserve: u32 = budget.output_reserve.try_into()?;
        anyhow::ensure!(
            reserve > 0,
            "chapter critic output reserve must be positive"
        );
        route.max_tokens = Some(route.max_tokens.map_or(reserve, |max| max.min(reserve)));
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
                "chapter critic window exceeds configured model context"
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
        let (messages, prompt_hash, input_tokens) = assemble_chapter_critic_messages(
            coverage,
            scene_texts,
            feedback,
            budget,
            t,
            &route_system,
        )?;
        route.system_prompt = messages.system.clone();
        let req = ModelRequest {
            route: "review".into(),
            prompt: messages.user.clone(),
            rating: Some(rating.into()),
            context: Some(RequestContext {
                project_id: None,
                scene_id: None,
                run_id: None,
                prompt_template_id: Some("review.chapter_critic_v1".into()),
                prompt_version: Some("v1".into()),
                context_packet_hash: Some(prompt_hash.clone()),
                ..Default::default()
            }),
        };
        let verified_count = if let (Some(scope), Some(encoder)) = (&scope, &configured)
            && encoder.config().verify_each_prompt
        {
            let receipt = self
                .probe_count(scope, &messages, Some(rating), req.context.as_ref())
                .await?;
            encoder.verify_count(&messages, receipt.input_tokens)?;
            Some(receipt.input_tokens)
        } else {
            None
        };
        #[cfg(any(test, feature = "test-support"))]
        self.record_dispatch(DispatchRecord::Dispatch {
            route: req.route.clone(),
            agent: dispatch_agent_label(&runtime, &route),
            rating: req.rating.clone(),
            prompt: req.prompt.clone(),
        })?;
        let start = std::time::Instant::now();
        let mut response = if let Some(scope) = &scope {
            self.run_ollama_packet(scope, &route, &messages).await
        } else {
            self.run_http_inner(&runtime, &route, req.rating.as_deref(), &req.prompt, false)
                .await
        };
        self.record_usage(
            &req.route,
            req.rating.as_deref(),
            req.context.as_ref(),
            &req.prompt,
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
                .and_then(|usage| usage.input_tokens)
                .context("native chapter critic missing prompt_eval_count; recovery required")?;
            encoder.verify_count(&messages, count.try_into()?)?;
            if let Some(verified) = verified_count {
                anyhow::ensure!(
                    count == verified as u64,
                    "native chapter critic prompt count differs from verified probe; recovery required"
                );
            }
        }
        let output_tokens = response
            .usage
            .as_ref()
            .and_then(|usage| usage.output_tokens)
            .map(|count| count as usize)
            .unwrap_or_else(|| {
                t.and_then(|tokenizer| tokenizer.count_text(&response.output).ok())
                    .unwrap_or(0)
            });
        let critique = parse_and_validate_chapter_critique_output(
            &response.output,
            coverage.chapter_number,
            &chapter_text_hash,
            coverage,
            scene_texts,
        )?;
        Ok(ChapterCriticRunResult {
            critique,
            prompt_hash,
            input_tokens,
            output_tokens,
            model_call_ids: response.call_id.into_iter().collect(),
        })
    }

    /// Execute an optional line-edit polish pass on the `line_edit` route. There
    /// is no fallback to `draft`: an unconfigured line-edit route is an honest
    /// skip. The returned candidate still passes deterministic checks and the
    /// swapped-order acceptance judge before any promotion.
    pub async fn line_edit_candidate(
        &self,
        policy: EditorialPolicy,
        packet: &DraftingPacketV1,
        accepted_text: &str,
        accepted_text_hash: &str,
        budget: &LineEditBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
    ) -> anyhow::Result<RevisionCandidateRunResult> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "line edit requires critique_v1 policy"
        );
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let rating = packet.scene_card.rating.as_str();
        let mut route = resolve_cleared_route(&runtime, "line_edit", Some(rating))?.clone();
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: line-edit framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        anyhow::ensure!(
            agent.config.model == budget.model_id,
            "budget_unknown: resolved line-edit model does not match budget"
        );
        let reserve: u32 = budget.output_reserve.try_into()?;
        anyhow::ensure!(reserve > 0, "line-edit output reserve must be positive");
        anyhow::ensure!(
            route.max_tokens.is_none_or(|max| max <= reserve),
            "configured line-edit output exceeds line-edit reservation"
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
                "line-edit window exceeds configured model context"
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
        let (messages, prompt_hash, input_tokens) = assemble_line_edit_messages(
            packet,
            accepted_text,
            accepted_text_hash,
            budget,
            t,
            &route_system,
        )?;
        route.system_prompt = messages.system.clone();
        let req = ModelRequest {
            route: "line_edit".into(),
            prompt: messages.user.clone(),
            rating: Some(rating.into()),
            context: Some(RequestContext {
                project_id: Some(packet.project_id.clone()),
                scene_id: packet.scene_id.clone(),
                run_id: packet.run_id.clone(),
                prompt_template_id: Some("draft.line_edit_v1".into()),
                prompt_version: Some("v1".into()),
                context_packet_hash: Some(prompt_hash.clone()),
                ..Default::default()
            }),
        };
        let verified_count = if let (Some(scope), Some(encoder)) = (&scope, &configured)
            && encoder.config().verify_each_prompt
        {
            let receipt = self
                .probe_count(scope, &messages, Some(rating), req.context.as_ref())
                .await?;
            encoder.verify_count(&messages, receipt.input_tokens)?;
            Some(receipt.input_tokens)
        } else {
            None
        };
        #[cfg(any(test, feature = "test-support"))]
        self.record_dispatch(DispatchRecord::Dispatch {
            route: req.route.clone(),
            agent: dispatch_agent_label(&runtime, &route),
            rating: req.rating.clone(),
            prompt: req.prompt.clone(),
        })?;
        let start = std::time::Instant::now();
        let mut response = if let Some(scope) = &scope {
            self.run_ollama_packet(scope, &route, &messages).await
        } else {
            self.run_http_inner(&runtime, &route, req.rating.as_deref(), &req.prompt, false)
                .await
        };
        self.record_usage(
            &req.route,
            req.rating.as_deref(),
            req.context.as_ref(),
            &req.prompt,
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
                .and_then(|usage| usage.input_tokens)
                .context("native line-edit missing prompt_eval_count; recovery required")?;
            encoder.verify_count(&messages, count.try_into()?)?;
            if let Some(verified) = verified_count {
                anyhow::ensure!(
                    count == verified as u64,
                    "native line-edit prompt count differs from verified probe; recovery required"
                );
            }
        }
        let output_tokens = response
            .usage
            .as_ref()
            .and_then(|usage| usage.output_tokens)
            .map(|count| count as usize)
            .unwrap_or_else(|| {
                t.and_then(|tokenizer| tokenizer.count_text(&response.output).ok())
                    .unwrap_or(0)
            });
        Ok(RevisionCandidateRunResult {
            route_used: "line_edit".into(),
            prompt_hash,
            input_tokens,
            output_tokens,
            response,
        })
    }

    /// Execute a single blinded pairwise acceptance judge pass on the `review` route.
    /// Any transport, token verification, or JSON validation error is returned so the
    /// caller can fail closed (`JudgeInvalidOrUnavailable`) and retain the incumbent.
    #[allow(clippy::too_many_arguments)]
    pub async fn judge_candidate_single_pass(
        &self,
        policy: EditorialPolicy,
        packet: &DraftingPacketV1,
        order_index: u8,
        passage_1_arm: DeblindedArm,
        passage_1_text: &str,
        passage_2_arm: DeblindedArm,
        passage_2_text: &str,
        budget: &JudgeBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        chapter_context: Option<&str>,
    ) -> anyhow::Result<SinglePassJudgeRunResult> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "pairwise acceptance judge requires critique_v1 policy"
        );
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let rating = packet.scene_card.rating.as_str();
        let mut route = resolve_cleared_route(&runtime, "review", Some(rating))?.clone();
        anyhow::ensure!(
            route.adapter_kind == "http",
            "budget_unknown: judge framing is only verified for HTTP adapters"
        );
        let agent = http_agent_for_route(&runtime, &route)?;
        anyhow::ensure!(
            agent.config.model == budget.model_id,
            "budget_unknown: resolved review model does not match judge budget"
        );
        let reserve: u32 = budget.output_reserve.try_into()?;
        anyhow::ensure!(reserve > 0, "judge output reserve must be positive");
        // Allow the judge call to cap output tokens at the judge's output_reserve (1,000)
        // even if the shared review route has a larger default max_tokens (e.g. 1,500).
        route.max_tokens = Some(route.max_tokens.map_or(reserve, |max| max.min(reserve)));
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
                "judge window exceeds configured review model context"
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
        let (messages, prompt_hash, input_tokens) = assemble_pairwise_judge_messages(
            packet,
            passage_1_text,
            passage_2_text,
            budget,
            t,
            &route_system,
            chapter_context,
        )?;
        route.system_prompt = messages.system.clone();
        let req = ModelRequest {
            route: "review".into(),
            prompt: messages.user.clone(),
            rating: Some(rating.into()),
            context: Some(RequestContext {
                project_id: Some(packet.project_id.clone()),
                scene_id: packet.scene_id.clone(),
                run_id: packet.run_id.clone(),
                prompt_template_id: Some(format!("review.pairwise_judge_v1.pass_{order_index}")),
                prompt_version: Some("v1".into()),
                context_packet_hash: Some(prompt_hash.clone()),
                ..Default::default()
            }),
        };
        let verified_count = if let (Some(scope), Some(encoder)) = (&scope, &configured)
            && encoder.config().verify_each_prompt
        {
            let receipt = self
                .probe_count(scope, &messages, Some(rating), req.context.as_ref())
                .await?;
            encoder.verify_count(&messages, receipt.input_tokens)?;
            Some(receipt.input_tokens)
        } else {
            None
        };
        #[cfg(any(test, feature = "test-support"))]
        self.record_dispatch(DispatchRecord::Dispatch {
            route: req.route.clone(),
            agent: dispatch_agent_label(&runtime, &route),
            rating: req.rating.clone(),
            prompt: req.prompt.clone(),
        })?;
        let start = std::time::Instant::now();
        let mut resp = if let Some(scope) = &scope {
            self.run_ollama_packet(scope, &route, &messages).await
        } else {
            self.run_http_inner(&runtime, &route, req.rating.as_deref(), &req.prompt, false)
                .await
        };
        self.record_usage(
            &req.route,
            req.rating.as_deref(),
            req.context.as_ref(),
            &req.prompt,
            false,
            Some(&route),
            start,
            &mut resp,
        )
        .await;
        let resp = resp?;
        if let Some(encoder) = &configured {
            let count = resp
                .usage
                .as_ref()
                .and_then(|u| u.input_tokens)
                .context("native judge missing prompt_eval_count; recovery required")?;
            encoder.verify_count(&messages, count.try_into()?)?;
            if let Some(verified) = verified_count {
                anyhow::ensure!(
                    count == verified as u64,
                    "native judge prompt count differs from verified probe; recovery required"
                );
            }
        }
        let output_tokens = resp
            .usage
            .as_ref()
            .and_then(|u| u.output_tokens)
            .map(|n| n as usize)
            .unwrap_or_else(|| {
                t.and_then(|tok| tok.count_text(&resp.output).ok())
                    .unwrap_or(0)
            });

        let record = parse_and_validate_single_judge_pass(
            &resp.output,
            order_index,
            passage_1_arm,
            passage_2_arm,
            prompt_hash.clone(),
            resp.call_id,
        )?;

        Ok(SinglePassJudgeRunResult {
            prompt_hash,
            input_tokens,
            output_tokens,
            record,
        })
    }
}

fn chapter_aggregate_hash(
    coverage: &ChapterCoverageV1,
    scene_texts: &BTreeMap<String, String>,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for scene in &coverage.chapter_index {
        hasher.update(scene.scene_id.as_bytes());
        hasher.update([0]);
        hasher.update(scene.text_sha256.as_bytes());
        hasher.update([0]);
        if let Some(text) = scene_texts.get(&scene.scene_id) {
            hasher.update(text.as_bytes());
        }
        hasher.update([0xff]);
    }
    format!("{:x}", hasher.finalize())
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
        let pkt_src = pkt.source_versions[0].clone();
        pkt.continuity.knowledge_boundaries.push(PacketItem {
            source: pkt_src.clone(),
            proposition_id: "kb-shadow-1".into(),
            text: "SHADOW_SENTINEL_KB_NO_TOWER_KEY".into(),
            relevance: 95,
            recency: 10,
        });
        pkt.continuity.facts.push(PacketItem {
            source: pkt_src.clone(),
            proposition_id: "fact-shadow-1".into(),
            text: "SHADOW_SENTINEL_FACT_ROPE_FRAYED".into(),
            relevance: 90,
            recency: 9,
        });
        pkt.voice.rules.push(PacketItem {
            source: pkt_src.clone(),
            proposition_id: "voice-shadow-1".into(),
            text: "SHADOW_SENTINEL_VOICE_SHORT_CADENCE".into(),
            relevance: 88,
            recency: 8,
        });
        pkt.editorial_memory.recurring_images.push(PacketItem {
            source: pkt_src,
            proposition_id: "mem-shadow-1".into(),
            text: "SHADOW_SENTINEL_MEMORY_FLOOD_FOAM".into(),
            relevance: 80,
            recency: 7,
        });

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

        // Verify both initial and repair prompts included the full CritiqueV1 contract/schema/template and packet context sentinels
        let first_req_body: serde_json::Value =
            serde_json::from_str(&captured_reqs.lock().unwrap()[0]).unwrap();
        let first_user_msg = first_req_body["messages"][1]["content"].as_str().unwrap();
        assert!(first_user_msg.contains("CRITIQUE_V1 FIELD & ENUM REQUIREMENTS"));
        assert!(first_user_msg.contains("CRITIQUE_V1 OUTPUT TEMPLATE"));
        assert!(first_user_msg.contains("CRITIQUE_V1 JSON SCHEMA"));
        assert!(first_user_msg.contains("SHADOW_SENTINEL_KB_NO_TOWER_KEY"));
        assert!(first_user_msg.contains("SHADOW_SENTINEL_FACT_ROPE_FRAYED"));
        assert!(first_user_msg.contains("SHADOW_SENTINEL_VOICE_SHORT_CADENCE"));
        assert!(first_user_msg.contains("SHADOW_SENTINEL_MEMORY_FLOOD_FOAM"));

        let second_req_body: serde_json::Value =
            serde_json::from_str(&captured_reqs.lock().unwrap()[1]).unwrap();
        let second_user_msg = second_req_body["messages"][1]["content"].as_str().unwrap();
        assert!(second_user_msg.contains("CRITIQUE_V1 OUTPUT TEMPLATE"));
        assert!(second_user_msg.contains("CRITIQUE_V1 JSON SCHEMA"));
        assert!(second_user_msg.contains("SHADOW_SENTINEL_KB_NO_TOWER_KEY"));
        assert!(second_user_msg.contains("SHADOW_SENTINEL_VOICE_SHORT_CADENCE"));
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

    fn make_valid_critique_json(
        packet_hash: &str,
        base_hash: &str,
        scene_id: Option<&str>,
        base_text: &str,
    ) -> serde_json::Value {
        let dims: Vec<serde_json::Value> = spindle_core::drafting::CriticDimension::ALL
            .iter()
            .map(|d| {
                serde_json::json!({
                    "dimension": d.as_str(),
                    "score_1_to_5": 4,
                    "evidence_spans": [],
                    "rationale": format!("Assessment of {}.", d.as_str()),
                })
            })
            .collect();
        let quote: String = base_text.chars().take(12).collect();
        let end_byte = quote.len();
        serde_json::json!({
            "schema_version": "critique_v1",
            "base_text_hash": base_hash,
            "packet_hash": packet_hash,
            "rubric_version": "scene-critic-v1",
            "dimensions": dims,
            "directives": [
                {
                    "id": "dir-1",
                    "dimension": "tension_turn",
                    "severity": "major",
                    "priority": 1,
                    "anchor": {
                        "scene_id": scene_id,
                        "base_text_hash": base_hash,
                        "start_byte": 0,
                        "end_byte": end_byte,
                        "exact_quote": quote,
                        "context_before": null,
                        "context_after": null
                    },
                    "diagnosis": "Opening beat stalls physical momentum.",
                    "requested_change": "Sharpen opening physical action while keeping all locked items.",
                    "preserve": ["POV discipline"],
                    "success_test": "Opening moves immediately into physical action.",
                    "source_finding_ids": [],
                    "scope": "span"
                }
            ],
            "strengths_to_preserve": ["Strong closing image"],
            "requires_author_decision": [],
            "no_edit_reason": null
        })
    }

    fn make_judge_json(overall_preference: &str) -> serde_json::Value {
        let mut rubric = serde_json::Map::new();
        for dim in spindle_core::drafting::JudgeRubricDimension::ALL {
            rubric.insert(
                dim.as_str().to_string(),
                serde_json::json!({
                    "score": 4,
                    "rationale": format!("Evidence for {}.", dim.as_str())
                }),
            );
        }
        serde_json::json!({
            "rubric": rubric,
            "overall_preference": overall_preference,
            "deciding_factor": "Stronger physical turn and cleaner continuity.",
            "strengths_passage_1": ["Atmosphere"],
            "strengths_passage_2": ["Pacing"]
        })
    }

    struct RecordingRevisionGate {
        admitted: std::sync::Mutex<Vec<(String, u32, String)>>,
        seen: std::sync::Mutex<std::collections::BTreeSet<(String, u32, String)>>,
        pass_2_claim: tokio::sync::Notify,
    }

    impl Default for RecordingRevisionGate {
        fn default() -> Self {
            Self {
                admitted: std::sync::Mutex::new(Vec::new()),
                seen: std::sync::Mutex::new(std::collections::BTreeSet::new()),
                pass_2_claim: tokio::sync::Notify::new(),
            }
        }
    }

    impl crate::drafting::revision::RevisionBudgetGate for RecordingRevisionGate {
        fn admit(
            &self,
            action: spindle_core::drafting::EditorialActionKind,
            attempt: u32,
            claim: &str,
        ) -> Result<crate::drafting::revision::RevisionBudgetAdmission, String> {
            let key = (action.as_str().to_string(), attempt, claim.to_string());
            let mut seen = self.seen.lock().map_err(|_| "gate lock".to_string())?;
            if !seen.insert(key.clone()) {
                return Err(format!("duplicate dispatch claim {key:?}"));
            }
            self.admitted
                .lock()
                .map_err(|_| "gate lock".to_string())?
                .push(key);
            if claim == "judge_pass_2" {
                self.pass_2_claim.notify_one();
            }
            Ok(crate::drafting::revision::RevisionBudgetAdmission {
                admission_id: format!("{}:{}:{}", action.as_str(), attempt, claim),
                action,
                reserved: spindle_core::drafting::EditorialBudgetUsage::default(),
            })
        }

        fn reconcile(
            &self,
            _admission: &crate::drafting::revision::RevisionBudgetAdmission,
            _usage: Option<crate::drafting::revision::RevisionPassUsage>,
            _model_call_ids: Vec<String>,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    /// Serves fixed JSON responses and blocks the fourth request until the test
    /// releases it, so a concurrent resume can race the in-flight pass-2 call.
    fn spawn_blocking_pass_two_server(
        responses: Vec<(u16, String)>,
        release: std::sync::mpsc::Receiver<()>,
    ) -> (String, std::thread::JoinHandle<usize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let address = listener.local_addr().expect("listener address");
        let handle = std::thread::spawn(move || {
            let mut handled = 0usize;
            for (index, (status, body)) in responses.into_iter().enumerate() {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut raw = Vec::new();
                let mut chunk = [0_u8; 65536];
                loop {
                    let count = stream.read(&mut chunk).expect("read request");
                    if count == 0 {
                        break;
                    }
                    raw.extend_from_slice(&chunk[..count]);
                    let current = String::from_utf8_lossy(&raw);
                    if let Some((headers, body_part)) = current.split_once("\r\n\r\n") {
                        let expected_len = headers
                            .lines()
                            .find_map(|line| {
                                let (k, v) = line.split_once(':')?;
                                k.trim()
                                    .eq_ignore_ascii_case("content-length")
                                    .then(|| v.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if body_part.len() >= expected_len {
                            break;
                        }
                    }
                }
                if index == 3 {
                    release.recv().expect("release pass 2");
                }
                let status_text = match status {
                    200 => "OK",
                    _ => "Test Response",
                };
                let response = format!(
                    "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                stream.flush().expect("flush response");
                handled += 1;
            }
            handled
        });
        (format!("http://{address}/v1"), handle)
    }

    #[tokio::test]
    async fn revision_loop_promotes_double_order_winner_and_recovers_from_crash_at_every_boundary()
    {
        use crate::drafting::critique::CriticBudget;
        use crate::drafting::revision::{
            BoundaryCrashPoint, JudgeBudget, RevisionBudget, RevisionLoopOptions,
            select_initial_judge_order,
        };
        use crate::drafting::tests::{VerifiedFixtureTokenizer, budget, fixture};
        use crate::sqlite::SqliteSpindleService;
        use spindle_core::drafting::{DeblindedArm, EditorialPolicy, EditorialStage};

        let temp = tempfile::tempdir().unwrap();
        let gate = std::sync::Arc::new(RecordingRevisionGate::default());
        let draft_prose = "Cold floodwater rose around the stone sluice.\n\nThe lantern flickered in the damp wind.";
        let revised_prose = "Cold floodwater surged against the stone sluice as Mara braced her shoulder.\n\nThe lantern flickered in the damp wind.";

        // Spawn draft capture server for initial draft generation
        let draft_reply = serde_json::json!({
            "choices": [{"message": {"content": draft_prose}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 18, "total_tokens": 118}
        })
        .to_string();
        let (draft_url, _, draft_srv) = spawn_http_capture_server(vec![(200, draft_reply)]);

        let pool = crate::sqlite::SqlitePool::open(&temp.path().join("spindle.db"))
            .await
            .unwrap();
        let router = ModelRouter::local_only();
        let cfg_path = temp.path().join("spindle.toml");
        let cfg_text = format!(
            "[health_check]\nenabled = false\n[[agents]]\nid = \"venice\"\nname = \"Venice\"\nprovider = \"venice\"\nendpoint = \"{draft_url}\"\nmodel = \"venice-model\"\nratings = [\"general\"]\n[[routing]]\nroute = \"draft\"\nagent = \"venice\"\nmax_tokens = 300\n[[routing]]\nroute = \"review\"\nagent = \"venice\"\nmax_tokens = 500\n"
        );
        std::fs::write(&cfg_path, &cfg_text).unwrap();
        router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

        let repository = crate::sqlite::repository::Repository::with_model_router(
            pool.clone(),
            temp.path().join("data"),
            router.clone(),
        );
        let service = SqliteSpindleService::new(repository.clone());
        let created = service
            .create_project(spindle_core::models::CreateProjectInput {
                name: "Revision Loop Test".into(),
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
        let active_branch = repository
            .get_active_branch(&created.project_id)
            .await
            .unwrap();

        let mut pkt = fixture();
        pkt.project_id = created.project_id.clone();
        pkt.branch_id = active_branch.id.clone();
        for src in &mut pkt.source_versions {
            src.branch_id = active_branch.id.clone();
        }
        pkt.scene_card.source.branch_id = active_branch.id.clone();
        pkt.scene_card.pov.source.branch_id = active_branch.id.clone();
        for c in &mut pkt.hard_constraints {
            c.source.branch_id = active_branch.id.clone();
        }

        let mut draft_budget = budget();
        draft_budget.model_id = "venice-model".into();
        let initial_draft_path = temp.path().join("initial_draft.artifact.json");
        let draft_art = service
            .draft_packet_to_artifact(
                EditorialPolicy::CritiqueV1,
                pkt.clone(),
                &draft_budget,
                Some(&VerifiedFixtureTokenizer),
                &initial_draft_path,
            )
            .await
            .unwrap();
        assert_eq!(draft_srv.join().unwrap(), 1);

        struct StubExtractor;
        impl spindle_core::drafting::extraction::AcceptedTextExtractor for StubExtractor {
            fn extract<'a>(
                &'a self,
                request: &'a spindle_core::drafting::extraction::AcceptedTextExtractionRequestV1,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = std::result::Result<
                                spindle_core::drafting::extraction::CompatibilityExtractionV1,
                                spindle_core::drafting::extraction::ExtractionFailure,
                            >,
                        > + Send
                        + 'a,
                >,
            > {
                Box::pin(async move {
                    Ok(
                        spindle_core::drafting::extraction::CompatibilityExtractionV1 {
                            canon_audit: None,
                            accepted_text_sha256: request.accepted.text_sha256.clone(),
                            summary: format!("Extracted {}", &request.accepted.text_sha256[..8]),
                            tone: Some("tense".into()),
                            character_states: vec![],
                            canonical_facts: vec![],
                            relationship_updates: vec![],
                            beats: vec![],
                            continuity_notes: vec![],
                        },
                    )
                })
            }
        }

        // Accept and extract the incumbent artifact before revision to verify that winning
        // promotion invalidates text-bound extraction state and requires fresh extraction.
        crate::drafting::artifact::accept_artifact(&initial_draft_path, &draft_art.text_sha256)
            .unwrap();
        let extracted_incumbent = crate::drafting::artifact::extract_accepted(
            &initial_draft_path,
            &StubExtractor,
            vec![],
            vec![],
        )
        .await
        .unwrap();
        assert_eq!(
            extracted_incumbent
                .extraction
                .as_ref()
                .unwrap()
                .accepted_text_sha256,
            draft_art.text_sha256
        );
        assert!(extracted_incumbent.extraction_request.is_some());
        assert_eq!(extracted_incumbent.extraction_attempts, 1);
        let initial_draft_bytes = std::fs::read(&initial_draft_path).unwrap();

        let rev_sha = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(revised_prose.as_bytes()))
        };
        let (p1_arm, _p2_arm) = select_initial_judge_order(
            &draft_art.packet_hash,
            &draft_art.text_sha256,
            &rev_sha,
            1,
            None,
        );
        let pass1_pref = if p1_arm == DeblindedArm::Candidate {
            "passage_1"
        } else {
            "passage_2"
        };
        let pass2_pref = if p1_arm == DeblindedArm::Candidate {
            "passage_2"
        } else {
            "passage_1"
        };

        let critique_json = make_valid_critique_json(
            &draft_art.packet_hash,
            &draft_art.text_sha256,
            draft_art.scene_id.as_deref(),
            &draft_art.text,
        );
        let four_paid_replies = vec![
            (
                200,
                serde_json::json!({
                    "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                })
                .to_string(),
            ),
            (
                200,
                serde_json::json!({
                    "choices": [{"message": {"content": revised_prose}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 140, "completion_tokens": 24, "total_tokens": 164}
                })
                .to_string(),
            ),
            (
                200,
                serde_json::json!({
                    "choices": [{"message": {"content": make_judge_json(pass1_pref).to_string()}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                })
                .to_string(),
            ),
            (
                200,
                serde_json::json!({
                    "choices": [{"message": {"content": make_judge_json(pass2_pref).to_string()}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                })
                .to_string(),
            ),
        ];

        let (loop_url, _loop_reqs, loop_srv) = spawn_http_capture_server(four_paid_replies);
        let updated_cfg = cfg_text.replace(&draft_url, &loop_url);
        std::fs::write(&cfg_path, updated_cfg).unwrap();
        router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

        let draft_path = temp.path().join("crash_recovery_draft.artifact.json");
        std::fs::write(&draft_path, &initial_draft_bytes).unwrap();
        let loop_path = temp.path().join("crash_recovery_loop.artifact.json");

        let critic_budget = CriticBudget::scene_default("venice-model".into(), 20_000);
        let revision_budget = RevisionBudget::scene_default("venice-model".into(), 25_000);
        let judge_budget = JudgeBudget::scene_default("venice-model".into(), 20_000);

        // Crash sequentially at EVERY boundary on the same loop_path!
        // Because each boundary persists its StageTransitionRecord and state before crashing,
        // resuming after each crash never replays any already-completed HTTP model call!
        let boundaries = [
            BoundaryCrashPoint::Stage(EditorialStage::Drafted, 1),
            BoundaryCrashPoint::Stage(EditorialStage::DeterministicChecked, 1),
            BoundaryCrashPoint::Stage(EditorialStage::Critiqued, 1),
            BoundaryCrashPoint::Stage(EditorialStage::CandidateReady, 1),
            BoundaryCrashPoint::Stage(EditorialStage::CandidateChecked, 1),
            BoundaryCrashPoint::JudgePass1(1),
            BoundaryCrashPoint::Stage(EditorialStage::PairwiseJudged, 1),
        ];

        for crash_point in boundaries {
            let err = service
                .revise_artifact_loop_with_gate(
                    EditorialPolicy::CritiqueV1,
                    &draft_path,
                    &loop_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions {
                        crash_after: Some(crash_point),
                        ..Default::default()
                    },
                    Some(gate.as_ref()),
                )
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("simulated_crash_at_boundary"),
                "unexpected error at {crash_point:?}: {err}"
            );
            // At every crash boundary prior to final promotion, the incumbent draft on disk is untouched!
            assert_eq!(
                std::fs::read(&draft_path).unwrap(),
                initial_draft_bytes,
                "incumbent artifact mutated prematurely at {crash_point:?}"
            );
        }

        // Crash immediately after saving the promoted draft artifact, before persisting the
        // terminal AcceptedRevision loop snapshot (two-file commit boundary).
        let mid_commit_err = service
            .revise_artifact_loop_with_gate(
                EditorialPolicy::CritiqueV1,
                &draft_path,
                &loop_path,
                vec![],
                &critic_budget,
                &revision_budget,
                &judge_budget,
                Some(&VerifiedFixtureTokenizer),
                RevisionLoopOptions {
                    crash_after: Some(BoundaryCrashPoint::AfterDraftPromotionSave),
                    ..Default::default()
                },
                Some(gate.as_ref()),
            )
            .await
            .unwrap_err();
        assert!(
            mid_commit_err
                .to_string()
                .contains("simulated_crash_at_boundary:after_draft_promotion_save"),
            "unexpected error at AfterDraftPromotionSave: {mid_commit_err}"
        );

        // At this point, the draft on disk is already promoted with extraction invalidated,
        // while the loop artifact on disk is still non-terminal with pending_promotion recorded.
        let mid_commit_draft = crate::drafting::artifact::load_artifact(&draft_path).unwrap();
        assert_eq!(mid_commit_draft.text, revised_prose);
        assert_eq!(mid_commit_draft.text_sha256, rev_sha);
        assert_eq!(
            mid_commit_draft.accepted_text_sha256.as_deref(),
            Some(rev_sha.as_str())
        );
        assert!(mid_commit_draft.extraction.is_none());
        assert!(mid_commit_draft.extraction_request.is_none());
        assert_eq!(mid_commit_draft.extraction_attempts, 0);
        assert!(mid_commit_draft.extraction_failure.is_none());

        let mid_commit_loop =
            crate::drafting::revision::load_revision_loop_artifact(&loop_path).unwrap();
        assert_eq!(
            mid_commit_loop.current_stage,
            EditorialStage::PairwiseJudged
        );
        assert!(mid_commit_loop.pending_promotion.is_some());

        // Now resume with no crash hook: it recognizes the completed promotion, does not false-positive
        // CAS fail against the already-revised draft, and finalizes AcceptedRevision with 0 extra HTTP calls!
        let final_loop = service
            .revise_artifact_loop_with_gate(
                EditorialPolicy::CritiqueV1,
                &draft_path,
                &loop_path,
                vec![],
                &critic_budget,
                &revision_budget,
                &judge_budget,
                Some(&VerifiedFixtureTokenizer),
                RevisionLoopOptions::default(),
                Some(gate.as_ref()),
            )
            .await
            .unwrap();

        assert_eq!(final_loop.current_stage, EditorialStage::AcceptedRevision);
        assert!(final_loop.revision_promoted);
        assert_eq!(final_loop.accepted_text, revised_prose);
        assert_eq!(final_loop.accepted_text_sha256, rev_sha);
        assert_eq!(final_loop.total_calls_used, 4);
        assert_eq!(final_loop.retention_reason, None);
        assert!(final_loop.pending_promotion.is_none());

        // Verify the draft artifact on disk was promoted to the revised text, had its old
        // extraction state cleared, and can now be extracted for the newly accepted hash.
        let promoted_draft = crate::drafting::artifact::load_artifact(&draft_path).unwrap();
        assert_eq!(promoted_draft.text, revised_prose);
        assert_eq!(
            promoted_draft.accepted_text_sha256.as_deref(),
            Some(rev_sha.as_str())
        );
        assert!(promoted_draft.extraction.is_none());
        assert!(promoted_draft.extraction_request.is_none());
        assert_eq!(promoted_draft.extraction_attempts, 0);
        assert!(promoted_draft.extraction_failure.is_none());

        let re_extracted = crate::drafting::artifact::extract_accepted(
            &draft_path,
            &StubExtractor,
            vec![],
            vec![],
        )
        .await
        .unwrap();
        assert_eq!(re_extracted.extraction_attempts, 1);
        assert_eq!(
            re_extracted
                .extraction
                .as_ref()
                .unwrap()
                .accepted_text_sha256,
            rev_sha
        );
        assert_eq!(
            re_extracted
                .extraction_request
                .as_ref()
                .unwrap()
                .accepted
                .text_sha256,
            rev_sha
        );

        // Re-running after terminal state is a pure idempotent no-op
        let idempotent_rerun = service
            .revise_artifact_loop_with_gate(
                EditorialPolicy::CritiqueV1,
                &draft_path,
                &loop_path,
                vec![],
                &critic_budget,
                &revision_budget,
                &judge_budget,
                Some(&VerifiedFixtureTokenizer),
                RevisionLoopOptions::default(),
                Some(gate.as_ref()),
            )
            .await
            .unwrap();
        assert_eq!(idempotent_rerun, final_loop);

        // The crash at JudgePass1 leaves pass 1 persisted; the resume must
        // claim the distinct reserved pass 2 rather than replaying pass 1.
        let admitted = gate.admitted.lock().unwrap().clone();
        assert!(
            admitted
                .iter()
                .any(|(action, _, claim)| action == "judge" && claim == "judge_pass_2"),
            "resume never admitted judge_pass_2: {admitted:?}"
        );
        assert!(
            admitted
                .iter()
                .filter(|(action, _, claim)| action == "judge" && claim == "judge_pass_1")
                .count()
                == 1,
            "judge_pass_1 was re-dispatched: {admitted:?}"
        );
        // Exactly 4 HTTP requests were made across all 8 crash boundaries + resume + idempotent rerun!
        assert_eq!(loop_srv.join().unwrap(), 4);
    }

    #[tokio::test]
    async fn concurrent_resume_cannot_duplicate_judge_pass_two() {
        use crate::drafting::critique::CriticBudget;
        use crate::drafting::revision::{JudgeBudget, RevisionBudget, RevisionLoopOptions};
        use crate::drafting::tests::{VerifiedFixtureTokenizer, budget, fixture};
        use crate::sqlite::SqliteSpindleService;
        use spindle_core::drafting::EditorialPolicy;
        use std::sync::Arc;

        let temp = tempfile::tempdir().unwrap();
        let gate = Arc::new(RecordingRevisionGate::default());
        let draft_prose = "Cold floodwater rose around the stone sluice.";
        let revised_prose = "Cold floodwater surged against the stone sluice.";

        let draft_reply = serde_json::json!({
            "choices": [{"message": {"content": draft_prose}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 18, "total_tokens": 118}
        })
        .to_string();
        let (draft_url, _, draft_srv) = spawn_http_capture_server(vec![(200, draft_reply)]);

        let pool = crate::sqlite::SqlitePool::open(&temp.path().join("spindle.db"))
            .await
            .unwrap();
        let router = ModelRouter::local_only();
        let cfg_path = temp.path().join("spindle.toml");
        let cfg_text = format!(
            "[health_check]\nenabled = false\n[[agents]]\nid = \"venice\"\nname = \"Venice\"\nprovider = \"venice\"\nendpoint = \"{draft_url}\"\nmodel = \"venice-model\"\nratings = [\"general\"]\n[[routing]]\nroute = \"draft\"\nagent = \"venice\"\nmax_tokens = 300\n[[routing]]\nroute = \"review\"\nagent = \"venice\"\nmax_tokens = 500\n"
        );
        std::fs::write(&cfg_path, &cfg_text).unwrap();
        router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

        let repository = crate::sqlite::repository::Repository::with_model_router(
            pool.clone(),
            temp.path().join("data"),
            router.clone(),
        );
        let service = SqliteSpindleService::new(repository.clone());
        let created = service
            .create_project(spindle_core::models::CreateProjectInput {
                name: "Overlap Judge Test".into(),
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
        let active_branch = repository
            .get_active_branch(&created.project_id)
            .await
            .unwrap();
        let mut pkt = fixture();
        pkt.project_id = created.project_id.clone();
        pkt.branch_id = active_branch.id.clone();
        for src in &mut pkt.source_versions {
            src.branch_id = active_branch.id.clone();
        }
        pkt.scene_card.source.branch_id = active_branch.id.clone();
        pkt.scene_card.pov.source.branch_id = active_branch.id.clone();
        for c in &mut pkt.hard_constraints {
            c.source.branch_id = active_branch.id.clone();
        }
        let mut draft_budget = budget();
        draft_budget.model_id = "venice-model".into();
        let draft_path = temp.path().join("overlap_draft.artifact.json");
        let draft_art = service
            .draft_packet_to_artifact(
                EditorialPolicy::CritiqueV1,
                pkt.clone(),
                &draft_budget,
                Some(&VerifiedFixtureTokenizer),
                &draft_path,
            )
            .await
            .unwrap();

        let critique_json = make_valid_critique_json(
            &draft_art.packet_hash,
            &draft_art.text_sha256,
            None,
            &draft_art.text,
        );
        let judge_json = make_judge_json("tie");
        let responses = vec![
            (
                200,
                serde_json::json!({"choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 120, "completion_tokens": 40, "total_tokens": 160}}).to_string(),
            ),
            (
                200,
                serde_json::json!({"choices": [{"message": {"content": revised_prose}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 100, "completion_tokens": 12, "total_tokens": 112}}).to_string(),
            ),
            (
                200,
                serde_json::json!({"choices": [{"message": {"content": judge_json.to_string()}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}}).to_string(),
            ),
            (
                200,
                serde_json::json!({"choices": [{"message": {"content": judge_json.to_string()}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}}).to_string(),
            ),
        ];
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (loop_url, loop_srv) = spawn_blocking_pass_two_server(responses, release_rx);
        let loop_cfg = cfg_text.replace(&draft_url, &loop_url);
        std::fs::write(&cfg_path, loop_cfg).unwrap();
        router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

        let loop_path = temp.path().join("overlap_loop.artifact.json");
        let critic_budget = CriticBudget::scene_default("venice-model".into(), 20_000);
        let revision_budget = RevisionBudget::scene_default("venice-model".into(), 25_000);
        let judge_budget = JudgeBudget::scene_default("venice-model".into(), 20_000);
        let options = RevisionLoopOptions::default();
        let pass_2_wait = gate.pass_2_claim.notified();
        let first = service.revise_artifact_loop_with_gate(
            EditorialPolicy::CritiqueV1,
            &draft_path,
            &loop_path,
            vec![],
            &critic_budget,
            &revision_budget,
            &judge_budget,
            Some(&VerifiedFixtureTokenizer),
            options.clone(),
            Some(gate.as_ref()),
        );
        let second = async {
            pass_2_wait.await;
            let error = service
                .revise_artifact_loop_with_gate(
                    EditorialPolicy::CritiqueV1,
                    &draft_path,
                    &loop_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    options.clone(),
                    Some(gate.as_ref()),
                )
                .await
                .expect_err("a concurrent resume must not dispatch pass 2 again");
            release_tx.send(()).expect("release pass 2");
            error
        };
        let (first_result, second_error) = tokio::join!(first, second);
        assert!(
            second_error
                .to_string()
                .contains("duplicate dispatch claim"),
            "unexpected concurrent resume error: {second_error}"
        );
        first_result.expect("the owning run completes after release");
        assert_eq!(loop_srv.join().unwrap(), 4, "no duplicate pass-2 request");
        assert_eq!(draft_srv.join().unwrap(), 1);
    }

    #[tokio::test]
    async fn revision_loop_retains_incumbent_on_tie_error_new_blocker_fingerprint_and_cas_conflict()
    {
        use crate::drafting::critique::CriticBudget;
        use crate::drafting::revision::{
            BoundaryCrashPoint, JudgeBudget, RevisionBudget, RevisionLoopOptions,
            select_initial_judge_order,
        };
        use crate::drafting::tests::{VerifiedFixtureTokenizer, budget, fixture};
        use crate::sqlite::SqliteSpindleService;
        use spindle_core::drafting::{
            DeblindedArm, EditorialPolicy, EditorialStage, RevisionRetentionReason,
        };

        let temp = tempfile::tempdir().unwrap();
        let draft_prose = "Cold floodwater rose around the stone sluice.\n\nThe lantern flickered in the damp wind.";
        let revised_clean = "Cold floodwater surged against the stone sluice as Mara braced her shoulder.\n\nThe lantern flickered in the damp wind.";
        let revised_with_slop_blocker = "Cold floodwater rose.\n\nShe felt a mix of relief and dread as the gate moved. She didn't know what tomorrow would bring.";

        let draft_reply = serde_json::json!({
            "choices": [{"message": {"content": draft_prose}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 18, "total_tokens": 118}
        })
        .to_string();
        let (draft_url, _, draft_srv) = spawn_http_capture_server(vec![(200, draft_reply)]);

        let pool = crate::sqlite::SqlitePool::open(&temp.path().join("spindle.db"))
            .await
            .unwrap();
        let router = ModelRouter::local_only();
        let cfg_path = temp.path().join("spindle.toml");
        let cfg_text = format!(
            "[health_check]\nenabled = false\n[[agents]]\nid = \"venice\"\nname = \"Venice\"\nprovider = \"venice\"\nendpoint = \"{draft_url}\"\nmodel = \"venice-model\"\nratings = [\"general\"]\n[[routing]]\nroute = \"draft\"\nagent = \"venice\"\nmax_tokens = 300\n[[routing]]\nroute = \"review\"\nagent = \"venice\"\nmax_tokens = 500\n"
        );
        std::fs::write(&cfg_path, &cfg_text).unwrap();
        router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

        let repository = crate::sqlite::repository::Repository::with_model_router(
            pool.clone(),
            temp.path().join("data"),
            router.clone(),
        );
        let service = SqliteSpindleService::new(repository.clone());
        let created = service
            .create_project(spindle_core::models::CreateProjectInput {
                name: "Retention Cases Test".into(),
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
        let active_branch = repository
            .get_active_branch(&created.project_id)
            .await
            .unwrap();

        let mut pkt = fixture();
        pkt.project_id = created.project_id.clone();
        pkt.branch_id = active_branch.id.clone();
        for src in &mut pkt.source_versions {
            src.branch_id = active_branch.id.clone();
        }
        pkt.scene_card.source.branch_id = active_branch.id.clone();
        pkt.scene_card.pov.source.branch_id = active_branch.id.clone();
        for c in &mut pkt.hard_constraints {
            c.source.branch_id = active_branch.id.clone();
        }

        let mut draft_budget = budget();
        draft_budget.model_id = "venice-model".into();
        let base_draft_path = temp.path().join("base.artifact.json");
        let draft_art = service
            .draft_packet_to_artifact(
                EditorialPolicy::CritiqueV1,
                pkt,
                &draft_budget,
                Some(&VerifiedFixtureTokenizer),
                &base_draft_path,
            )
            .await
            .unwrap();
        assert_eq!(draft_srv.join().unwrap(), 1);
        let base_draft_bytes = std::fs::read(&base_draft_path).unwrap();

        let critique_json = make_valid_critique_json(
            &draft_art.packet_hash,
            &draft_art.text_sha256,
            draft_art.scene_id.as_deref(),
            &draft_art.text,
        );
        let critic_budget = CriticBudget::scene_default("venice-model".into(), 20_000);
        let revision_budget = RevisionBudget::scene_default("venice-model".into(), 25_000);
        let judge_budget = JudgeBudget::scene_default("venice-model".into(), 20_000);

        // Case 1: New deterministic hard anti-slop blocker disqualifies candidate BEFORE pairwise judging (only 2 calls: critic + revision)
        {
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": revised_with_slop_blocker}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 25, "total_tokens": 165}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case1_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case1_loop.json");
            let res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions::default(),
                )
                .await
                .unwrap();
            assert_eq!(res.current_stage, EditorialStage::AcceptedOriginal);
            assert!(!res.revision_promoted);
            assert_eq!(
                res.retention_reason,
                Some(RevisionRetentionReason::CandidateDisqualifiedByDeterministicCheck)
            );
            assert_eq!(res.total_calls_used, 2);
            assert_eq!(res.accepted_text, draft_prose);
            assert_eq!(srv.join().unwrap(), 2);
        }

        // Case 2: Swapped-order position bias (both passes pick passage_1) resolves to Tie and retains incumbent
        {
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": revised_clean}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 24, "total_tokens": 164}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json("passage_1").to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json("passage_1").to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case2_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case2_loop.json");
            let res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions::default(),
                )
                .await
                .unwrap();
            assert_eq!(res.current_stage, EditorialStage::AcceptedOriginal);
            assert!(!res.revision_promoted);
            assert_eq!(
                res.retention_reason,
                Some(RevisionRetentionReason::JudgeOrderDisagreementTie)
            );
            assert_eq!(res.accepted_text, draft_prose);
            assert_eq!(srv.join().unwrap(), 4);
        }

        // Case 3: Invalid judge JSON output retains incumbent and marks parked_for_review
        {
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": revised_clean}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 24, "total_tokens": 164}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": "not valid judge json"}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 10, "total_tokens": 160}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case3_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case3_loop.json");
            let res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions::default(),
                )
                .await
                .unwrap();
            assert_eq!(res.current_stage, EditorialStage::AcceptedOriginal);
            assert!(!res.revision_promoted);
            assert!(res.parked_for_review);
            assert_eq!(
                res.retention_reason,
                Some(RevisionRetentionReason::JudgeInvalidOrUnavailable)
            );
            assert_eq!(res.accepted_text, draft_prose);
            assert_eq!(srv.join().unwrap(), 3);
        }

        // Case 4: Identical candidate hash stops before judging (only 2 calls)
        {
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": draft_prose}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 18, "total_tokens": 158}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case4_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case4_loop.json");
            let res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions::default(),
                )
                .await
                .unwrap();
            assert_eq!(res.current_stage, EditorialStage::AcceptedOriginal);
            assert!(!res.revision_promoted);
            assert_eq!(
                res.retention_reason,
                Some(RevisionRetentionReason::IdenticalCandidateHash)
            );
            assert_eq!(res.total_calls_used, 2);
            assert_eq!(srv.join().unwrap(), 2);
        }

        // Case 5: Compare-and-Swap (CAS) conflict when incumbent artifact on disk is modified before promotion
        {
            let rev_sha = {
                use sha2::{Digest, Sha256};
                format!("{:x}", Sha256::digest(revised_clean.as_bytes()))
            };
            let (p1_arm, _) = select_initial_judge_order(
                &draft_art.packet_hash,
                &draft_art.text_sha256,
                &rev_sha,
                1,
                None,
            );
            let pass1_pref = if p1_arm == DeblindedArm::Candidate {
                "passage_1"
            } else {
                "passage_2"
            };
            let pass2_pref = if p1_arm == DeblindedArm::Candidate {
                "passage_2"
            } else {
                "passage_1"
            };
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": revised_clean}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 24, "total_tokens": 164}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json(pass1_pref).to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json(pass2_pref).to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case5_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case5_loop.json");

            // Pause at PairwiseJudged just before CAS promotion
            let _ = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions {
                        crash_after: Some(BoundaryCrashPoint::Stage(
                            EditorialStage::PairwiseJudged,
                            1,
                        )),
                        ..Default::default()
                    },
                )
                .await
                .unwrap_err();

            // Simulate a concurrent edit to the packet's plan_revision on disk before promotion!
            let mut concurrent_draft = crate::drafting::artifact::load_artifact(&d_path).unwrap();
            concurrent_draft
                .prepared_packet
                .as_mut()
                .unwrap()
                .packet
                .plan_revision = "concurrent-plan-rev-99".into();
            crate::drafting::artifact::save_artifact(&d_path, &concurrent_draft).unwrap();
            let concurrent_bytes = std::fs::read(&d_path).unwrap();

            let cas_res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions::default(),
                )
                .await
                .unwrap();

            assert_eq!(cas_res.current_stage, EditorialStage::AcceptedOriginal);
            assert!(!cas_res.revision_promoted);
            assert_eq!(
                cas_res.retention_reason,
                Some(RevisionRetentionReason::StaleCasConflict)
            );
            // Concurrent draft artifact on disk was NOT overwritten!
            assert_eq!(std::fs::read(&d_path).unwrap(), concurrent_bytes);
            assert_eq!(srv.join().unwrap(), 4);
        }

        // Case 6: Unresolved external finding with unchanged finding fingerprint stops with NoDirectiveProgress before judging (2 calls)
        {
            use spindle_core::drafting::{
                CriticDimension, CriticFindingInput, DirectiveSeverity, FeedbackSourceKind,
            };
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": revised_clean}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 24, "total_tokens": 164}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case6_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case6_loop.json");
            let unresolved_finding = CriticFindingInput {
                finding_id: "ext-1".into(),
                source: FeedbackSourceKind::Continuity,
                check_type: "timeline_drift".into(),
                dimension: CriticDimension::Continuity,
                severity: DirectiveSeverity::Major,
                message: "Lantern was left behind in chapter 1".into(),
                anchor: None,
                suggested_fix: Some("Remove lantern reference".into()),
            };
            let res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![unresolved_finding],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions::default(),
                )
                .await
                .unwrap();
            assert_eq!(res.current_stage, EditorialStage::AcceptedOriginal);
            assert!(!res.revision_promoted);
            assert_eq!(
                res.retention_reason,
                Some(RevisionRetentionReason::NoDirectiveProgress)
            );
            assert_eq!(res.total_calls_used, 2);
            assert_eq!(srv.join().unwrap(), 2);
        }

        // Case 7: Scene call budget exhaustion (max_scene_calls = 2) stops before pairwise judging with BudgetExhausted
        {
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": revised_clean}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 24, "total_tokens": 164}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case7_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case7_loop.json");
            let res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions {
                        max_scene_calls: 2,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(res.current_stage, EditorialStage::AcceptedOriginal);
            assert!(!res.revision_promoted);
            assert_eq!(
                res.retention_reason,
                Some(RevisionRetentionReason::BudgetExhausted)
            );
            assert_eq!(res.total_calls_used, 2);
            assert_eq!(srv.join().unwrap(), 2);
        }

        // Case 8: Anchored external finding whose exact_quote is rewritten by the revision is projected without stale_anchor_hash and increments resolved_finding_count
        {
            use spindle_core::drafting::{
                CriticDimension, CriticFindingInput, CritiqueAnchor, DirectiveSeverity,
                FeedbackSourceKind,
            };
            let rev_sha = {
                use sha2::{Digest, Sha256};
                format!("{:x}", Sha256::digest(revised_clean.as_bytes()))
            };
            let (p1_arm, _p2_arm) = select_initial_judge_order(
                &draft_art.packet_hash,
                &draft_art.text_sha256,
                &rev_sha,
                1,
                None,
            );
            let pass1_pref = if p1_arm == DeblindedArm::Candidate {
                "passage_1"
            } else {
                "passage_2"
            };
            let pass2_pref = if p1_arm == DeblindedArm::Candidate {
                "passage_2"
            } else {
                "passage_1"
            };
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": revised_clean}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 24, "total_tokens": 164}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json(pass1_pref).to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 180, "completion_tokens": 30, "total_tokens": 210}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json(pass2_pref).to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 180, "completion_tokens": 30, "total_tokens": 210}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case8_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case8_loop.json");
            let quote = "rose around the stone sluice.";
            let quote_start = draft_prose.find(quote).unwrap();
            let quote_end = quote_start + quote.len();
            let anchored_finding = CriticFindingInput {
                finding_id: "ext-anchored-1".into(),
                source: FeedbackSourceKind::AntiSlop,
                check_type: "weak_verb".into(),
                dimension: CriticDimension::Freshness,
                severity: DirectiveSeverity::Minor,
                message: "Sharpen physical action around the sluice".into(),
                anchor: Some(CritiqueAnchor {
                    scene_id: draft_art.scene_id.clone(),
                    base_text_hash: draft_art.text_sha256.clone(),
                    start_byte: quote_start,
                    end_byte: quote_end,
                    exact_quote: quote.to_string(),
                    context_before: Some("Cold floodwater".into()),
                    context_after: Some("The lantern".into()),
                }),
                suggested_fix: Some("Show physical tension against the sluice".into()),
            };
            let res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![anchored_finding],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions::default(),
                )
                .await
                .unwrap();
            assert_eq!(res.current_stage, EditorialStage::AcceptedRevision);
            assert!(res.revision_promoted);
            let cmp = res.rounds[0]
                .candidate_check_comparison
                .as_ref()
                .expect("comparison recorded");
            assert_eq!(cmp.resolved_finding_count, 1);
            assert_eq!(srv.join().unwrap(), 4);
        }

        // Case 9: Live repository canon mutation (create_world_rule) while leaving the artifact file on disk untouched triggers StaleCasConflict
        {
            let rev_sha = {
                use sha2::{Digest, Sha256};
                format!("{:x}", Sha256::digest(revised_clean.as_bytes()))
            };
            let (p1_arm, _) = select_initial_judge_order(
                &draft_art.packet_hash,
                &draft_art.text_sha256,
                &rev_sha,
                1,
                None,
            );
            let pass1_pref = if p1_arm == DeblindedArm::Candidate {
                "passage_1"
            } else {
                "passage_2"
            };
            let pass2_pref = if p1_arm == DeblindedArm::Candidate {
                "passage_2"
            } else {
                "passage_1"
            };
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": revised_clean}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 24, "total_tokens": 164}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json(pass1_pref).to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json(pass2_pref).to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case9_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case9_loop.json");

            let _ = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions {
                        crash_after: Some(BoundaryCrashPoint::Stage(
                            EditorialStage::PairwiseJudged,
                            1,
                        )),
                        ..Default::default()
                    },
                )
                .await
                .unwrap_err();

            // Mutate canon in the SQLite repository while leaving d_path completely untouched on disk!
            repository
                .create_world_rule(&spindle_core::models::CreateWorldRuleInput {
                    project_id: created.project_id.clone(),
                    rule_name: "Sluice Gate Hydraulics".into(),
                    rule_type: "physics".into(),
                    description: "The stone sluice requires two operators to lift.".into(),
                    established_in: None,
                    relevance_tags: vec!["sluice".into()],
                    scan_pattern: None,
                })
                .await
                .unwrap();
            assert_eq!(std::fs::read(&d_path).unwrap(), base_draft_bytes);

            let cas_res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions::default(),
                )
                .await
                .unwrap();

            assert_eq!(cas_res.current_stage, EditorialStage::AcceptedOriginal);
            assert!(!cas_res.revision_promoted);
            assert_eq!(
                cas_res.retention_reason,
                Some(RevisionRetentionReason::StaleCasConflict)
            );
            assert_eq!(cas_res.accepted_text, draft_prose);
            assert_eq!(std::fs::read(&d_path).unwrap(), base_draft_bytes);
            assert_eq!(srv.join().unwrap(), 4);
        }

        // Case 10: Live repository chapter plan mutation (plan_chapter) while leaving the artifact file on disk untouched triggers StaleCasConflict
        {
            let rev_sha = {
                use sha2::{Digest, Sha256};
                format!("{:x}", Sha256::digest(revised_clean.as_bytes()))
            };
            let (p1_arm, _) = select_initial_judge_order(
                &draft_art.packet_hash,
                &draft_art.text_sha256,
                &rev_sha,
                1,
                None,
            );
            let pass1_pref = if p1_arm == DeblindedArm::Candidate {
                "passage_1"
            } else {
                "passage_2"
            };
            let pass2_pref = if p1_arm == DeblindedArm::Candidate {
                "passage_2"
            } else {
                "passage_1"
            };
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": revised_clean}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 24, "total_tokens": 164}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json(pass1_pref).to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json(pass2_pref).to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case10_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case10_loop.json");

            let _ = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions {
                        crash_after: Some(BoundaryCrashPoint::Stage(
                            EditorialStage::PairwiseJudged,
                            1,
                        )),
                        ..Default::default()
                    },
                )
                .await
                .unwrap_err();

            // Mutate chapter plan in the SQLite repository while leaving d_path completely untouched on disk!
            repository
                .plan_chapter(&spindle_core::models::PlanChapterInput {
                    project_id: created.project_id.clone(),
                    book_number: 1,
                    chapter_number: 1,
                    pov_character_id: None,
                    synopsis: "Updated chapter plan synopsis during model calls".into(),
                    target_theme_ids: vec![],
                    target_conflict_ids: vec![],
                    target_plot_line_ids: vec![],
                    scenes: vec![spindle_core::models::PlanChapterSceneInput {
                        scene_order: 1,
                        summary: "Mara crosses the flooded sluice".into(),
                        beat_structure: vec![],
                        character_ids: vec![],
                        location_id: None,
                        content_rating: None,
                        purpose: "Establish flood stakes".into(),
                        research_required: Some(false),
                        research_tags: vec![],
                        explicit_query: None,
                    }],
                })
                .await
                .unwrap();
            assert_eq!(std::fs::read(&d_path).unwrap(), base_draft_bytes);

            let cas_res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions::default(),
                )
                .await
                .unwrap();

            assert_eq!(cas_res.current_stage, EditorialStage::AcceptedOriginal);
            assert!(!cas_res.revision_promoted);
            assert_eq!(
                cas_res.retention_reason,
                Some(RevisionRetentionReason::StaleCasConflict)
            );
            assert_eq!(cas_res.accepted_text, draft_prose);
            assert_eq!(std::fs::read(&d_path).unwrap(), base_draft_bytes);
            assert_eq!(srv.join().unwrap(), 4);
        }

        // Case 11: Active authoring-run scene handoff plan-revision mismatch (P3 #54 composition)
        // rejects promotion with StaleCasConflict even if the draft artifact file on disk is untouched.
        {
            let rev_sha = {
                use sha2::{Digest, Sha256};
                format!("{:x}", Sha256::digest(revised_clean.as_bytes()))
            };
            let (p1_arm, _) = select_initial_judge_order(
                &draft_art.packet_hash,
                &draft_art.text_sha256,
                &rev_sha,
                1,
                None,
            );
            let pass1_pref = if p1_arm == DeblindedArm::Candidate {
                "passage_1"
            } else {
                "passage_2"
            };
            let pass2_pref = if p1_arm == DeblindedArm::Candidate {
                "passage_2"
            } else {
                "passage_1"
            };
            let replies = vec![
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": critique_json.to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 120, "completion_tokens": 80, "total_tokens": 200}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": revised_clean}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 140, "completion_tokens": 24, "total_tokens": 164}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json(pass1_pref).to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                    })
                    .to_string(),
                ),
                (
                    200,
                    serde_json::json!({
                        "choices": [{"message": {"content": make_judge_json(pass2_pref).to_string()}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 150, "completion_tokens": 60, "total_tokens": 210}
                    })
                    .to_string(),
                ),
            ];
            let (url, _, srv) = spawn_http_capture_server(replies);
            std::fs::write(&cfg_path, cfg_text.replace(&draft_url, &url)).unwrap();
            router.configure(Some(cfg_path.to_str().unwrap())).unwrap();

            let d_path = temp.path().join("case11_draft.json");
            std::fs::write(&d_path, &base_draft_bytes).unwrap();
            let l_path = temp.path().join("case11_loop.json");

            let _ = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions {
                        crash_after: Some(BoundaryCrashPoint::Stage(
                            EditorialStage::PairwiseJudged,
                            1,
                        )),
                        ..Default::default()
                    },
                )
                .await
                .unwrap_err();

            // Record an active authoring_run + authoring_run_scene with a superseded handoff
            // plan revision for chapter 1 scene 1 while leaving d_path untouched on disk.
            let pid = created.project_id.clone();
            let bid = active_branch.id.clone();
            repository
                .pool()
                .write(move |conn| {
                    let now = chrono::Utc::now().timestamp_micros();
                    conn.execute(
                        "INSERT INTO authoring_run (
                            id, project_id, active_branch_id, book_number, start_chapter, end_chapter,
                            checkpoint_interval, last_checkpoint_end_chapter, artifacts_dir,
                            editorial_directives, status, created_at, updated_at, mining_policy,
                            max_revise_attempts, checkpoint_policy, replan_policy
                        ) VALUES ('run_handoff_stale', ?1, ?2, 1, 1, 1, 1, 0, 'runs/stale', '[]', 'active', ?3, ?3, 'auto', 2, 'manual', 'manual')",
                        rusqlite::params![&pid, &bid, now],
                    )?;
                    conn.execute(
                        "INSERT INTO authoring_run_chapter (
                            authoring_run_id, chapter_number, planned, synopsis, pov_character_id,
                            status, summary_saved, summary_artifact_path
                        ) VALUES ('run_handoff_stale', 1, 1, 'Flooded sluice crossing', NULL, 'in_progress', 0, NULL)",
                        [],
                    )?;
                    conn.execute(
                        "INSERT INTO authoring_run_scene (
                            authoring_run_id, chapter_number, scene_order, character_ids, location_id,
                            content_rating, tone, source_path, phase, scene_id, scene_artifact_path,
                            draft_diagnostics, blocked_reason, research_required, research_tags,
                            explicit_query, mine_status, mine_detail, verify_status, verify_detail,
                            revise_attempts, last_finding_fingerprint
                        ) VALUES ('run_handoff_stale', 1, 1, '[]', 'loc:sluice', 'mature', NULL, NULL, 'pending', NULL, 'scenes/ch01_s01.md', '{\"superseded_handoff_plan_revision\":1}', NULL, 0, '[]', NULL, NULL, NULL, NULL, NULL, 0, NULL)",
                        [],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
            assert_eq!(std::fs::read(&d_path).unwrap(), base_draft_bytes);

            let cas_res = service
                .revise_artifact_loop(
                    EditorialPolicy::CritiqueV1,
                    &d_path,
                    &l_path,
                    vec![],
                    &critic_budget,
                    &revision_budget,
                    &judge_budget,
                    Some(&VerifiedFixtureTokenizer),
                    RevisionLoopOptions::default(),
                )
                .await
                .unwrap();

            assert_eq!(cas_res.current_stage, EditorialStage::AcceptedOriginal);
            assert!(!cas_res.revision_promoted);
            assert_eq!(
                cas_res.retention_reason,
                Some(RevisionRetentionReason::StaleCasConflict)
            );
            assert_eq!(cas_res.accepted_text, draft_prose);
            assert_eq!(std::fs::read(&d_path).unwrap(), base_draft_bytes);
            assert_eq!(srv.join().unwrap(), 4);
        }
    }
}
