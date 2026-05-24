mod artifacts;
mod execution;
mod mcp;
mod operator;
mod plan;
mod state;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use execution::execute_one;
use mcp::{McpHarnessClient, TransportConfig};
use operator::{render_status, resolve_scene_block, review_checkpoint};
use plan::{FindingSeverity, NextAction, reconcile_state};
use state::{HarnessState, ScenePhase, load_seed};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "spindle-harness")]
#[command(about = "Continuity-first harness state manager for Spindle")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init(InitCommand),
    Status(StatusCommand),
    Verify(VerifyCommand),
    Resume(ResumeCommand),
    ReviewCheckpoint(ReviewCheckpointCommand),
    ResolveSceneBlock(ResolveSceneBlockCommand),
}

#[derive(Debug, Args)]
struct InitCommand {
    #[arg(long)]
    state: PathBuf,
    #[arg(long)]
    seed: PathBuf,
    #[command(flatten)]
    transport: TransportArgs,
}

#[derive(Debug, Args)]
struct StatusCommand {
    #[arg(long)]
    state: PathBuf,
    #[arg(
        long,
        help = "Show artifact paths, blocked reasons, and checkpoint details"
    )]
    verbose: bool,
}

#[derive(Debug, Args)]
struct VerifyCommand {
    #[arg(long)]
    state: PathBuf,
    #[arg(long)]
    writeback: bool,
    #[command(flatten)]
    transport: TransportArgs,
}

#[derive(Debug, Args)]
struct ResumeCommand {
    #[arg(long)]
    state: PathBuf,
    #[arg(long)]
    writeback: bool,
    #[arg(
        long,
        help = "Execute exactly one continuity-safe action instead of a dry run"
    )]
    execute_one: bool,
    #[command(flatten)]
    transport: TransportArgs,
}

#[derive(Debug, Args)]
struct ReviewCheckpointCommand {
    #[arg(long)]
    state: PathBuf,
    #[arg(long)]
    start_chapter: i32,
    #[arg(long)]
    end_chapter: i32,
    #[arg(long = "directive")]
    directives: Vec<String>,
}

#[derive(Debug, Args)]
struct ResolveSceneBlockCommand {
    #[arg(long)]
    state: PathBuf,
    #[arg(long)]
    chapter_number: i32,
    #[arg(long)]
    scene_order: i32,
    #[arg(long, value_enum)]
    target_phase: ScenePhaseArg,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ScenePhaseArg {
    DraftSaved,
    ChangesCommitted,
    BeatsAnnotated,
}

impl ScenePhaseArg {
    fn to_state_phase(self) -> ScenePhase {
        match self {
            Self::DraftSaved => ScenePhase::DraftSaved,
            Self::ChangesCommitted => ScenePhase::ChangesCommitted,
            Self::BeatsAnnotated => ScenePhase::BeatsAnnotated,
        }
    }
}

#[derive(Debug, Args, Clone)]
struct TransportArgs {
    #[arg(
        long,
        help = "Connect to an already-running spindle-mcp HTTP endpoint, e.g. http://127.0.0.1:4321/mcp"
    )]
    server_url: Option<String>,
    #[arg(
        long,
        help = "When spawning a local spindle-mcp child, set SPINDLE_DATA_DIR for that child"
    )]
    server_data_dir: Option<PathBuf>,
    #[arg(
        long,
        help = "When spawning a local spindle-mcp child, set SPINDLE_CONFIG for that child"
    )]
    server_config: Option<PathBuf>,
}

impl TransportArgs {
    fn to_config(&self) -> TransportConfig {
        match self.server_url.clone() {
            Some(url) => TransportConfig::Http { url },
            None => TransportConfig::Child {
                data_dir: self.server_data_dir.clone(),
                config_path: self.server_config.clone(),
            },
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();

    match cli.command {
        Command::Init(command) => init(command).await,
        Command::Status(command) => status(command),
        Command::Verify(command) => verify(command).await,
        Command::Resume(command) => resume(command).await,
        Command::ReviewCheckpoint(command) => review_checkpoint_command(command),
        Command::ResolveSceneBlock(command) => resolve_scene_block_command(command),
    }
}

async fn init(command: InitCommand) -> Result<()> {
    let seed = load_seed(&command.seed)?;
    let provisional = HarnessState::from_seed(seed.clone(), "bible_branch:unknown".to_string());
    let client = McpHarnessClient::connect(&command.transport.to_config()).await?;
    let snapshot = client.project_snapshot(&provisional).await?;
    let state = HarnessState::from_seed(seed, snapshot.active_branch_id.clone());
    let outcome = reconcile_state(state, &snapshot);

    print_findings(&outcome.findings);
    if outcome.has_errors() {
        anyhow::bail!(
            "refusing to write harness state because initialization is not continuity-safe"
        );
    }

    outcome.state.save(&command.state)?;
    println!("Initialized harness state at {}", command.state.display());
    println!("Active branch: {}", outcome.state.active_branch_id);
    println!("Next action: {}", outcome.next_action);
    Ok(())
}

fn status(command: StatusCommand) -> Result<()> {
    let state = HarnessState::load(&command.state)?;
    print!("{}", render_status(&state, &command.state, command.verbose));
    Ok(())
}

async fn verify(command: VerifyCommand) -> Result<()> {
    let state = HarnessState::load(&command.state)?;
    let client = McpHarnessClient::connect(&command.transport.to_config()).await?;
    let snapshot = client.project_snapshot(&state).await?;
    let outcome = reconcile_state(state, &snapshot);

    print_findings(&outcome.findings);
    println!("Next action: {}", outcome.next_action);

    if command.writeback {
        if outcome.has_errors() {
            anyhow::bail!(
                "verification found blocking issues; refusing to write back mutated state"
            );
        }
        outcome.state.save(&command.state)?;
        println!("Updated harness state at {}", command.state.display());
    }

    if outcome.has_errors() {
        anyhow::bail!("verification blocked by continuity safety checks");
    }
    Ok(())
}

async fn resume(command: ResumeCommand) -> Result<()> {
    let state = HarnessState::load(&command.state)?;
    let client = McpHarnessClient::connect(&command.transport.to_config()).await?;
    let snapshot = client.project_snapshot(&state).await?;
    let outcome = reconcile_state(state, &snapshot);

    print_findings(&outcome.findings);

    if command.writeback {
        if outcome.has_errors() {
            anyhow::bail!(
                "resume reconciliation found blocking issues; refusing to write back mutated state"
            );
        }
        outcome.state.save(&command.state)?;
        println!("Updated harness state at {}", command.state.display());
    }

    println!("Next action: {}", outcome.next_action);
    if outcome.has_errors() {
        anyhow::bail!("resume blocked by continuity safety checks");
    }

    if command.execute_one {
        let execution = execute_one(
            &command.state,
            outcome.state,
            &client,
            outcome.next_action.clone(),
        )
        .await?;
        println!("{}", execution.message);

        let snapshot = client.project_snapshot(&execution.state).await?;
        let verified = reconcile_state(execution.state, &snapshot);
        print_findings(&verified.findings);
        println!("Next action: {}", verified.next_action);
        if verified.has_errors() {
            anyhow::bail!("post-execution verification found continuity issues");
        }
        verified.state.save(&command.state)?;
        println!("Updated harness state at {}", command.state.display());
        return Ok(());
    }

    match outcome.next_action {
        NextAction::Blocked => anyhow::bail!("resume blocked"),
        _ => {
            println!("Execution is not implemented yet; this is a continuity-safe dry run.");
            Ok(())
        }
    }
}

fn review_checkpoint_command(command: ReviewCheckpointCommand) -> Result<()> {
    let mut state = HarnessState::load(&command.state)?;
    let message = review_checkpoint(
        &mut state,
        &command.state,
        command.start_chapter,
        command.end_chapter,
        &command.directives,
    )?;
    println!("{message}");
    Ok(())
}

fn resolve_scene_block_command(command: ResolveSceneBlockCommand) -> Result<()> {
    let mut state = HarnessState::load(&command.state)?;
    let message = resolve_scene_block(
        &mut state,
        &command.state,
        command.chapter_number,
        command.scene_order,
        command.target_phase.to_state_phase(),
    )?;
    println!("{message}");
    Ok(())
}

fn print_findings(findings: &[plan::Finding]) {
    if findings.is_empty() {
        println!("No findings.");
        return;
    }

    for finding in findings {
        let label = match finding.severity {
            FindingSeverity::Error => "error",
            FindingSeverity::Warning => "warning",
            FindingSeverity::Info => "info",
        };
        println!("[{label}] {}: {}", finding.code, finding.message);
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
