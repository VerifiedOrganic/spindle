//! Thin agent-entry orchestration over the planning staging, generation and
//! ratification services: begin/resume, advance, status and decide.
use super::*;

fn premise_preview(premise: &str) -> String {
    const MAX_CHARS: usize = 280;
    let mut preview: String = premise.chars().take(MAX_CHARS).collect();
    if premise.chars().count() > MAX_CHARS {
        preview.push('…');
    }
    preview
}

fn next_level(proposals: &[PlanningProposal]) -> PlanningLevel {
    PlanningLevel::all()
        .into_iter()
        .find(|level| {
            !proposals.iter().any(|p| {
                p.artifact.level == *level
                    && matches!(
                        p.status,
                        PlanningProposalStatus::Staged | PlanningProposalStatus::Applied
                    )
            })
        })
        .unwrap_or(PlanningLevel::Scene)
}

fn policy_summary(policy: &PlanningPolicy) -> PlanningPolicySummary {
    PlanningPolicySummary {
        gates: PlanningGatePolicy::from_gates(&policy.gates),
        policy_id: policy.policy_id.clone(),
        policy_version: policy.policy_version,
        max_advisory_findings: policy.max_advisory_findings,
        has_pov_policy: policy.pov_policy.is_some(),
        has_pacing_policy: policy.pacing_policy.is_some(),
    }
}

fn session_summary(
    session: &PlanningSession,
    proposals: &[PlanningProposal],
) -> PlanningSessionSummary {
    PlanningSessionSummary {
        session_id: session.id.clone(),
        premise_preview: premise_preview(&session.premise),
        intent_hash: session.intent_hash.clone(),
        project_id: session.scope.as_ref().map(|scope| scope.project_id.clone()),
        branch_id: session.scope.as_ref().map(|scope| scope.branch_id.clone()),
        next_level: next_level(proposals),
        policy: policy_summary(&session.policy),
        horizon: session.horizon.clone(),
        budget: session.budget.clone(),
        usage: session.usage.clone(),
        rating_ceiling: session.rating_ceiling.clone(),
        allow_review_backend: session.allow_review_backend,
        projection_stale: session.projection_stale,
        stale_reason: session.stale_reason.clone(),
        last_outcome: session.last_outcome,
        created_at: session.created_at,
    }
}

fn proposal_summary(proposal: &PlanningProposal) -> Result<PlanningProposalSummary> {
    Ok(PlanningProposalSummary {
        proposal_id: proposal.id.clone(),
        level: proposal.artifact.level,
        status: proposal.status,
        payload_hash: proposal.payload_hash.clone(),
        rationale: proposal.artifact.rationale.clone(),
        operations: proposal
            .artifact
            .operations
            .iter()
            .map(|item| {
                Ok(PlanningOperationSummary {
                    output_ref: item.output_ref.clone(),
                    operation: serde_json::to_value(&item.operation)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        schedule: serde_json::to_value(&proposal.artifact.schedule)?,
        assumptions: proposal.artifact.assumptions.clone(),
        author_questions: proposal.artifact.author_questions.clone(),
        findings: proposal.findings.clone(),
        decided_by: proposal.decided_by.clone(),
        decision_note: proposal.decision_note.clone(),
        created_at: proposal.created_at,
    })
}

fn readiness_summary(report: PlanningReadinessReport) -> PlanningReadinessSummary {
    PlanningReadinessSummary {
        target_range: report.target_range,
        ready_to_draft: report.ready_to_draft,
        planner_checks_passed: report.planner_checks_passed,
        stale_projection: report.stale_projection,
        findings: report.findings,
        pacing_metrics: report.pacing_metrics,
        prepare_run: PlanningPrepareRunSummary {
            project_id: report.prepare_run.project_id,
            book_number: report.prepare_run.book_number,
            start_chapter: report.prepare_run.start_chapter,
            end_chapter: report.prepare_run.end_chapter,
            ready_to_draft: report.prepare_run.ready_to_draft,
            missing_requirements: report.prepare_run.missing_requirements,
            chapters: report
                .prepare_run
                .details
                .into_iter()
                .map(|detail| PlanningPrepareChapterSummary {
                    chapter_number: detail.chapter_number,
                    ready: detail.ready,
                    missing_items: detail.missing_items,
                })
                .collect(),
        },
    }
}

fn host_request_summary(request: &PlanningHostRequest) -> PlanningHostRequestSummary {
    PlanningHostRequestSummary {
        level: request.level,
        route_task: request.route_task.clone(),
        prompt_id: request.prompt_id,
        prompt_hash: request.prompt_hash.clone(),
        schema_hash: request.schema_hash.clone(),
        system_prompt: request.system_prompt.clone(),
        user_prompt: request.user_prompt.clone(),
        artifact_schema: request.artifact_schema.clone(),
        parent_proposals: request.parent_proposals.clone(),
        target_range: request.target_range.clone(),
        route_problem: request
            .route_problem
            .as_ref()
            .map(|problem| serde_json::to_string(problem).unwrap_or_else(|_| "{}".to_string())),
        prior_findings: request.prior_findings.clone(),
    }
}

fn advance_summary(report: PlanningAdvanceReport) -> Result<PlanningAdvanceSummary> {
    let proposal = report.proposal.as_ref().map(proposal_summary).transpose()?;
    let (source, attempts_used, repaired) = match report.generation_receipt {
        Some(receipt) => (
            Some(receipt.source),
            receipt.attempts_used,
            receipt.repaired,
        ),
        None => (None, 0, false),
    };
    Ok(PlanningAdvanceSummary {
        attempt_id: report.attempt_id,
        level: report.level,
        outcome: report.outcome,
        proposal,
        source,
        attempts_used,
        repaired,
        findings: report.findings,
        author_questions: report.author_questions,
        host_request: report.host_request.as_ref().map(host_request_summary),
        usage: report.usage,
        budget: report.budget,
        message: report.message,
    })
}

fn gate_for(session: &PlanningSession, level: PlanningLevel) -> PlanningGate {
    PlanningGatePolicy::from_gates(&session.policy.gates).gate_for(level)
}

/// The active gate belongs to a proposal awaiting a decision, not to the next
/// unfilled level. Falling through to `next_level` would report Scene's gate
/// while a human-gated Chapter proposal is still staged.
fn gate_level(proposals: &[PlanningProposal]) -> PlanningLevel {
    proposals
        .iter()
        .find(|p| p.status == PlanningProposalStatus::Staged)
        .map(|p| p.artifact.level)
        .unwrap_or_else(|| next_level(proposals))
}

fn review_next_action(session: &PlanningSession, proposals: &[PlanningProposal]) -> String {
    if session.projection_stale {
        return "Resolve the stale planning projection before continuing: review the staged reconciliation proposal and record a human decision."
            .to_string();
    }
    if let Some(staged) = proposals
        .iter()
        .find(|p| p.status == PlanningProposalStatus::Staged)
    {
        if !staged.artifact.author_questions.is_empty() {
            return format!(
                "Answer the author questions on the staged {} proposal, supersede it, then call planning_advance to regenerate.",
                proposal_level_name(staged.artifact.level)
            );
        }
        return format!(
            "Review the staged {} proposal with the author, then call planning_decide with its payload_hash.",
            proposal_level_name(staged.artifact.level)
        );
    }
    if session.scope.is_none() {
        return "Call planning_advance to stage the Contract proposal. It stays staged until a human decision.".to_string();
    }
    let next = next_level(proposals);
    format!(
        "Call planning_advance to generate the next {} level, or planning_status to recheck readiness.",
        proposal_level_name(next)
    )
}

fn proposal_level_name(level: PlanningLevel) -> &'static str {
    match level {
        PlanningLevel::Contract => "Contract",
        PlanningLevel::CastWorld => "CastWorld",
        PlanningLevel::SeriesArc => "SeriesArc",
        PlanningLevel::Book => "Book",
        PlanningLevel::Chapter => "Chapter",
        PlanningLevel::Scene => "Scene",
    }
}

fn policy_from_override(policy: PlanningPolicyOverride) -> PlanningPolicy {
    PlanningPolicy {
        gates: policy.gates.to_gates(),
        policy_id: policy.policy_id,
        policy_version: policy.policy_version,
        max_advisory_findings: policy.max_advisory_findings,
        pov_policy: None,
        pacing_policy: None,
    }
}

fn reconnect_hint(session: &PlanningSession) -> String {
    format!(
        "Reconnect with planning_status(session_id=\"{}\") and continue with planning_advance; staged proposals and accepted IDs are durable.",
        session.id
    )
}

impl SqliteSpindleService {
    async fn planning_scope_for(
        &self,
        project_id: Option<&str>,
        branch_id: Option<&str>,
    ) -> Result<Option<PlanningScope>> {
        match (project_id, branch_id) {
            (None, None) => Ok(None),
            (None, Some(_)) => anyhow::bail!("branch_id requires project_id"),
            (Some(project_id), branch_id) => {
                let branch_id = match branch_id {
                    Some(branch_id) => branch_id.to_string(),
                    None => self.active_branch_id_for_project(project_id).await?,
                };
                Ok(Some(PlanningScope {
                    project_id: project_id.to_string(),
                    branch_id,
                }))
            }
        }
    }

    fn verify_requested_scope(
        session: &PlanningSession,
        project_id: Option<&str>,
        branch_id: Option<&str>,
    ) -> Result<()> {
        if let Some(project_id) = project_id
            && session
                .scope
                .as_ref()
                .is_some_and(|scope| scope.project_id != project_id)
        {
            anyhow::bail!("planning scope mismatch: session belongs to a different project");
        }
        if let Some(branch_id) = branch_id
            && session
                .scope
                .as_ref()
                .is_some_and(|scope| scope.branch_id != branch_id)
        {
            anyhow::bail!("planning scope mismatch: session belongs to a different branch");
        }
        Ok(())
    }

    pub async fn planning_begin_entry(
        &self,
        input: PlanningBeginInput,
    ) -> Result<PlanningBeginOutput> {
        let (session, resumed) = match input.session_id.as_deref() {
            Some(session_id) => {
                let session = self.get_planning_session(session_id).await?;
                Self::verify_requested_scope(
                    &session,
                    input.project_id.as_deref(),
                    input.branch_id.as_deref(),
                )?;
                // Resuming with a different premise is a deliberate intent
                // change; persist it and supersede staged work so the next
                // generation cannot be accepted under the old intent hash.
                let session = match input
                    .premise
                    .as_deref()
                    .map(str::trim)
                    .filter(|premise| !premise.is_empty() && *premise != session.premise)
                {
                    Some(premise) => {
                        self.revise_planning_premise(
                            session.id.clone(),
                            session.scope.clone(),
                            premise.to_string(),
                            "author".to_string(),
                            "Premise revised while resuming the planning session.".to_string(),
                        )
                        .await?
                    }
                    None => session,
                };
                let session = self
                    .configure_planning_session(ConfigurePlanningSessionInput {
                        session_id: session.id.clone(),
                        scope: session.scope.clone(),
                        horizon: input.horizon.clone(),
                        budget: input.budget.clone(),
                        rating_ceiling: input.rating_ceiling.clone(),
                        allow_review_backend: input.allow_review_backend,
                    })
                    .await?;
                (session, true)
            }
            None => {
                let premise = input
                    .premise
                    .as_deref()
                    .map(str::trim)
                    .filter(|premise| !premise.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "premise is required to start a new planning session; pass session_id to resume"
                        )
                    })?;
                let scope = self
                    .planning_scope_for(input.project_id.as_deref(), input.branch_id.as_deref())
                    .await?;
                let session = self
                    .begin_planning(BeginPlanningInput {
                        premise: premise.to_string(),
                        scope,
                    })
                    .await?;
                let session = self
                    .configure_planning_session(ConfigurePlanningSessionInput {
                        session_id: session.id.clone(),
                        scope: session.scope.clone(),
                        horizon: input.horizon.clone(),
                        budget: input.budget.clone(),
                        rating_ceiling: input.rating_ceiling.clone(),
                        allow_review_backend: input.allow_review_backend,
                    })
                    .await?;
                (session, false)
            }
        };

        let session = if let Some(policy) = input.policy {
            let actor = input
                .policy_actor
                .as_deref()
                .map(str::trim)
                .filter(|actor| !actor.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!("policy_actor is required with a policy override")
                })?;
            let note = input
                .policy_note
                .as_deref()
                .map(str::trim)
                .filter(|note| !note.is_empty())
                .ok_or_else(|| anyhow::anyhow!("policy_note is required with a policy override"))?;
            self.set_planning_policy(SetPlanningPolicyInput {
                session_id: session.id.clone(),
                scope: session.scope.clone(),
                policy: policy_from_override(policy),
                actor: actor.to_string(),
                note: note.to_string(),
            })
            .await?
        } else {
            session
        };

        let proposals = self
            .list_planning_proposals(&session.id, session.scope.clone())
            .await?;
        let next_action = if proposals
            .iter()
            .any(|p| p.status == PlanningProposalStatus::Staged)
        {
            review_next_action(&session, &proposals)
        } else {
            "Call planning_advance to generate the first planning level. Every level stays staged until a human decision.".to_string()
        };
        Ok(PlanningBeginOutput {
            session: session_summary(&session, &proposals),
            resumed,
            next_action,
            message: if resumed {
                "Resumed the durable planning session.".to_string()
            } else {
                "Started a durable planning session; no story records were written.".to_string()
            },
        })
    }

    pub async fn planning_advance_entry(
        &self,
        input: PlanningAdvanceInput,
    ) -> Result<PlanningAdvanceOutput> {
        let session = self.get_planning_session(&input.session_id).await?;
        Self::verify_requested_scope(
            &session,
            input.project_id.as_deref(),
            input.branch_id.as_deref(),
        )?;

        if input.action == PlanningAdvanceAction::SetPolicy {
            let policy = input
                .policy
                .clone()
                .ok_or_else(|| anyhow::anyhow!("policy is required for the set_policy action"))?;
            let actor = input
                .policy_actor
                .as_deref()
                .map(str::trim)
                .filter(|actor| !actor.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!("policy_actor is required for the set_policy action")
                })?;
            let note = input
                .policy_note
                .as_deref()
                .map(str::trim)
                .filter(|note| !note.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!("policy_note is required for the set_policy action")
                })?;
            let session = self
                .set_planning_policy(SetPlanningPolicyInput {
                    session_id: session.id.clone(),
                    scope: session.scope.clone(),
                    policy: policy_from_override(policy),
                    actor: actor.to_string(),
                    note: note.to_string(),
                })
                .await?;
            let proposals = self
                .list_planning_proposals(&session.id, session.scope.clone())
                .await?;
            return Ok(PlanningAdvanceOutput {
                session: session_summary(&session, &proposals),
                action: input.action,
                advance: None,
                readiness: None,
                next_action: review_next_action(&session, &proposals),
            });
        }

        if input.action == PlanningAdvanceAction::RecomputeReadiness {
            let scope = session.scope.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "readiness requires the session to be attached to an accepted project root"
                )
            })?;
            let report = self
                .planning_readiness(PlanningReadinessInput {
                    session_id: session.id.clone(),
                    scope: Some(scope),
                    target_range: input.target_range.clone(),
                    mining_policy: input.mining_policy,
                    checkpoint_policy: input.checkpoint_policy,
                })
                .await?;
            let readiness = readiness_summary(report);
            let proposals = self
                .list_planning_proposals(&session.id, session.scope.clone())
                .await?;
            let next_action = if readiness.ready_to_draft {
                "The planned range is ready. Call planning_decide with action=prepare_authoring_run, then start drafting only on the author's instruction.".to_string()
            } else {
                readiness
                    .prepare_run
                    .missing_requirements
                    .first()
                    .map(|missing| format!("Readiness is blocked: {missing}"))
                    .unwrap_or_else(|| review_next_action(&session, &proposals))
            };
            return Ok(PlanningAdvanceOutput {
                session: session_summary(&session, &proposals),
                action: input.action,
                advance: None,
                readiness: Some(readiness),
                next_action,
            });
        }

        let level = match input.action {
            PlanningAdvanceAction::RepairLastProposal => match input.level {
                Some(level) => Some(level),
                None => {
                    let attempts = self
                        .list_planning_generation_attempts(&session.id, session.scope.clone())
                        .await?;
                    attempts.last().map(|attempt| attempt.level)
                }
            },
            _ => input.level,
        };
        if input.action == PlanningAdvanceAction::RepairLastProposal && level.is_none() {
            anyhow::bail!("no previous planning attempt exists to repair; pass level explicitly");
        }
        let host_submission = match input.action {
            PlanningAdvanceAction::StageHostArtifact => Some(PlanningHostSubmission {
                source: PlanningOutputSource::Host,
                payload: input.host_artifact.clone().ok_or_else(|| {
                    anyhow::anyhow!("host_artifact is required for the stage_host_artifact action")
                })?,
            }),
            _ => {
                anyhow::ensure!(
                    input.host_artifact.is_none(),
                    "host_artifact is only accepted with the stage_host_artifact action"
                );
                None
            }
        };

        let report = self
            .advance_planning(AdvancePlanningInput {
                session_id: session.id.clone(),
                scope: session.scope.clone(),
                level,
                parents: input.parents.clone(),
                target_range: input.target_range.clone(),
                allow_review_backend: input.allow_review_backend,
                rating_ceiling: input.rating_ceiling.clone(),
                budget: input.budget.clone(),
                host_submission,
                retry_key: input.retry_key.clone(),
            })
            .await?;
        let next_action = match report.outcome {
            PlanningSessionOutcome::Staged => match report.proposal.as_ref() {
                Some(proposal) if proposal.status == PlanningProposalStatus::Staged => {
                    if proposal.artifact.author_questions.is_empty() {
                        format!(
                            "Review the staged {} proposal with the author, then call planning_decide with its payload_hash.",
                            proposal_level_name(proposal.artifact.level)
                        )
                    } else {
                        "The proposal carries unresolved author questions. Resolve them, then stage a replacement proposal.".to_string()
                    }
                }
                _ => "The proposal was accepted under the delegated gate. Continue with planning_advance or check planning_status.".to_string(),
            },
            PlanningSessionOutcome::NeedsRoute => {
                "No planning route could run. Copy host_request, generate the artifact with the host model, then call planning_advance with action=stage_host_artifact.".to_string()
            }
            PlanningSessionOutcome::NeedsAuthor => {
                "The attempt needs an author decision before acceptance. Review the proposal and author_questions, then decide or regenerate.".to_string()
            }
            PlanningSessionOutcome::ModelOutputRejected => {
                "The model output was rejected after the single allowed repair. Call planning_advance with action=repair_last_proposal and a new retry_key after adjusting the route or budget.".to_string()
            }
            PlanningSessionOutcome::BudgetExhausted => {
                "The planning budget is exhausted. Raise the session budget explicitly, then retry with a new retry_key.".to_string()
            }
        };
        let session = self.get_planning_session(&session.id).await?;
        let proposals = self
            .list_planning_proposals(&session.id, session.scope.clone())
            .await?;
        Ok(PlanningAdvanceOutput {
            session: session_summary(&session, &proposals),
            action: input.action,
            advance: Some(advance_summary(report)?),
            readiness: None,
            next_action,
        })
    }

    pub async fn planning_status(
        &self,
        input: PlanningStatusInput,
    ) -> Result<PlanningStatusOutput> {
        let include_readiness = input.include_readiness.unwrap_or(true);
        let (session, session_records) = match input.session_id.as_deref() {
            Some(session_id) => {
                let session = self.get_planning_session(session_id).await?;
                Self::verify_requested_scope(
                    &session,
                    input.project_id.as_deref(),
                    input.branch_id.as_deref(),
                )?;
                (Some(session), Vec::new())
            }
            None => {
                let sessions = self
                    .repository
                    .list_planning_sessions(input.project_id.clone(), input.branch_id.clone(), 20)
                    .await?;
                let selected = sessions.first().cloned();
                (selected, sessions)
            }
        };

        let mut sessions = Vec::with_capacity(session_records.len());
        for candidate in &session_records {
            let proposals = self
                .list_planning_proposals(&candidate.id, candidate.scope.clone())
                .await?;
            sessions.push(session_summary(candidate, &proposals));
        }

        let Some(session) = session else {
            return Ok(PlanningStatusOutput {
                session: None,
                sessions,
                gate: None,
                proposals: Vec::new(),
                findings: Vec::new(),
                readiness: None,
                next_action: "No planning session exists yet. Call planning_begin with the author's premise to start one.".to_string(),
                reconnect_hint: "Planning sessions are durable; call planning_status again after planning_begin.".to_string(),
            });
        };

        let proposals = self
            .list_planning_proposals(&session.id, session.scope.clone())
            .await?;
        if sessions.is_empty() {
            sessions = vec![session_summary(&session, &proposals)];
        }
        let proposal_summaries = proposals
            .iter()
            .map(proposal_summary)
            .collect::<Result<Vec<_>>>()?;
        let gate = Some(gate_for(&session, gate_level(&proposals)));

        let readiness = if include_readiness {
            match session.scope.clone() {
                Some(scope) => {
                    let target_range = input
                        .target_range
                        .clone()
                        .or_else(|| Some(session.horizon.clone()));
                    let report = self
                        .planning_readiness(PlanningReadinessInput {
                            session_id: session.id.clone(),
                            scope: Some(scope),
                            target_range,
                            mining_policy: None,
                            checkpoint_policy: None,
                        })
                        .await?;
                    Some(readiness_summary(report))
                }
                None => None,
            }
        } else {
            None
        };

        let mut findings: Vec<PlanningFinding> = proposals
            .iter()
            .filter(|p| p.status == PlanningProposalStatus::Staged)
            .flat_map(|p| p.findings.clone())
            .collect();
        if let Some(readiness) = &readiness {
            for finding in &readiness.findings {
                if !findings
                    .iter()
                    .any(|f| f.code == finding.code && f.path == finding.path)
                {
                    findings.push(finding.clone());
                }
            }
        }

        let next_action = if let Some(readiness) = &readiness {
            if readiness.ready_to_draft {
                "The planned range is ready. Call planning_decide with action=prepare_authoring_run, then start drafting only on the author's instruction.".to_string()
            } else {
                review_next_action(&session, &proposals)
            }
        } else {
            review_next_action(&session, &proposals)
        };

        Ok(PlanningStatusOutput {
            session: Some(session_summary(&session, &proposals)),
            sessions,
            gate,
            proposals: proposal_summaries,
            findings,
            readiness,
            next_action,
            reconnect_hint: reconnect_hint(&session),
        })
    }

    pub async fn planning_decide_entry(
        &self,
        input: PlanningDecideInput,
    ) -> Result<PlanningDecideOutput> {
        let session = self.get_planning_session(&input.session_id).await?;
        Self::verify_requested_scope(
            &session,
            input.project_id.as_deref(),
            input.branch_id.as_deref(),
        )?;

        if input.action == PlanningDecideAction::PrepareAuthoringRun {
            let scope = session.scope.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "prepare_authoring_run requires the session to be attached to an accepted project root"
                )
            })?;
            let report = self
                .planning_readiness(PlanningReadinessInput {
                    session_id: session.id.clone(),
                    scope: Some(scope),
                    target_range: input.target_range.clone(),
                    mining_policy: input.mining_policy,
                    checkpoint_policy: input.checkpoint_policy,
                })
                .await?;
            let readiness = readiness_summary(report);
            let next_action = if readiness.ready_to_draft {
                "Readiness passed. Start an authoring run only when the author asks to draft."
                    .to_string()
            } else {
                readiness
                    .prepare_run
                    .missing_requirements
                    .first()
                    .map(|missing| format!("Drafting is blocked: {missing}"))
                    .unwrap_or_else(|| {
                        "Drafting is blocked by planner findings; inspect the readiness findings."
                            .to_string()
                    })
            };
            let proposals = self
                .list_planning_proposals(&session.id, session.scope.clone())
                .await?;
            return Ok(PlanningDecideOutput {
                session: session_summary(&session, &proposals),
                action: input.action,
                decided: Vec::new(),
                readiness: Some(readiness),
                next_action,
                message: "Checked readiness with the real authoring prepare report.".to_string(),
            });
        }

        let decision = match input.action {
            PlanningDecideAction::Revise => PlanningDecision::Supersede,
            _ => input
                .decision
                .ok_or_else(|| anyhow::anyhow!("decision is required for the decide action"))?,
        };
        let actor = input
            .actor
            .as_deref()
            .map(str::trim)
            .filter(|actor| !actor.is_empty())
            .ok_or_else(|| anyhow::anyhow!("actor is required for the decide action"))?;
        let note = input
            .note
            .as_deref()
            .map(str::trim)
            .filter(|note| !note.is_empty())
            .ok_or_else(|| anyhow::anyhow!("note is required for the decide action"))?;
        anyhow::ensure!(
            !input.expected_hashes.is_empty(),
            "expected_hashes is required for the decide action"
        );
        let decided = self
            .decide_planning(DecidePlanningInput {
                session_id: session.id.clone(),
                scope: session.scope.clone(),
                expected_hashes: input.expected_hashes,
                decision,
                actor: actor.to_string(),
                note: note.to_string(),
            })
            .await?;
        let session = self.get_planning_session(&session.id).await?;
        let proposals = self
            .list_planning_proposals(&session.id, session.scope.clone())
            .await?;
        let decided_summaries = decided
            .iter()
            .map(proposal_summary)
            .collect::<Result<Vec<_>>>()?;
        let next_action = review_next_action(&session, &proposals);
        Ok(PlanningDecideOutput {
            session: session_summary(&session, &proposals),
            action: input.action,
            decided: decided_summaries,
            readiness: None,
            next_action,
            message: if input.action == PlanningDecideAction::Revise {
                "Superseded the selected proposals; stage a replacement at the same level with planning_advance.".to_string()
            } else {
                format!("Recorded planning decision {decision:?} for the selected proposals.")
            },
        })
    }
}
