use super::*;
use crate::drafting::DraftMessages;
use crate::tokenization::{
    CalibratedTokenizer, CalibrationCertificate, PinnedTokenizer, ProbeReceipt, TokenizerConfig,
    calibration_messages, messages_hash, sha256,
};
use anyhow::ensure;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone)]
pub(super) struct CounterScope {
    agent: AgentRuntime,
    route: ModelRoute,
    config: TokenizerConfig,
    endpoint: String,
    model: String,
    pub(super) max_context: usize,
    client: reqwest::Client,
    identity: String,
}
#[derive(Serialize, Deserialize)]
struct CachedCount {
    identity: String,
    receipt: ProbeReceipt,
}

impl ModelRouter {
    /// Operator-only calibration/counting utility: no health-check network work.
    pub fn for_tokenizer_tools(
        config_path: &str,
        usage_pool: Option<crate::sqlite::SqlitePool>,
    ) -> anyhow::Result<Self> {
        let mut loaded = crate::agent_config::load_agent_config(Some(config_path))?;
        loaded.health_check.enabled = Some(false);
        let router = Self::from_loaded_config(loaded);
        Ok(if let Some(pool) = usage_pool {
            router.with_usage_pool(pool)
        } else {
            router
        })
    }
    pub(super) fn counter_scope(
        &self,
        runtime: &RuntimeConfig,
        route: &ModelRoute,
    ) -> anyhow::Result<CounterScope> {
        ensure!(
            route.adapter_kind == "http",
            "budget_unknown: tokenizer framing unsupported for this adapter"
        );
        let agent = runtime
            .agents
            .get(&route.model_name)
            .context("budget_unknown: configured model agent required")?
            .clone();
        let raw = agent
            .config
            .tokenizer
            .as_ref()
            .context("budget_unknown: configure a pinned tokenizer profile")?;
        let base = runtime
            .source_path
            .as_deref()
            .and_then(|p| Path::new(p).parent())
            .unwrap_or(Path::new("."));
        let config = raw.resolved(base);
        let max_context = agent
            .config
            .max_context
            .context("max_context required for token counter")?;
        let root = agent.config.endpoint.trim_end_matches('/');
        let root = root.strip_suffix("/v1").unwrap_or(root);
        let endpoint = format!("{root}/api/chat");
        let parsed = reqwest::Url::parse(&endpoint)?;
        ensure!(
            matches!(parsed.scheme(), "http" | "https")
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.query().is_none()
                && parsed.fragment().is_none(),
            "count endpoint must not contain credentials/query/fragment"
        );
        let model = agent.config.model.clone();
        let identity = sha256(&serde_json::to_vec(&(
            "ollama-chat-count/v1",
            endpoint.as_str(),
            &model,
            &config.model_revision,
            config.probe_mode,
            max_context,
        ))?);
        Ok(CounterScope {
            agent,
            route: route.clone(),
            config,
            endpoint,
            model,
            max_context,
            identity,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
    pub(super) fn configured_tokenizer(
        &self,
        scope: &CounterScope,
    ) -> anyhow::Result<Arc<CalibratedTokenizer>> {
        let key = sha256(&serde_json::to_vec(&(&scope.identity, &scope.config))?);
        let mut cache = self
            .tokenizer_cache
            .lock()
            .map_err(|_| poisoned_lock("tokenizer cache"))?;
        if let Some(encoder) = cache.get(&key) {
            return Ok(encoder.clone());
        }
        let encoder = Arc::new(PinnedTokenizer::load_verified(
            &scope.model,
            scope.config.clone(),
            &scope.identity,
        )?);
        cache.insert(key, encoder.clone());
        Ok(encoder)
    }
    /// Explicit operator setup. Synthetic prompts only; each miss is attributed
    /// to the existing model_call ledger and stored without generated text.
    pub async fn calibrate_tokenizer(
        &self,
        route_name: &str,
        rating: &str,
    ) -> anyhow::Result<CalibrationCertificate> {
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let route = resolve_cleared_route(&runtime, route_name, Some(rating))?;
        let scope = self.counter_scope(&runtime, route)?;
        let encoder = PinnedTokenizer::load(&scope.model, scope.config.clone())?;
        let mut samples = Vec::new();
        for messages in calibration_messages() {
            samples.push(
                self.probe_count(&scope, &messages, Some(rating), None)
                    .await?,
            );
        }
        let certificate = encoder.certificate(&scope.identity, samples)?;
        write_json_atomic(&scope.config.calibration_file, &certificate)?;
        self.tokenizer_cache
            .lock()
            .map_err(|_| poisoned_lock("tokenizer cache"))?
            .clear();
        Ok(certificate)
    }
    /// No network I/O. A calibration certificate bound to this exact profile
    /// and provider deployment is required even when the caller is offline.
    pub fn count_tokenizer_messages(
        &self,
        route_name: &str,
        rating: &str,
        messages: &DraftMessages,
    ) -> anyhow::Result<usize> {
        use crate::drafting::DraftTokenizer;
        let runtime = self
            .runtime
            .read()
            .map_err(|_| poisoned_lock("model router read lock"))?
            .clone();
        let route = resolve_cleared_route(&runtime, route_name, Some(rating))?;
        let scope = self.counter_scope(&runtime, route)?;
        self.configured_tokenizer(&scope)?
            .count_messages(messages)
            .map_err(anyhow::Error::msg)
    }
    pub(super) async fn probe_count(
        &self,
        scope: &CounterScope,
        messages: &DraftMessages,
        rating: Option<&str>,
        context: Option<&RequestContext>,
    ) -> anyhow::Result<ProbeReceipt> {
        ensure!(
            self.usage_pool.is_some(),
            "count probes require the model-usage database"
        );
        let message_hash = messages_hash(messages)?;
        let key = sha256(&serde_json::to_vec(&(&scope.identity, &message_hash))?);
        let cell = {
            let mut cache = self
                .token_count_cache
                .lock()
                .map_err(|_| poisoned_lock("token count cache"))?;
            cache.entry(key.clone()).or_default().clone()
        };
        let value = cell
            .get_or_try_init(|| async {
                let path = scope.config.cache_dir.join(format!("{key}.json"));
                if path.exists() {
                    let cached: CachedCount = serde_json::from_reader(std::fs::File::open(&path)?)?;
                    ensure!(
                        cached.identity == scope.identity,
                        "count cache identity mismatch"
                    );
                    validate_receipt(scope, &message_hash, &cached.receipt)?;
                    return Ok(cached.receipt);
                }
                let mut lock = crate::drafting::artifact::lock_artifact(&path)?;
                lock.retain_for_recovery(&message_hash)?;
                let body = serde_json::json!({
                    "model": scope.model,
                    "messages": [
                        {"role": "system", "content": messages.system},
                        {"role": "user", "content": messages.user}
                    ],
                    "stream": false,
                    "truncate": false,
                    "options": {
                        "num_predict": scope.config.probe_mode.output_limit(),
                        "temperature": 0,
                        "num_ctx": scope.max_context
                    }
                });
                let start = std::time::Instant::now();
                let sent = with_provider_headers(
                    scope
                        .client
                        .post(&scope.endpoint)
                        .json(&body)
                        .timeout(Duration::from_secs(30)),
                    &scope.agent,
                )
                .send()
                .await;
                let mut parsed = None;
                let mut result = match sent {
                    Ok(response) if response.status().is_success() => {
                        match response.json::<serde_json::Value>().await {
                            Ok(value) => {
                                let usage = spindle_core::models::ModelUsage {
                                    input_tokens: value
                                        .get("prompt_eval_count")
                                        .and_then(serde_json::Value::as_u64),
                                    output_tokens: value
                                        .get("eval_count")
                                        .and_then(serde_json::Value::as_u64),
                                    cached_input_tokens: value
                                        .get("prompt_eval_cached_count")
                                        .and_then(serde_json::Value::as_u64),
                                };
                                parsed = Some(value);
                                Ok(ModelResponse {
                                    adapter_kind: "http".into(),
                                    model_name: scope.model.clone(),
                                    output: String::new(),
                                    usage: Some(usage),
                                    ..Default::default()
                                })
                            }
                            Err(_) => Err(anyhow::anyhow!("invalid count probe response JSON")),
                        }
                    }
                    Ok(response) => Err(anyhow::anyhow!(
                        "count probe HTTP {}",
                        response.status().as_u16()
                    )),
                    Err(_) => Err(anyhow::anyhow!("count probe transport failure")),
                };
                let mut call_context = context.cloned().unwrap_or_default();
                call_context.prompt_template_id = Some("token_count.ollama_probe".into());
                call_context.prompt_version = Some("v1".into());
                call_context.context_packet_hash = Some(message_hash.clone());
                let mut count_route = scope.route.clone();
                count_route.model_name = scope.model.clone();
                count_route.max_tokens = Some(scope.config.probe_mode.output_limit());
                count_route.temperature = Some(0.0);
                count_route.top_p = None;
                self.record_usage(
                    &scope.route.route_name,
                    rating,
                    Some(&call_context),
                    "",
                    false,
                    Some(&count_route),
                    start,
                    &mut result,
                )
                .await;
                let response = result?;
                let value = parsed.context("missing probe response")?;
                ensure!(
                    value.get("done").and_then(serde_json::Value::as_bool) == Some(true),
                    "incomplete count probe"
                );
                ensure!(
                    value.get("model").and_then(serde_json::Value::as_str)
                        == Some(scope.model.as_str()),
                    "count probe model mismatch"
                );
                let input = value
                    .get("prompt_eval_count")
                    .and_then(serde_json::Value::as_u64)
                    .context("count probe missing prompt_eval_count; no estimate permitted")?;
                let output = value
                    .get("eval_count")
                    .and_then(serde_json::Value::as_u64)
                    .context("count probe missing eval_count")?;
                if scope.config.probe_mode.output_limit() == 0 {
                    let generated = |key| {
                        value
                            .get("message")
                            .and_then(|m| m.get(key))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                    };
                    ensure!(
                        generated("content").is_empty() && generated("thinking").is_empty(),
                        "zero-output probe returned generated text"
                    );
                }
                let receipt = ProbeReceipt {
                    model: scope.model.clone(),
                    messages_hash: message_hash.clone(),
                    input_tokens: input.try_into()?,
                    output_tokens: output.try_into()?,
                    model_call_id: response
                        .call_id
                        .context("count probe telemetry could not be persisted")?,
                };
                validate_receipt(scope, &message_hash, &receipt)?;
                write_json_atomic(
                    &path,
                    &CachedCount {
                        identity: scope.identity.clone(),
                        receipt: receipt.clone(),
                    },
                )?;
                lock.captured();
                Ok::<_, anyhow::Error>(receipt)
            })
            .await?;
        Ok(value.clone())
    }
}
fn validate_receipt(
    scope: &CounterScope,
    hash: &str,
    receipt: &ProbeReceipt,
) -> anyhow::Result<()> {
    ensure!(
        receipt.model == scope.model
            && receipt.messages_hash == hash
            && !receipt.model_call_id.is_empty(),
        "count receipt identity mismatch"
    );
    ensure!(
        receipt.input_tokens > 0,
        "count probe did not evaluate prompt"
    );
    ensure!(
        receipt.output_tokens <= scope.config.probe_mode.output_limit(),
        "count probe exceeded generation cap"
    );
    Ok(())
}
fn write_json_atomic(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(file.as_file_mut(), value)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests;
