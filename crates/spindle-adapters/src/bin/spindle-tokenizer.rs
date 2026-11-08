//! Explicit setup/counting and evaluation generation bridge utility, not an authoring controller.
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use spindle_adapters::{
    ModelRouter, SqlitePool,
    drafting::artifact::{accept_artifact, load_artifact},
    sqlite::SqliteSpindleService,
    sqlite::repository::Repository,
};
use spindle_core::drafting::{
    CriticFindingInput, DraftMessages, DraftingPacketV1, EditorialPolicy,
};
use std::path::Path;

fn parse_optional_seed(raw: Option<&String>) -> Result<Option<i64>> {
    match raw {
        Some(value) if !value.trim().is_empty() => {
            Ok(Some(value.trim().parse::<i64>().with_context(|| {
                format!("invalid generation seed '{value}'")
            })?))
        }
        _ => Ok(None),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, config, database, route, rating] if command == "calibrate" => {
            let pool = SqlitePool::open(Path::new(database)).await?;
            let router = ModelRouter::for_tokenizer_tools(config, Some(pool))?;
            let certificate = router.calibrate_tokenizer(route, rating).await?;
            println!("{}", serde_json::to_string_pretty(&certificate)?);
        }
        [command, config, route, rating, messages] if command == "count" => {
            let messages: DraftMessages = serde_json::from_reader(
                std::fs::File::open(messages).context("read messages JSON")?,
            )?;
            let router = ModelRouter::for_tokenizer_tools(config, None)?;
            let count = router.count_tokenizer_messages(route, rating, &messages)?;
            println!(
                "{}",
                serde_json::json!({"input_tokens":count,"counter":"pinned_local"})
            );
        }
        [command, config, route, rating, packet_json, rest @ ..]
            if command == "verify-packet" && rest.len() <= 1 =>
        {
            let seed = parse_optional_seed(rest.first())?;
            let packet: DraftingPacketV1 = serde_json::from_reader(
                std::fs::File::open(packet_json).context("read packet JSON")?,
            )?;
            let router = ModelRouter::for_tokenizer_tools(config, None)?;
            let prepared = router.verify_packet_offline(route, rating, packet, seed)?;
            println!("{}", serde_json::to_string_pretty(&prepared)?);
        }
        [
            command,
            config,
            database,
            packet_json,
            artifact_path,
            rest @ ..,
        ] if command == "draft-packet" && rest.len() <= 1 => {
            let seed = parse_optional_seed(rest.first())?;
            let packet: DraftingPacketV1 = serde_json::from_reader(
                std::fs::File::open(packet_json).context("read packet JSON")?,
            )?;
            let db_path = Path::new(database);
            let pool = SqlitePool::open(db_path).await?;
            let router = ModelRouter::for_tokenizer_tools(config, Some(pool.clone()))?;
            let (budget, settings) = router.eval_draft_budget_and_settings(
                "draft",
                packet.scene_card.rating.as_str(),
                seed,
            )?;
            let data_dir = db_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .join("eval-data");
            let repository = Repository::with_model_router(pool.clone(), data_dir, router);
            let service = SqliteSpindleService::new(repository);
            service
                .ensure_eval_project_branch(&packet.project_id, &packet.branch_id)
                .await?;
            let path = Path::new(artifact_path);
            let mut artifact = service
                .draft_packet_to_artifact(EditorialPolicy::CritiqueV1, packet, &budget, None, path)
                .await?;
            if !artifact.truncated
                && artifact.accepted_text_sha256.is_none()
                && let Ok(accepted) = accept_artifact(path, &artifact.text_sha256)
            {
                artifact = accepted;
            }
            let assembly = artifact
                .prepared_packet
                .as_ref()
                .and_then(|p| p.packet.assembly.as_ref())
                .context("prepared packet assembly missing")?;
            let packet_hash_query = artifact.packet_hash.clone();
            let (output_tokens, elapsed_ms): (Option<u64>, Option<u64>) = pool
                .read(move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT json_extract(payload, '$.usage.output_tokens'), \
                                json_extract(payload, '$.elapsed_ms') \
                         FROM model_call \
                         WHERE prompt_template_id = 'draft.packet' AND context_packet_hash = ?1 \
                         ORDER BY recorded_at DESC LIMIT 1",
                    )?;
                    let mut rows = stmt.query([&packet_hash_query])?;
                    if let Some(row) = rows.next()? {
                        Ok((row.get(0)?, row.get(1)?))
                    } else {
                        Ok((None, None))
                    }
                })
                .await
                .unwrap_or((None, None));
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "artifact_id": artifact.artifact_id,
                    "artifact_path": artifact_path,
                    "project_id": artifact.project_id,
                    "branch_id": artifact.branch_id,
                    "scene_id": artifact.scene_id,
                    "packet_hash": artifact.packet_hash,
                    "source_packet_sha256": artifact.source_packet_sha256,
                    "prompt_hash": artifact.packet_hash,
                    "tokenizer_id": assembly.tokenizer_id,
                    "tokenizer_version": assembly.tokenizer_version,
                    "input_tokens": assembly.input_tokens,
                    "output_tokens": output_tokens,
                    "elapsed_ms": elapsed_ms,
                    "generation_seed": assembly.generation_seed,
                    "model": assembly.model_id,
                    "agent_id": artifact.receipt.agent_id,
                    "settings": settings,
                    "receipt": artifact.receipt,
                    "truncated": artifact.truncated,
                    "text_sha256": artifact.text_sha256,
                    "accepted_text_sha256": artifact.accepted_text_sha256,
                    "text": artifact.text,
                }))?
            );
        }
        [
            command,
            config,
            database,
            route,
            rating,
            messages_json,
            rest @ ..,
        ] if command == "draft-messages" && rest.len() <= 1 => {
            let seed = parse_optional_seed(rest.first())?;
            let messages: DraftMessages = serde_json::from_reader(
                std::fs::File::open(messages_json).context("read messages JSON")?,
            )?;
            let pool = SqlitePool::open(Path::new(database)).await?;
            let router = ModelRouter::for_tokenizer_tools(config, Some(pool))?;
            let (budget, settings) = router.eval_draft_budget_and_settings(route, rating, seed)?;
            let (prompt_hash, input_tokens, response) = router
                .draft_eval_messages(route, rating, &messages, &budget, None)
                .await?;
            let text_sha256 = format!("{:x}", Sha256::digest(response.output.as_bytes()));
            let packet_hash = spindle_adapters::ai::compute_context_packet_hash(&messages.user);
            let output_tokens = response.usage.as_ref().and_then(|u| u.output_tokens);
            let model = response
                .provider_model
                .clone()
                .unwrap_or_else(|| budget.model_id.clone());
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "packet_hash": packet_hash,
                    "source_packet_sha256": serde_json::Value::Null,
                    "prompt_hash": prompt_hash,
                    "tokenizer_id": settings["tokenizer_id"],
                    "tokenizer_version": settings["tokenizer_version"],
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "elapsed_ms": response.elapsed_ms,
                    "generation_seed": budget.generation_seed,
                    "model": model,
                    "agent_id": response.model_name,
                    "call_id": response.call_id,
                    "settings": settings,
                    "truncated": response.truncated,
                    "text_sha256": text_sha256,
                    "text": response.output,
                }))?
            );
        }
        [
            command,
            config,
            database,
            draft_artifact_path,
            critique_artifact_path,
            rest @ ..,
        ] if command == "critique-artifact" && rest.len() <= 2 => {
            let findings: Vec<CriticFindingInput> = match rest.first() {
                Some(path) if !path.trim().is_empty() && path.trim() != "-" => {
                    serde_json::from_reader(
                        std::fs::File::open(path).context("read findings JSON")?,
                    )?
                }
                _ => Vec::new(),
            };
            let seed = parse_optional_seed(rest.get(1))?;
            let draft_path = Path::new(draft_artifact_path);
            let critique_path = Path::new(critique_artifact_path);
            let before_draft = load_artifact(draft_path)?;
            let packet = before_draft
                .prepared_packet
                .as_ref()
                .map(|p| &p.packet)
                .context("draft artifact missing prepared_packet")?;
            let rating = packet.scene_card.rating.as_str().to_string();
            let project_id = before_draft.project_id.clone();
            let branch_id = before_draft.branch_id.clone();
            let db_path = Path::new(database);
            let pool = SqlitePool::open(db_path).await?;
            let router = ModelRouter::for_tokenizer_tools(config, Some(pool.clone()))?;
            let (budget, settings) =
                router.eval_critic_budget_and_settings("review", &rating, seed)?;
            let data_dir = db_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .join("eval-data");
            let repository = Repository::with_model_router(pool, data_dir, router);
            let service = SqliteSpindleService::new(repository);
            service
                .ensure_eval_project_branch(&project_id, &branch_id)
                .await?;
            let shadow_artifact = service
                .critique_artifact_shadow(
                    EditorialPolicy::CritiqueV1,
                    draft_path,
                    critique_path,
                    findings,
                    &budget,
                    None,
                    2,
                )
                .await?;
            let after_draft = load_artifact(draft_path)?;
            let base_text_hash_unchanged = after_draft.text_sha256 == before_draft.text_sha256
                && after_draft.text == before_draft.text;
            anyhow::ensure!(
                base_text_hash_unchanged,
                "shadow_mode_violation: draft artifact changed during shadow critique"
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "critique_artifact_id": shadow_artifact.artifact_id,
                    "critique_artifact_path": critique_artifact_path,
                    "draft_artifact_id": shadow_artifact.draft_artifact_id,
                    "project_id": shadow_artifact.project_id,
                    "branch_id": shadow_artifact.branch_id,
                    "scene_id": shadow_artifact.scene_id,
                    "packet_hash": shadow_artifact.packet_hash,
                    "base_text_hash": shadow_artifact.base_text_hash,
                    "base_text_hash_unchanged": base_text_hash_unchanged,
                    "prompt_hashes": shadow_artifact.prompt_hashes,
                    "model_call_ids": shadow_artifact.model_call_ids,
                    "ingested_findings": shadow_artifact.ingested_findings,
                    "deduplicated_finding_ids": shadow_artifact.deduplicated_finding_ids,
                    "settings": settings,
                    "outcome": shadow_artifact.outcome,
                }))?
            );
        }
        _ => bail!(
            "usage: spindle-tokenizer calibrate CONFIG USAGE_DB ROUTE RATING | count CONFIG ROUTE RATING MESSAGES_JSON | verify-packet CONFIG ROUTE RATING PACKET_JSON [SEED] | draft-packet CONFIG USAGE_DB PACKET_JSON ARTIFACT_PATH [SEED] | draft-messages CONFIG USAGE_DB ROUTE RATING MESSAGES_JSON [SEED] | critique-artifact CONFIG USAGE_DB DRAFT_ARTIFACT_PATH CRITIQUE_ARTIFACT_PATH [FINDINGS_JSON|-] [SEED]"
        ),
    }
    Ok(())
}
