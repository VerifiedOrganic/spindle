//! Detached deep `check_consistency` jobs.
//!
//! A deep audit is `deep-tier checks x scenes` model calls and can run for
//! tens of minutes. Running it inline means an MCP client that gives up after
//! its request timeout also throws away the finished audit: the server keeps
//! computing, but there is no caller left to receive the result. The job
//! registry below detaches the work from the request, lets a retry join the
//! same run, and keeps the finished result collectable for a short window.
//!
//! Jobs are intentionally in-memory. A server restart drops every job; the
//! next call starts a fresh run. Per-scene deep-tier cache rows survive the
//! restart, so scenes already analyzed are not re-paid. This is documented on
//! the `check_consistency` tool and in `docs/semantic-continuity-design.md`.

use super::*;
use spindle_core::models::{AuditCoverage, CheckConsistencyOutput, ConsistencyIssue};

/// Progress events emitted after each deep-tier check. The MCP layer forwards
/// these as `notifications/progress` when the caller supplied a progress token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsistencyProgress {
    pub job_id: String,
    pub checks_completed: Vec<String>,
    pub checks_pending: Vec<String>,
    pub issue_count: usize,
}

/// Receiver side of the progress stream. Sending is best-effort: a closed
/// receiver (client disconnected) never fails the audit.
pub type ConsistencyProgressSink = tokio::sync::mpsc::UnboundedSender<ConsistencyProgress>;

/// Running-or-terminal state of a detached deep consistency job. The result
/// itself lives in [`ConsistencyJobShared::result`], not in the channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConsistencyJobState {
    Running,
    Done,
    Failed(String),
}

/// A detached consistency job. Cloning the handle is cheap and lets retries
/// join the same run.
#[derive(Clone)]
pub(crate) struct ConsistencyJob {
    pub(crate) job_id: String,
    pub(crate) state: tokio::sync::watch::Receiver<ConsistencyJobState>,
    pub(crate) shared: std::sync::Arc<std::sync::Mutex<ConsistencyJobShared>>,
}

/// Mutable state shared by the job task and every waiter.
pub(crate) struct ConsistencyJobShared {
    pub(crate) checks_completed: Vec<String>,
    pub(crate) checks_pending: Vec<String>,
    pub(crate) issues: Vec<ConsistencyIssue>,
    pub(crate) audit_coverage: Vec<AuditCoverage>,
    pub(crate) result: Option<CheckConsistencyOutput>,
    pub(crate) sinks: Vec<ConsistencyProgressSink>,
}

/// Point-in-time progress read by a joining caller.
pub(crate) struct ConsistencyJobSnapshot {
    pub(crate) checks_completed: Vec<String>,
    pub(crate) checks_pending: Vec<String>,
    pub(crate) issues: Vec<ConsistencyIssue>,
    pub(crate) audit_coverage: Vec<AuditCoverage>,
}

impl ConsistencyJob {
    pub(crate) fn snapshot(&self) -> Result<ConsistencyJobSnapshot> {
        let shared = self
            .shared
            .lock()
            .map_err(|_| poisoned_lock("consistency job state lock"))?;
        Ok(ConsistencyJobSnapshot {
            checks_completed: shared.checks_completed.clone(),
            checks_pending: shared.checks_pending.clone(),
            issues: shared.issues.clone(),
            audit_coverage: shared.audit_coverage.clone(),
        })
    }

    pub(crate) fn add_sink(&self, sink: ConsistencyProgressSink) -> Result<()> {
        let mut shared = self
            .shared
            .lock()
            .map_err(|_| poisoned_lock("consistency job state lock"))?;
        if shared.result.is_none() {
            shared.sinks.push(sink);
        }
        Ok(())
    }

    pub(crate) fn completed_output(&self) -> Result<Option<CheckConsistencyOutput>> {
        let shared = self
            .shared
            .lock()
            .map_err(|_| poisoned_lock("consistency job state lock"))?;
        Ok(shared.result.clone())
    }
}

/// The handle handed to the check runner. Every deep-tier completion records
/// the checks finished so far and fans a progress event out to joined callers.
#[derive(Clone)]
pub(crate) struct ConsistencyJobTracker {
    job_id: String,
    shared: std::sync::Arc<std::sync::Mutex<ConsistencyJobShared>>,
}

impl ConsistencyJobTracker {
    pub(crate) fn new(
        job_id: String,
        shared: std::sync::Arc<std::sync::Mutex<ConsistencyJobShared>>,
    ) -> Self {
        Self { job_id, shared }
    }

    pub(crate) fn job_id(&self) -> &str {
        &self.job_id
    }

    pub(crate) fn completed_checks(&self) -> Vec<String> {
        self.shared
            .lock()
            .map(|shared| shared.checks_completed.clone())
            .unwrap_or_default()
    }

    /// Record one finished deep-tier check and notify every joined caller.
    /// The snapshot includes findings accumulated so far so an `in_progress`
    /// response is useful rather than an empty shell.
    pub(crate) fn stage_complete(
        &self,
        stage: &str,
        issues: &[ConsistencyIssue],
        audit_coverage: &[AuditCoverage],
    ) {
        let Ok(mut shared) = self.shared.lock() else {
            tracing::warn!(
                job_id = %self.job_id,
                stage,
                "consistency job state lock poisoned; skipping progress event"
            );
            return;
        };
        shared.checks_pending.retain(|pending| pending != stage);
        if !shared.checks_completed.iter().any(|done| done == stage) {
            shared.checks_completed.push(stage.to_string());
        }
        shared.issues = issues.to_vec();
        let mut coverage_snapshot = audit_coverage.to_vec();
        for coverage in &mut coverage_snapshot {
            coverage.finish();
        }
        shared.audit_coverage = coverage_snapshot;
        let event = ConsistencyProgress {
            job_id: self.job_id.clone(),
            checks_completed: shared.checks_completed.clone(),
            checks_pending: shared.checks_pending.clone(),
            issue_count: shared.issues.len(),
        };
        let sinks = shared.sinks.clone();
        drop(shared);
        for sink in sinks {
            // A disconnected client is not a job failure.
            let _ = sink.send(event.clone());
        }
        tracing::debug!(
            job_id = %event.job_id,
            completed = event.checks_completed.len(),
            pending = event.checks_pending.len(),
            issues = event.issue_count,
            "deep consistency job progress"
        );
    }

    /// Store the finished audit, emit one final progress event, and close the
    /// progress channels.
    pub(crate) fn complete(&self, output: CheckConsistencyOutput) {
        let Ok(mut shared) = self.shared.lock() else {
            tracing::warn!(
                job_id = %self.job_id,
                "consistency job state lock poisoned; completed result not published"
            );
            return;
        };
        shared.checks_pending.clear();
        shared.issues = output.issues.clone();
        shared.audit_coverage = output.audit_coverage.clone();
        shared.result = Some(output);
        let event = ConsistencyProgress {
            job_id: self.job_id.clone(),
            checks_completed: shared.checks_completed.clone(),
            checks_pending: Vec::new(),
            issue_count: shared.issues.len(),
        };
        let sinks = std::mem::take(&mut shared.sinks);
        drop(shared);
        for sink in sinks {
            let _ = sink.send(event.clone());
        }
    }

    /// Mark the run failed and close progress channels. The result remains
    /// unset so waiters surface the error instead of a partial audit.
    pub(crate) fn fail(&self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.sinks.clear();
        }
    }
}
