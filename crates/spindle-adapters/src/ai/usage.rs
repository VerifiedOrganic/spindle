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

impl ModelRouter {
    pub(crate) fn with_usage_pool(mut self, pool: crate::sqlite::SqlitePool) -> Self {
        self.usage_pool = Some(pool);
        self
    }

    pub(super) async fn record_usage(
        &self,
        route: &str,
        context: Option<&RequestContext>,
        start: std::time::Instant,
        result: &mut anyhow::Result<ModelResponse>,
    ) {
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        if let Ok(response) = result {
            response.elapsed_ms = Some(elapsed_ms);
        }
        let Some(pool) = &self.usage_pool else {
            return;
        };
        let now = chrono::Utc::now();
        let record = ModelCallRecord {
            id: format!("model_call:{}", ulid::Ulid::new()),
            project_id: context.and_then(|c| c.project_id.clone()),
            scene_id: context.and_then(|c| c.scene_id.clone()),
            route: route.to_string(),
            adapter_kind: result.as_ref().ok().map(|r| r.adapter_kind.clone()),
            model_name: result.as_ref().ok().map(|r| r.model_name.clone()),
            outcome: if result.is_ok() {
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
        let saved = pool.write(move |conn| {
            let payload = serde_json::to_string(&record)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            conn.execute("INSERT INTO model_call (id, project_id, recorded_at, payload) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![record.id, record.project_id, now.timestamp_micros(), payload])?;
            Ok(())
        }).await;
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
