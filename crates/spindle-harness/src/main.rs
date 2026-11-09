use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use spindle_harness::editorial;
use spindle_harness::execution::execute_one;
use spindle_harness::mcp::{McpHarnessClient, TransportConfig};
use spindle_harness::operator::{render_status, resolve_scene_block, review_checkpoint};
use spindle_harness::plan::{Finding, FindingSeverity, NextAction, reconcile_state};
use spindle_harness::state::{HarnessState, ScenePhase, load_seed};
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
    #[arg(
        long,
        help = "Path to a hybrid host-draft submission JSON ({task_id, packet_hash, text, usage?, truncated?})"
    )]
    host_submission: Option<PathBuf>,
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
    if outcome.state.resolved_editorial.is_some() {
        println!("Editorial settings persisted (opt-in critique_v1 controller).");
    }
    println!("Next action: {}", outcome.next_action);
    Ok(())
}

fn status(command: StatusCommand) -> Result<()> {
    let state = HarnessState::load(&command.state)?;
    print!("{}", render_status(&state, &command.state, command.verbose));
    if state.resolved_editorial.is_some() {
        print!(
            "{}",
            editorial::render_editorial_status(&state, command.verbose)
        );
    }
    Ok(())
}

async fn verify(command: VerifyCommand) -> Result<()> {
    let state = HarnessState::load(&command.state)?;
    verify_editorial_state(&state)?;
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
    if outcome.state.resolved_editorial.is_some() {
        print!(
            "{}",
            editorial::render_editorial_status(&outcome.state, false)
        );
    }

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
        if outcome.state.resolved_editorial.is_some() {
            return execute_one_editorial(&command, &outcome.state);
        }
        if let Some(code) = execute_one_completion_exit_code(&outcome.next_action) {
            println!("Harness range is complete.");
            std::process::exit(code);
        }
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
        NextAction::AwaitResearch { .. } => {
            anyhow::bail!(
                "resume blocked: action requires research. Use research tools to satisfy requirements, then resume."
            );
        }
        _ => {
            if outcome.state.resolved_editorial.is_some() {
                println!(
                    "Dry run: editorial action reported above; pass --execute-one to advance one persisted action."
                );
            } else {
                println!("Execution is not implemented yet; this is a continuity-safe dry run.");
            }
            Ok(())
        }
    }
}

fn verify_editorial_state(state: &HarnessState) -> Result<()> {
    if state.resolved_editorial.is_none() {
        return Ok(());
    }
    let Some(path) = editorial::controller_path(state) else {
        return Ok(());
    };
    if !path.exists() {
        println!("Editorial controller: not prepared at {}", path.display());
        return Ok(());
    }
    editorial::load_editorial_controller(&path)?;
    println!("Editorial controller verified: {}", path.display());
    Ok(())
}

fn execute_one_editorial(command: &ResumeCommand, state: &HarnessState) -> Result<()> {
    let Some(settings) = &state.resolved_editorial else {
        anyhow::bail!("editorial execution requires resolved editorial settings");
    };
    let Some(path) = editorial::controller_path(state) else {
        anyhow::bail!(
            "editorial controller is not prepared; the server-side prepare step must create the controller artifact first"
        );
    };
    let mut controller = editorial::load_editorial_controller(&path)?;
    if let Some(submission_path) = &command.host_submission {
        let submission = editorial::parse_host_submission(submission_path)?;
        let (outcome, _key) = editorial::accept_host_draft_submission(
            &mut controller,
            &submission,
            editorial::now_millis(),
        )?;
        match &outcome {
            spindle_core::drafting::HostSubmissionOutcomeV1::DraftAccepted { receipt, .. } => {
                let text_path =
                    editorial::write_host_submission_artifacts(&path, &submission, receipt)?;
                editorial::save_editorial_controller(&path, &controller)?;
                println!(
                    "Accepted host draft for task {}; exact prose retained at {}.",
                    receipt.task_id,
                    text_path.display()
                );
                Ok(())
            }
            spindle_core::drafting::HostSubmissionOutcomeV1::Rejected { rejection, message } => {
                anyhow::bail!(
                    "host submission rejected ({}): {message}",
                    rejection.as_str()
                )
            }
            _ => anyhow::bail!(
                "revision host submissions require server-side deterministic checks and judging"
            ),
        }
    } else {
        let Some(scene) = controller
            .scenes
            .iter()
            .find(|scene| spindle_core::drafting::editorial_next_action(scene, settings).is_some())
        else {
            println!("Editorial controller has no pending action.");
            return Ok(());
        };
        if let Some(task) = scene.find_pending_task() {
            println!(
                "Awaiting host submission for task {} ({:?}).",
                task.task_id, task.kind
            );
            println!("Submit a plain-text draft with --execute-one --host-submission <json>.");
            return Ok(());
        }
        if let Some(action) = spindle_core::drafting::editorial_next_action(scene, settings) {
            println!(
                "Editorial next action: {} (stage={}, reserve calls={} tokens={} millis={})",
                action.action.as_str(),
                action.stage.as_str(),
                action.reservation.calls,
                action.reservation.tokens,
                action.reservation.millis
            );
            if let Some(route) = &action.missing_route {
                println!("Missing route: {route}");
            }
            if action.missing_tokenizer {
                println!("Missing verified tokenizer: yes");
            }
            if action.pending_manual_judgment {
                println!(
                    "Editorial action parked for author review; no automated judge preference was applied."
                );
                return Ok(());
            }
            if action.action.is_model_pass() {
                anyhow::bail!(
                    "editorial action '{}' requires server-side dispatch; the CLI does not hold model routes",
                    action.action.as_str()
                );
            }
        }
        println!("Editorial controller has no locally executable action.");
        Ok(())
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
    // The offline CLI cannot reconcile against Spindle to detect a run-level
    // block, so only scene-level blocks are resolvable here; the MCP tool
    // (authoring_resolve_block) covers the run-level case.
    let message = resolve_scene_block(
        &mut state,
        &command.state,
        command.chapter_number,
        command.scene_order,
        command.target_phase.to_state_phase(),
        false,
    )?;
    println!("{message}");
    Ok(())
}

fn print_findings(findings: &[Finding]) {
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

/// Distinct exit code returned by `resume --execute-one` when the run is
/// already `Complete` (non-zero so `while ...; do` loops terminate, and
/// distinct from `1` which signals a checkpoint/research block or error).
const EXIT_CODE_COMPLETE: i32 = 2;

fn execute_one_completion_exit_code(next_action: &NextAction) -> Option<i32> {
    (*next_action == NextAction::Complete).then_some(EXIT_CODE_COMPLETE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spindle_harness::state::HarnessSeed;

    #[test]
    fn editorial_execute_one_without_prepared_controller_is_honest() {
        let seed = HarnessSeed {
            project_id: "project:test".into(),
            book_number: 1,
            range: spindle_harness::state::ChapterRange {
                start_chapter: 1,
                end_chapter: 1,
            },
            checkpoint_interval: 1,
            editorial_directives: Vec::new(),
            mining_policy: None,
            max_revise_attempts: None,
            checkpoint_policy: None,
            replan_policy: None,
            editorial: Some(spindle_core::drafting::EditorialRunConfig {
                editorial_policy: Some(spindle_core::drafting::EditorialPolicy::CritiqueV1),
                ..Default::default()
            }),
            editorial_controller_path: None,
            chapters: Vec::new(),
        };
        let state = HarnessState::from_seed(seed, "branch:test".into());
        let command = ResumeCommand {
            state: PathBuf::from("state.json"),
            writeback: false,
            execute_one: true,
            host_submission: None,
            transport: TransportArgs {
                server_url: None,
                server_data_dir: None,
                server_config: None,
            },
        };
        let error = execute_one_editorial(&command, &state).expect_err("no controller");
        assert!(
            error.to_string().contains("not prepared"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn legacy_state_still_skips_editorial_execution() {
        let seed = HarnessSeed {
            project_id: "project:test".into(),
            book_number: 1,
            range: spindle_harness::state::ChapterRange {
                start_chapter: 1,
                end_chapter: 1,
            },
            checkpoint_interval: 1,
            editorial_directives: Vec::new(),
            mining_policy: None,
            max_revise_attempts: None,
            checkpoint_policy: None,
            replan_policy: None,
            editorial: None,
            editorial_controller_path: None,
            chapters: Vec::new(),
        };
        let state = HarnessState::from_seed(seed, "branch:test".into());
        assert!(state.resolved_editorial.is_none());
    }

    #[test]
    fn execute_one_completion_exit_code_returns_distinct_non_zero_code_for_complete() {
        assert_eq!(
            execute_one_completion_exit_code(&NextAction::Complete),
            Some(EXIT_CODE_COMPLETE)
        );
        assert_ne!(EXIT_CODE_COMPLETE, 0);
        assert_ne!(EXIT_CODE_COMPLETE, 1);
        assert_eq!(
            execute_one_completion_exit_code(&NextAction::DraftScene {
                chapter_number: 1,
                scene_order: 1,
            }),
            None
        );
        assert_eq!(
            execute_one_completion_exit_code(&NextAction::SaveChapterSummary { chapter_number: 1 }),
            None
        );
    }
}
