use super::{ModelResponse, ModelRouter, RequestContext};
use spindle_core::models::{ModelCallRecord, ModelUsage};

pub(super) fn parse_usage(body: &serde_json::Value) -> Option<ModelUsage> {
    let usage = body.get("usage")?.as_object()?;
    let number = |keys: &[&str]| keys.iter().find_map(|key| usage.get(*key)?.as_u64());
    let result = ModelUsage {
        input_tokens: number(&["prompt_tokens", "input_tokens"]),
        output_tokens: number(&["completion_tokens", "output_tokens"]),
        cached_input_tokens: usage
            .get("prompt_tokens_details")
            .or_else(|| usage.get("input_tokens_details"))
            .and_then(|details| details.get("cached_tokens"))
            .and_then(serde_json::Value::as_u64),
    };
    (result.input_tokens.is_some() || result.output_tokens.is_some()).then_some(result)
}

#[derive(Default)]
pub(super) struct UsageBinding {
    pub id: Option<String>,
    pub pending: bool,
}

impl ModelRouter {
    pub(crate) fn with_usage_pool(mut self, pool: crate::sqlite::SqlitePool) -> Self {
        self.usage_pool = Some(pool);
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record_usage(
        &self,
        route: &str,
        rating: Option<&str>,
        context: Option<&RequestContext>,
        prompt: &str,
        is_continuation: bool,
        resolved_route: Option<&super::ModelRoute>,
        start: std::time::Instant,
        result: &mut anyhow::Result<ModelResponse>,
    ) {
        self.record_bound_usage(
            route,
            rating,
            context,
            prompt,
            is_continuation,
            resolved_route,
            start,
            result,
            UsageBinding::default(),
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record_bound_usage(
        &self,
        route: &str,
        rating: Option<&str>,
        context: Option<&RequestContext>,
        prompt: &str,
        is_continuation: bool,
        resolved_route: Option<&super::ModelRoute>,
        start: std::time::Instant,
        result: &mut anyhow::Result<ModelResponse>,
        binding: UsageBinding,
    ) {
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        if let Ok(response) = result {
            response.elapsed_ms = Some(elapsed_ms);
        }
        let Some(pool) = &self.usage_pool else {
            return;
        };
        let now = chrono::Utc::now();
        let adapter_kind_hint = result
            .as_ref()
            .ok()
            .map(|r| r.adapter_kind.as_str())
            .or_else(|| resolved_route.map(|r| r.adapter_kind.as_str()));
        let (inferred_template_id, inferred_prompt_version) =
            super::infer_prompt_template(route, rating, adapter_kind_hint, prompt, is_continuation);
        let prompt_template_id = context
            .and_then(|c| c.prompt_template_id.clone())
            .or_else(|| Some(inferred_template_id.to_string()));
        let prompt_version = context
            .and_then(|c| c.prompt_version.clone())
            .or_else(|| Some(inferred_prompt_version.to_string()));
        let context_packet_hash = context
            .and_then(|c| c.context_packet_hash.clone())
            .or_else(|| Some(super::compute_context_packet_hash(prompt)));
        let mut record = ModelCallRecord {
            id: binding
                .id
                .unwrap_or_else(|| format!("model_call:{}", ulid::Ulid::new())),
            project_id: context.and_then(|c| c.project_id.clone()),
            scene_id: context.and_then(|c| c.scene_id.clone()),
            run_id: context.and_then(|c| c.run_id.clone()),
            route: route.to_string(),
            adapter_kind: result
                .as_ref()
                .ok()
                .map(|r| r.adapter_kind.clone())
                .or_else(|| resolved_route.map(|r| r.adapter_kind.clone())),
            model_name: result
                .as_ref()
                .ok()
                .map(|r| r.model_name.clone())
                .or_else(|| resolved_route.map(|r| r.model_name.clone())),
            prompt_template_id,
            prompt_version,
            context_packet_hash,
            temperature: resolved_route.and_then(|r| r.temperature),
            top_p: resolved_route.and_then(|r| r.top_p),
            max_tokens: resolved_route.and_then(|r| r.max_tokens),
            outcome: if binding.pending {
                "pending"
            } else if result.is_ok() {
                "succeeded"
            } else {
                "failed"
            }
            .into(),
            usage: result.as_ref().ok().and_then(|r| r.usage.clone()),
            elapsed_ms,
            recorded_at: now.to_rfc3339(),
        };
        let id = record.id.clone();
        let saved = pool
            .write(move |conn| {
                if record.run_id.is_none()
                    && let Some(project_id) = record.project_id.as_deref()
                {
                    record.run_id = conn
                        .query_row(
                            "SELECT id FROM authoring_run WHERE project_id = ?1 AND status = 'active' ORDER BY updated_at DESC LIMIT 1",
                            rusqlite::params![project_id],
                            |row| row.get::<_, String>(0),
                        )
                        .ok();
                }
                let payload = serde_json::to_string(&record)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                conn.execute(
                    "INSERT INTO model_call (
                        id, project_id, scene_id, run_id, route,
                        prompt_template_id, prompt_version, context_packet_hash,
                        temperature, top_p, max_tokens, recorded_at, payload
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                     ON CONFLICT(id) DO UPDATE SET
                        project_id=excluded.project_id, scene_id=excluded.scene_id,
                        run_id=excluded.run_id, route=excluded.route,
                        prompt_template_id=excluded.prompt_template_id,
                        prompt_version=excluded.prompt_version,
                        context_packet_hash=excluded.context_packet_hash,
                        temperature=excluded.temperature, top_p=excluded.top_p,
                        max_tokens=excluded.max_tokens, recorded_at=excluded.recorded_at,
                        payload=excluded.payload",
                    rusqlite::params![
                        record.id,
                        record.project_id,
                        record.scene_id,
                        record.run_id,
                        record.route,
                        record.prompt_template_id,
                        record.prompt_version,
                        record.context_packet_hash,
                        record.temperature,
                        record.top_p,
                        record.max_tokens,
                        now.timestamp_micros(),
                        payload
                    ],
                )?;
                Ok(())
            })
            .await;
        match saved {
            Ok(()) => {
                if let Ok(response) = result {
                    response.call_id = Some(id);
                }
            }
            // A metrics write must never throw away paid-for generated prose.
            Err(error) => tracing::warn!(%error, "model usage could not be persisted"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_preserves_zero_unknown_and_cached_tokens() {
        let usage = parse_usage(&serde_json::json!({"usage": {"prompt_tokens": 80, "completion_tokens": 0, "prompt_tokens_details": {"cached_tokens": 60}}})).unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens
            ),
            (Some(80), Some(0), Some(60))
        );
        assert!(parse_usage(&serde_json::json!({})).is_none());
        let partial =
            parse_usage(&serde_json::json!({"usage": {"input_tokens": 20, "output_tokens": -1}}))
                .unwrap();
        assert_eq!(partial.output_tokens, None);
    }
}
