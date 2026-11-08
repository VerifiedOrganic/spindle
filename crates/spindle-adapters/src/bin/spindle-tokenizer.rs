//! Explicit setup/counting utility, not an authoring controller.
use anyhow::{Context, Result, bail};
use spindle_adapters::{ModelRouter, SqlitePool};
#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, config, database, route, rating] if command == "calibrate" => {
            let pool = SqlitePool::open(std::path::Path::new(database)).await?;
            let router = ModelRouter::for_tokenizer_tools(config, Some(pool))?;
            let certificate = router.calibrate_tokenizer(route, rating).await?;
            println!("{}", serde_json::to_string_pretty(&certificate)?);
        }
        [command, config, route, rating, messages] if command == "count" => {
            let messages: spindle_core::drafting::DraftMessages = serde_json::from_reader(
                std::fs::File::open(messages).context("read messages JSON")?,
            )?;
            let router = ModelRouter::for_tokenizer_tools(config, None)?;
            let count = router.count_tokenizer_messages(route, rating, &messages)?;
            println!(
                "{}",
                serde_json::json!({"input_tokens":count,"counter":"pinned_local"})
            );
        }
        _ => bail!(
            "usage: spindle-tokenizer calibrate CONFIG USAGE_DB ROUTE RATING | count CONFIG ROUTE RATING MESSAGES_JSON"
        ),
    }
    Ok(())
}
