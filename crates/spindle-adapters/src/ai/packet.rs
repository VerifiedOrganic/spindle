use super::*;
use crate::drafting::{
    DraftTokenizer, PacketBudget, PreparedDraftingPacket, assemble_packet_with_system,
};
use spindle_core::drafting::{DraftingPacketV1, EditorialPolicy};

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
}
