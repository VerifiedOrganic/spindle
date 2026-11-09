//! Planning staging, dependency guards and durable application persistence.
use super::*;
use sha2::{Digest, Sha256};
use spindle_core::planning::*;
pub(super) mod apply;

fn hash<T: serde::Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn session(conn: &rusqlite::Connection, id: &str) -> Result<PlanningSession> {
    let data: String =
        conn.query_row("SELECT data FROM planning_session WHERE id=?1", [id], |r| {
            r.get(0)
        })?;
    Ok(serde_json::from_str(&data)?)
}
fn proposals(conn: &rusqlite::Connection, id: &str) -> Result<Vec<PlanningProposal>> {
    let mut stmt = conn
        .prepare("SELECT data FROM planning_proposal WHERE session_id=?1 ORDER BY created_at,id")?;
    let rows = stmt
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|data| Ok(serde_json::from_str(&data)?))
        .collect()
}
fn check_scope(conn: &rusqlite::Connection, scope: &Option<PlanningScope>) -> Result<()> {
    if let Some(scope) = scope {
        let matches: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM bible_branch WHERE id=?1 AND project_id=?2)",
            [&scope.branch_id, &scope.project_id],
            |r| r.get(0),
        )?;
        anyhow::ensure!(matches, "planning branch does not belong to project");
    }
    Ok(())
}
fn check_session(
    conn: &rusqlite::Connection,
    id: &str,
    scope: &Option<PlanningScope>,
) -> Result<PlanningSession> {
    let s = session(conn, id)?;
    anyhow::ensure!(&s.scope == scope, "planning scope mismatch");
    check_scope(conn, scope)?;
    Ok(s)
}
fn save(conn: &rusqlite::Connection, p: &PlanningProposal) -> Result<()> {
    let status = serde_json::to_value(p.status)?.as_str().unwrap().to_owned();
    conn.execute(
        "UPDATE planning_proposal SET status=?2,data=?3 WHERE id=?1",
        rusqlite::params![p.id, status, serde_json::to_string(p)?],
    )?;
    Ok(())
}

impl Repository {
    pub async fn begin_planning(&self, input: BeginPlanningInput) -> Result<PlanningSession> {
        anyhow::ensure!(
            !input.premise.trim().is_empty() && input.premise.len() <= 32_000,
            "premise must contain 1..32000 bytes"
        );
        let s = PlanningSession {
            id: mint_id("planning_session"),
            intent_hash: hash(&input.premise)?,
            premise: input.premise,
            scope: input.scope,
            policy: PlanningPolicy::default(),
            horizon: PlanningTargetRange::default(),
            budget: PlanningBudget::default(),
            usage: PlanningBudgetUsage::default(),
            rating_ceiling: None,
            allow_review_backend: false,
            projection_stale: false,
            stale_reason: None,
            last_outcome: None,
            created_at: timestamp_to_micros(chrono::Utc::now()),
        };
        self.pool()
            .write(move |conn| Ok(begin_in_transaction(conn, s)))
            .await?
    }

    pub async fn set_planning_policy(
        &self,
        input: SetPlanningPolicyInput,
    ) -> Result<PlanningSession> {
        anyhow::ensure!(
            !input.actor.trim().is_empty()
                && input.actor.len() <= 256
                && !input.actor.trim_start().starts_with("policy:"),
            "human actor required to ratify planning policy"
        );
        anyhow::ensure!(
            !input.note.trim().is_empty() && input.note.len() <= 8_000,
            "policy decision note required (max 8000 bytes)"
        );
        for (level, gate) in &input.policy.gates {
            anyhow::ensure!(
                !matches!(
                    level,
                    PlanningLevel::Contract
                        | PlanningLevel::CastWorld
                        | PlanningLevel::SeriesArc
                        | PlanningLevel::Book
                ) || *gate != PlanningGate::DelegateIfValid,
                "root planning level {level:?} requires human review and cannot use delegate_if_valid"
            );
        }
        let has_delegated = input
            .policy
            .gates
            .values()
            .any(|g| *g == PlanningGate::DelegateIfValid);
        if has_delegated {
            anyhow::ensure!(
                input.policy.delegated_actor().is_some(),
                "delegate_if_valid requires explicit non-empty policy_id and policy_version >= 1"
            );
        }
        if let Some(ref pov) = input.policy.pov_policy {
            if let Some(tol) = pov.share_tolerance {
                anyhow::ensure!(
                    !tol.is_nan() && (0.0..=1.0).contains(&tol),
                    "pov_policy.share_tolerance must be within [0.0, 1.0]"
                );
            }
            for val in pov.desired_shares.values() {
                anyhow::ensure!(
                    !val.is_nan() && (0.0..=1.0).contains(val),
                    "pov_policy.desired_shares values must be within [0.0, 1.0]"
                );
            }
            if let Some(max_abs) = pov.max_absence_chapters {
                anyhow::ensure!(
                    max_abs >= 0,
                    "pov_policy.max_absence_chapters must be non-negative"
                );
            }
            if let Some(max_consec) = pov.max_consecutive_chapters {
                anyhow::ensure!(
                    max_consec >= 1,
                    "pov_policy.max_consecutive_chapters must be >= 1"
                );
            }
        }
        self.pool()
            .write(move |conn| {
                Ok((|| -> Result<PlanningSession> {
                    let tx = conn.transaction()?;
                    let mut s = if let Some(ref req_scope) = input.scope {
                        check_session(&tx, &input.session_id, &Some(req_scope.clone()))?
                    } else {
                        let s = session(&tx, &input.session_id)?;
                        check_scope(&tx, &s.scope)?;
                        s
                    };
                    if has_delegated {
                        anyhow::ensure!(
                            s.scope.is_some(),
                            "delegate_if_valid requires the planning session to be attached to an accepted project root"
                        );
                        let all = proposals(&tx, &s.id)?;
                        let unaccepted_root = all.iter().any(|p| {
                            p.status == PlanningProposalStatus::Staged
                                && matches!(
                                    p.artifact.level,
                                    PlanningLevel::Contract
                                        | PlanningLevel::CastWorld
                                        | PlanningLevel::SeriesArc
                                        | PlanningLevel::Book
                                )
                        });
                        anyhow::ensure!(
                            !unaccepted_root,
                            "delegate_if_valid on Chapter/Scene requires upstream root proposals (Contract, CastWorld, SeriesArc, Book) to be human-accepted first"
                        );
                    }
                    s.policy = input.policy;
                    tx.execute(
                        "UPDATE planning_session SET data=?2 WHERE id=?1",
                        rusqlite::params![s.id, serde_json::to_string(&s)?],
                    )?;
                    let mut all = proposals(&tx, &s.id)?;
                    for p in &mut all {
                        if p.status == PlanningProposalStatus::Staged {
                            p.policy_snapshot = s.policy.clone();
                            p.findings = validate_stage_input_in_conn_excluding(
                                &tx,
                                &p.artifact,
                                None,
                                Some(&p.id),
                            )?;
                            save(&tx, p)?;
                        }
                    }
                    tx.commit()?;
                    Ok(s)
                })())
            })
            .await?
    }

    pub async fn get_planning_session(&self, id: &str) -> Result<PlanningSession> {
        let id = id.to_owned();
        self.pool().read(move |conn| Ok(session(conn, &id))).await?
    }

    /// List recent planning sessions for reconnect discovery. Passing no
    /// project returns sessions across all projects, newest first.
    pub async fn list_planning_sessions(
        &self,
        project_id: Option<String>,
        branch_id: Option<String>,
        limit: usize,
    ) -> Result<Vec<PlanningSession>> {
        anyhow::ensure!(
            limit > 0 && limit <= 100,
            "planning session limit out of range"
        );
        self.pool()
            .read(move |conn| {
                Ok((|| -> Result<Vec<PlanningSession>> {
                    let mut out = Vec::new();
                    match (project_id, branch_id) {
                        (Some(project_id), Some(branch_id)) => {
                            let mut stmt = conn.prepare(
                                "SELECT data FROM planning_session WHERE project_id=?1 AND branch_id=?2 ORDER BY created_at DESC, id DESC LIMIT ?3",
                            )?;
                            let rows = stmt.query_map(
                                rusqlite::params![project_id, branch_id, limit as i64],
                                |r| r.get::<_, String>(0),
                            )?;
                            for row in rows {
                                out.push(serde_json::from_str(&row?)?);
                            }
                        }
                        (Some(project_id), None) => {
                            let mut stmt = conn.prepare(
                                "SELECT data FROM planning_session WHERE project_id=?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
                            )?;
                            let rows = stmt.query_map(
                                rusqlite::params![project_id, limit as i64],
                                |r| r.get::<_, String>(0),
                            )?;
                            for row in rows {
                                out.push(serde_json::from_str(&row?)?);
                            }
                        }
                        (None, None) => {
                            let mut stmt = conn.prepare(
                                "SELECT data FROM planning_session ORDER BY created_at DESC, id DESC LIMIT ?1",
                            )?;
                            let rows = stmt.query_map(rusqlite::params![limit as i64], |r| {
                                r.get::<_, String>(0)
                            })?;
                            for row in rows {
                                out.push(serde_json::from_str(&row?)?);
                            }
                        }
                        (None, Some(_)) => {
                            anyhow::bail!("planning branch filter requires a project")
                        }
                    }
                    Ok(out)
                })())
            })
            .await?
    }

    /// Update session-level horizon, budget and route settings without
    /// generating a level. Never changes the accepted project scope.
    pub async fn configure_planning_session(
        &self,
        input: ConfigurePlanningSessionInput,
    ) -> Result<PlanningSession> {
        if let Some(horizon) = &input.horizon {
            anyhow::ensure!(
                horizon.book_number >= 1
                    && horizon.chapter_start >= 1
                    && horizon.chapter_end >= horizon.chapter_start
                    && horizon.scenes_per_chapter >= 1,
                "invalid planning target_range"
            );
        }
        self.pool()
            .write(move |conn| {
                Ok((|| -> Result<PlanningSession> {
                    let mut s = match input.scope.as_ref() {
                        Some(scope) => {
                            check_session(conn, &input.session_id, &Some(scope.clone()))?
                        }
                        None => {
                            let s = session(conn, &input.session_id)?;
                            check_scope(conn, &s.scope)?;
                            s
                        }
                    };
                    if let Some(horizon) = input.horizon {
                        s.horizon = horizon;
                    }
                    if let Some(budget) = input.budget {
                        s.budget = budget;
                    }
                    if let Some(rating_ceiling) = input.rating_ceiling {
                        s.rating_ceiling = Some(rating_ceiling);
                    }
                    if let Some(allow_review_backend) = input.allow_review_backend {
                        s.allow_review_backend = allow_review_backend;
                    }
                    conn.execute(
                        "UPDATE planning_session SET data=?2 WHERE id=?1",
                        rusqlite::params![s.id, serde_json::to_string(&s)?],
                    )?;
                    Ok(s)
                })())
            })
            .await?
    }

    pub async fn list_planning_proposals(
        &self,
        id: &str,
        scope: Option<PlanningScope>,
    ) -> Result<Vec<PlanningProposal>> {
        let id = id.to_owned();
        self.pool()
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    check_session(conn, &id, &scope)?;
                    proposals(conn, &id)
                })())
            })
            .await?
    }

    pub async fn stage_planning(&self, input: StagePlanningInput) -> Result<PlanningProposal> {
        anyhow::ensure!(
            input.schema_version == 1,
            "unsupported planning schema version"
        );
        anyhow::ensure!(
            !input.operations.is_empty() && input.operations.len() <= 128,
            "planning requires 1..128 operations"
        );
        anyhow::ensure!(
            !input.rationale.trim().is_empty() && input.rationale.len() <= 8_000,
            "rationale must contain 1..8000 bytes"
        );
        anyhow::ensure!(
            serde_json::to_vec(&input)?.len() <= 1_000_000,
            "planning artifact exceeds size limit"
        );
        self.pool()
            .write(move |conn| Ok(stage_in_transaction(conn, input)))
            .await?
    }

    /// Reject/supersede staging only; acceptance must use the service's atomic
    /// domain-write/receipt path. These decisions never mutate bible tables.
    pub async fn decide_planning(
        &self,
        input: DecidePlanningInput,
    ) -> Result<Vec<PlanningProposal>> {
        anyhow::ensure!(!input.expected_hashes.is_empty(), "empty planning decision");
        anyhow::ensure!(
            !input.actor.trim().is_empty()
                && input.actor.len() <= 256
                && !input.actor.trim_start().starts_with("policy:"),
            "human decision actor required"
        );
        anyhow::ensure!(
            !input.note.trim().is_empty() && input.note.len() <= 8000,
            "decision note required (max 8000 bytes)"
        );
        anyhow::ensure!(
            input.decision != PlanningDecision::Accept,
            "planning acceptance requires the service-level atomic apply path"
        );
        self.pool()
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    check_session(&tx, &input.session_id, &input.scope)?;
                    let mut all = proposals(&tx, &input.session_id)?;
                    // Preflight the entire batch before the first update.
                    for (id, expected) in &input.expected_hashes {
                        let p = all
                            .iter()
                            .find(|p| &p.id == id)
                            .context("proposal not in planning session")?;
                        anyhow::ensure!(
                            &p.payload_hash == expected
                                && p.status == PlanningProposalStatus::Staged,
                            "stale hash or proposal already decided"
                        );
                    }
                    let mut invalid: std::collections::BTreeSet<String> =
                        input.expected_hashes.keys().cloned().collect();
                    loop {
                        let before = invalid.len();
                        for p in &all {
                            if p.status == PlanningProposalStatus::Staged
                                && p.artifact.parents.keys().any(|id| invalid.contains(id))
                            {
                                invalid.insert(p.id.clone());
                            }
                        }
                        if invalid.len() == before {
                            break;
                        }
                    }
                    for id in &invalid {
                        let applied:i64=tx.query_row("SELECT COUNT(*) FROM planning_operation_receipt WHERE proposal_id=?1 AND outcome='applied'",[id],|r|r.get(0))?;
                        anyhow::ensure!(applied==0,"partially applied planning intent must be resumed or corrected by a new proposal, never rejected as unwritten");
                    }
                    let now = timestamp_to_micros(chrono::Utc::now());
                    let mut changed = Vec::new();
                    for p in &mut all {
                        if !invalid.contains(&p.id) {
                            continue;
                        }
                        p.status = if input.expected_hashes.contains_key(&p.id)
                            && input.decision == PlanningDecision::Reject
                        {
                            PlanningProposalStatus::Rejected
                        } else {
                            PlanningProposalStatus::Superseded
                        };
                        p.decided_by = Some(input.actor.clone());
                        p.decided_at = Some(now);
                        p.decision_note = Some(input.note.clone());
                        save(&tx, p)?;
                        changed.push(p.clone());
                    }
                    let mut s = session(&tx, &input.session_id)?;
                    if s.projection_stale
                        && input.decision == PlanningDecision::Supersede
                        && changed.iter().any(|p| {
                            p.artifact
                                .operations
                                .iter()
                                .any(|op| op.output_ref.starts_with("reconcile:"))
                        })
                    {
                        let remaining_recon = all.iter().any(|p| {
                            p.status == PlanningProposalStatus::Staged
                                && p.artifact
                                    .operations
                                    .iter()
                                    .any(|op| op.output_ref.starts_with("reconcile:"))
                        });
                        if !remaining_recon {
                            s.projection_stale = false;
                            s.stale_reason = None;
                            tx.execute(
                                "UPDATE planning_session SET data=?2 WHERE id=?1",
                                rusqlite::params![s.id, serde_json::to_string(&s)?],
                            )?;
                        }
                    }
                    tx.commit()?;
                    Ok(changed)
                })())
            })
            .await?
    }
}

fn check_level(level: PlanningLevel, op: &PlanningOperation, attached: bool) -> Result<()> {
    use PlanningOperation::*;
    let valid = match op {
        CreateProject(_) => level == PlanningLevel::Contract && !attached,
        UpdateReaderContract(_) => level == PlanningLevel::Contract && attached,
        CreateCharacter(_) | CreateLocation(_) | CreateWorldRule(_) | CreateRelationship(_) => {
            level == PlanningLevel::CastWorld
        }
        CreatePlotLine(_)
        | CreateConflict(_)
        | CreateTheme(_)
        | CreateMotif(_)
        | CreateNarrativePromise(_)
        | CreateCharacterArc(_)
        | CreatePacingConfig(_)
        | CreatePacingCurve(_)
        | SetArcPacingConstraints(_) => level == PlanningLevel::SeriesArc,
        CreateBook(_) | SetBookOutline(_) => level == PlanningLevel::Book,
        CreateChapter(_) | SetChapterOutline(_) => level == PlanningLevel::Chapter,
        PlanChapter(_) => matches!(level, PlanningLevel::Chapter | PlanningLevel::Scene),
    };
    anyhow::ensure!(valid, "operation not allowed for planning level/scope");
    let value = serde_json::to_value(op)?;
    fn no_realized(v: &Value) -> bool {
        match v {
            Value::Object(m) => m.iter().all(|(k, v)| match k.as_str() {
                "reached_at" | "established_in_scene_id" | "scene_id" => v.is_null(),
                "delivered" => v == false,
                _ => no_realized(v),
            }),
            Value::Array(a) => a.iter().all(no_realized),
            _ => true,
        }
    }
    anyhow::ensure!(
        no_realized(&value),
        "initial planning cannot assert realized events"
    );
    Ok(())
}

fn check_refs(
    conn: &rusqlite::Connection,
    v: &Value,
    available: &std::collections::BTreeMap<String, &str>,
    scope: &Option<PlanningScope>,
    expected: Option<&str>,
) -> Result<()> {
    match v {
        Value::Object(m) if m.get("kind").and_then(Value::as_str) == Some("proposal_ref") => {
            let key = m
                .get("value")
                .and_then(Value::as_str)
                .context("invalid proposal ref")?;
            anyhow::ensure!(
                available.contains_key(key),
                "unresolved proposal reference {key}; include its exact parent hash or create it earlier"
            );
            if let Some(expected) = expected {
                anyhow::ensure!(
                    available.get(key) == Some(&expected),
                    "proposal reference has wrong record type"
                );
            }
        }
        Value::Object(m) if m.get("kind").and_then(Value::as_str) == Some("existing_record_id") => {
            let id = m
                .get("value")
                .and_then(Value::as_str)
                .context("invalid record ref")?;
            let scope = scope
                .as_ref()
                .context("existing record requires attached project scope")?;
            let table = id.split(':').next().unwrap_or("");
            if let Some(expected) = expected {
                anyhow::ensure!(
                    table == expected,
                    "existing reference has wrong record type"
                );
            }
            anyhow::ensure!(
                [
                    "character",
                    "location",
                    "world_rule",
                    "relationship",
                    "plot_line",
                    "conflict",
                    "theme",
                    "motif",
                    "narrative_promise",
                    "character_arc",
                    "pacing_config",
                    "pacing_curve",
                    "book",
                    "chapter",
                    "faction"
                ]
                .contains(&table),
                "unsupported planning record type"
            );
            if matches!(table, "book" | "chapter") {
                let project: String = conn.query_row(
                    &format!("SELECT project_id FROM {table} WHERE id=?1"),
                    [id],
                    |r| r.get(0),
                )?;
                anyhow::ensure!(
                    project == scope.project_id,
                    "planning reference belongs to another project"
                );
                return Ok(());
            }
            let mut stmt = conn.prepare(&format!(
                "SELECT project_id,branch_id FROM {table} WHERE id=?1"
            ))?;
            let (project, branch): (String, String) =
                stmt.query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            anyhow::ensure!(
                project == scope.project_id,
                "planning reference belongs to another project"
            );
            let visible:bool=conn.query_row("WITH RECURSIVE lineage(id,parent_branch_id) AS (SELECT id,parent_branch_id FROM bible_branch WHERE id=?1 AND project_id=?2 UNION SELECT b.id,b.parent_branch_id FROM bible_branch b JOIN lineage l ON b.id=l.parent_branch_id WHERE b.project_id=?2) SELECT EXISTS(SELECT 1 FROM lineage WHERE id=?3)",rusqlite::params![scope.branch_id,scope.project_id,branch],|r|r.get(0))?;
            anyhow::ensure!(visible, "planning reference not visible on branch");
        }
        Value::Object(m) => {
            for (key, v) in m {
                let field_type = match key.as_str() {
                    "character_id" | "character_a_id" | "character_b_id" | "character_ids"
                    | "pov_character_id" => Some("character"),
                    "location_id" => Some("location"),
                    "connected_theme_ids" | "target_theme_ids" => Some("theme"),
                    "connected_conflict_ids" | "target_conflict_ids" => Some("conflict"),
                    "target_plot_line_ids" => Some("plot_line"),
                    "promise_ref" => Some("narrative_promise"),
                    "character_arc_id" => Some("character_arc"),
                    "book_id" => Some("book"),
                    "entity_id" => Some("chapter"),
                    "controlling_faction" => Some("faction"),
                    _ => None,
                };
                check_refs(conn, v, available, scope, field_type)?;
            }
        }
        Value::Array(a) => {
            for v in a {
                check_refs(conn, v, available, scope, expected)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn output_type(op: &PlanningOperation) -> &'static str {
    use PlanningOperation::*;
    match op {
        CreateProject(_) | UpdateReaderContract(_) => "project",
        CreateCharacter(_) => "character",
        CreateLocation(_) => "location",
        CreateWorldRule(_) => "world_rule",
        CreateRelationship(_) => "relationship",
        CreatePlotLine(_) => "plot_line",
        CreateConflict(_) => "conflict",
        CreateTheme(_) => "theme",
        CreateMotif(_) => "motif",
        CreateNarrativePromise(_) => "narrative_promise",
        CreateCharacterArc(_) => "character_arc",
        CreatePacingConfig(_) => "pacing_config",
        CreatePacingCurve(_) => "pacing_curve",
        SetArcPacingConstraints(_) => "pacing_tracker",
        CreateBook(_) => "book",
        CreateChapter(_) => "chapter",
        SetBookOutline(_) => "book_outline",
        SetChapterOutline(_) => "chapter_outline",
        PlanChapter(_) => "chapter_plan",
    }
}

fn stage_in_transaction(
    conn: &mut rusqlite::Connection,
    input: StagePlanningInput,
) -> Result<PlanningProposal> {
    let tx = conn.transaction()?;
    let s = check_session(&tx, &input.session_id, &input.scope)?;
    anyhow::ensure!(
        s.intent_hash == input.intent_hash,
        "stale planning intent hash"
    );
    let old = proposals(&tx, &s.id)?;
    for (id, expected) in &input.parents {
        let parent = old
            .iter()
            .find(|p| &p.id == id)
            .context("missing planning parent in session")?;
        anyhow::ensure!(
            &parent.payload_hash == expected
                && matches!(
                    parent.status,
                    PlanningProposalStatus::Staged | PlanningProposalStatus::Applied
                ),
            "stale or inactive planning parent"
        );
    }
    let mut available = std::collections::BTreeMap::new();
    let mut reserved = std::collections::BTreeSet::new();
    for p in &old {
        if matches!(
            p.status,
            PlanningProposalStatus::Staged | PlanningProposalStatus::Applied
        ) {
            for op in &p.artifact.operations {
                for (key, _) in output_refs(op) {
                    reserved.insert(key);
                }
                if input.parents.contains_key(&p.id) {
                    available.extend(output_refs(op));
                }
            }
        }
    }
    for item in &input.operations {
        anyhow::ensure!(
            !item.output_ref.is_empty()
                && item.output_ref.len() <= 128
                && item
                    .output_ref
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b)),
            "invalid proposal output reference"
        );
        anyhow::ensure!(
            !reserved.contains(&item.output_ref) && !available.contains_key(&item.output_ref),
            "duplicate output reference"
        );
        for (key, _) in output_refs(item) {
            anyhow::ensure!(
                !reserved.contains(&key) && !available.contains_key(&key),
                "duplicate output reference or root alias"
            );
        }
        check_level(input.level, &item.operation, s.scope.is_some())?;
        check_refs(
            &tx,
            &serde_json::to_value(&item.operation)?,
            &available,
            &s.scope,
            None,
        )?;
        available.extend(output_refs(item));
    }
    if !input.schedule.is_empty() {
        check_refs(
            &tx,
            &serde_json::to_value(&input.schedule)?,
            &available,
            &s.scope,
            None,
        )?;
    }
    let stage_findings = validate_stage_input_in_conn(&tx, &input, s.rating_ceiling.clone())?;
    let blocking: Vec<String> = stage_findings
        .iter()
        .filter(|f| f.severity == PlanningFindingSeverity::Blocking)
        .map(|f| format!("{}: {} (at {})", f.code, f.actual, f.path))
        .collect();
    anyhow::ensure!(
        blocking.is_empty(),
        "planning validation failed with blocking finding(s): {}",
        blocking.join("; ")
    );
    let captured_base_state = apply::base_state(&tx, &s.scope, &input)?;
    let p = PlanningProposal {
        id: mint_id("planning_proposal"),
        payload_hash: hash(&input)?,
        artifact: input,
        status: PlanningProposalStatus::Staged,
        policy_snapshot: s.policy,
        base_state: captured_base_state,
        generation_receipt: None,
        findings: stage_findings,
        decided_by: None,
        decision_note: None,
        decided_at: None,
        created_at: timestamp_to_micros(chrono::Utc::now()),
    };
    tx.execute("INSERT INTO planning_proposal(id,session_id,payload_hash,status,data,created_at) VALUES(?1,?2,?3,'staged',?4,?5)",rusqlite::params![p.id,p.artifact.session_id,p.payload_hash,serde_json::to_string(&p)?,p.created_at])?;
    tx.commit()?;
    Ok(p)
}

fn begin_in_transaction(
    conn: &mut rusqlite::Connection,
    s: PlanningSession,
) -> Result<PlanningSession> {
    let tx = conn.transaction()?;
    check_scope(&tx, &s.scope)?;
    tx.execute("INSERT INTO planning_session(id,project_id,branch_id,data,created_at) VALUES(?1,?2,?3,?4,?5)",rusqlite::params![s.id,s.scope.as_ref().map(|x| &x.project_id),s.scope.as_ref().map(|x| &x.branch_id),serde_json::to_string(&s)?,s.created_at])?;
    tx.commit()?;
    Ok(s)
}

pub(crate) fn output_refs(item: &PlanningOperationItem) -> Vec<(String, &'static str)> {
    let mut out = vec![(item.output_ref.clone(), output_type(&item.operation))];
    if matches!(item.operation, PlanningOperation::CreateProject(_)) {
        for (suffix, kind) in [
            ("book", "book"),
            ("chapter", "chapter"),
            ("branch", "bible_branch"),
        ] {
            out.push((format!("{}:{suffix}", item.output_ref), kind));
        }
    }
    if matches!(item.operation, PlanningOperation::CreateCharacterArc(_)) {
        out.push((
            format!("{}:pacing_tracker", item.output_ref),
            "pacing_tracker",
        ));
    }
    out
}

fn rating_rank(rating: &spindle_core::models::ContentRating) -> u8 {
    use spindle_core::models::ContentRating::*;
    match rating {
        General => 0,
        Teen => 1,
        Mature => 2,
        Explicit => 3,
    }
}

fn collect_ref_findings(
    conn: &rusqlite::Connection,
    v: &Value,
    path: &str,
    available: &std::collections::BTreeMap<String, &str>,
    scope: &Option<PlanningScope>,
    expected: Option<&str>,
    findings: &mut Vec<PlanningFinding>,
) {
    match v {
        Value::Object(m) if m.get("kind").and_then(Value::as_str) == Some("proposal_ref") => {
            let key = m.get("value").and_then(Value::as_str).unwrap_or("");
            if key.is_empty() || !available.contains_key(key) {
                findings.push(PlanningFinding {
                    code: "unresolved_proposal_ref".to_string(),
                    severity: PlanningFindingSeverity::Blocking,
                    path: path.to_string(),
                    related_refs: vec![key.to_string()],
                    expected: expected.unwrap_or("valid proposal_ref").to_string(),
                    actual: format!("proposal_ref:{key}"),
                    repair_hint: format!(
                        "Reference an output_ref declared in an accepted/staged parent proposal or an earlier operation in this artifact (available: {:?}).",
                        available.keys().collect::<Vec<_>>()
                    ),
                });
            } else if let Some(exp) = expected
                && available.get(key) != Some(&exp)
            {
                findings.push(PlanningFinding {
                    code: "wrong_proposal_ref_type".to_string(),
                    severity: PlanningFindingSeverity::Blocking,
                    path: path.to_string(),
                    related_refs: vec![key.to_string()],
                    expected: exp.to_string(),
                    actual: available.get(key).copied().unwrap_or("unknown").to_string(),
                    repair_hint: format!(
                        "Proposal reference '{key}' produces a '{}' record, but '{exp}' is required at {path}.",
                        available.get(key).copied().unwrap_or("unknown")
                    ),
                });
            }
        }
        Value::Object(m) if m.get("kind").and_then(Value::as_str) == Some("existing_record_id") => {
            let id = m.get("value").and_then(Value::as_str).unwrap_or("");
            if let Err(err) = check_refs(conn, v, available, scope, expected) {
                findings.push(PlanningFinding {
                    code: "invalid_existing_record_ref".to_string(),
                    severity: PlanningFindingSeverity::Blocking,
                    path: path.to_string(),
                    related_refs: vec![id.to_string()],
                    expected: expected.unwrap_or("existing visible record").to_string(),
                    actual: format!("{id} ({err})"),
                    repair_hint: "Use only existing_record_id values from the attached project/branch digest, or use proposal_ref for newly planned entities.".to_string(),
                });
            }
        }
        Value::Object(m) => {
            for (key, child) in m {
                let field_type = match key.as_str() {
                    "character_id" | "character_a_id" | "character_b_id" | "character_ids"
                    | "pov_character_id" => Some("character"),
                    "location_id" => Some("location"),
                    "connected_theme_ids" | "target_theme_ids" => Some("theme"),
                    "connected_conflict_ids" | "target_conflict_ids" => Some("conflict"),
                    "target_plot_line_ids" => Some("plot_line"),
                    "promise_ref" => Some("narrative_promise"),
                    "character_arc_id" => Some("character_arc"),
                    "book_id" => Some("book"),
                    "entity_id" => Some("chapter"),
                    "controlling_faction" => Some("faction"),
                    _ => None,
                };
                let child_path = format!("{path}/{key}");
                collect_ref_findings(
                    conn,
                    child,
                    &child_path,
                    available,
                    scope,
                    field_type,
                    findings,
                );
            }
        }
        Value::Array(a) => {
            for (idx, child) in a.iter().enumerate() {
                let child_path = format!("{path}/{idx}");
                collect_ref_findings(
                    conn,
                    child,
                    &child_path,
                    available,
                    scope,
                    expected,
                    findings,
                );
            }
        }
        _ => {}
    }
}

fn is_noop_scene_purpose(purpose: &str) -> bool {
    let norm = purpose
        .trim()
        .to_ascii_lowercase()
        .trim_matches(|c: char| c.is_ascii_punctuation())
        .trim()
        .to_string();
    matches!(
        norm.as_str(),
        "no state change"
            | "nothing changes"
            | "no change"
            | "status quo"
            | "none"
            | "n/a"
            | "tbd"
            | "todo"
            | "filler"
            | "placeholder"
    ) || norm.starts_with("no state change")
        || norm.starts_with("nothing changes")
}

fn act_breakpoint_stage_rank(key: &str) -> u8 {
    let k = key.trim().to_ascii_lowercase();
    match k.as_str() {
        "setup" | "opening" | "inciting" | "inciting_incident" | "act_1" | "act1" | "act1_end"
        | "act_1_end" | "first" | "1" => 1,
        "rising" | "midpoint" | "second" | "act_2" | "act2" | "2" => 2,
        "act2_end" | "act_2_end" | "dark_night" | "all_is_lost" | "third" | "act_3" | "act3"
        | "3" => 3,
        "climax" | "fourth" | "act_4" | "act4" | "4" => 4,
        "resolution" | "denouement" | "ending" | "fifth" | "5" => 5,
        _ => 10,
    }
}

fn interpolate_intensity(points: &[PlanningIntensityPoint], pos: f64) -> f64 {
    if points.is_empty() {
        return 0.5;
    }
    let mut sorted: Vec<(f64, f64)> = points.iter().map(|p| (p.position, p.intensity)).collect();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if pos <= sorted[0].0 {
        return sorted[0].1;
    }
    let last = sorted[sorted.len() - 1];
    if pos >= last.0 {
        return last.1;
    }
    for window in sorted.windows(2) {
        let (p0, v0) = window[0];
        let (p1, v1) = window[1];
        if pos >= p0 && pos <= p1 {
            let span = p1 - p0;
            if span.abs() <= 1e-12 {
                return v1;
            }
            let t = (pos - p0) / span;
            return v0 + t * (v1 - v0);
        }
    }
    last.1
}

#[derive(Debug, Clone)]
struct ValidatorPromiseDef {
    planted_at: PlanningStoryPlacement,
    planned_payoff: Option<PlanningStoryPlacement>,
}

#[derive(Debug, Clone, Default)]
struct ValidatorThreadLinks {
    themes: std::collections::BTreeSet<String>,
    conflicts: std::collections::BTreeSet<String>,
}

#[derive(Debug, Clone, Default)]
struct ValidatorContext {
    id_to_ref: std::collections::BTreeMap<String, String>,
    project_genre: Option<String>,
    project_type: Option<String>,
    reader_contract: Option<PlanningReaderContract>,
    style_world_rules: Vec<spindle_core::style::StyleRule>,
    promises: std::collections::BTreeMap<String, ValidatorPromiseDef>,
    plot_lines: std::collections::BTreeMap<String, ValidatorThreadLinks>,
    character_arcs: std::collections::BTreeMap<String, ValidatorThreadLinks>,
    arc_milestones: std::collections::BTreeMap<String, Vec<PlanningStoryPlacement>>,
    pacing_curves: std::collections::BTreeMap<i32, PlanningCreatePacingCurveInput>,
    ancestor_promise_events: Vec<PlannedPromiseEvent>,
    ancestor_pov_plan: Vec<PlannedPov>,
    ancestor_intensity_targets: Vec<PlannedIntensity>,
    chapter_povs: std::collections::BTreeMap<(i32, i32), String>,
}

impl ValidatorContext {
    fn normalize_str(&self, raw: &str) -> String {
        self.id_to_ref
            .get(raw)
            .cloned()
            .unwrap_or_else(|| raw.to_string())
    }

    fn normalize_ref(&self, r: &PlanningRef) -> String {
        match r {
            PlanningRef::ProposalRef(k) => k.clone(),
            PlanningRef::ExistingRecordId(id) => self.normalize_str(id),
        }
    }

    fn ingest_operation(&mut self, output_ref: &str, op: &PlanningOperation) {
        let norm_out = self.normalize_str(output_ref);
        match op {
            PlanningOperation::CreateProject(proj) => {
                self.project_genre = Some(proj.genre.clone());
                self.project_type = Some(proj.project_type.clone());
                self.reader_contract = Some(proj.reader_contract.clone());
            }
            PlanningOperation::UpdateReaderContract(rc) => {
                self.reader_contract = Some(rc.clone());
            }
            PlanningOperation::CreateWorldRule(wr) => {
                if wr.rule_type.eq_ignore_ascii_case("style") {
                    self.style_world_rules.push(spindle_core::style::StyleRule {
                        rule_name: wr.rule_name.clone(),
                        description: wr.description.clone(),
                    });
                }
            }
            PlanningOperation::CreateNarrativePromise(np) => {
                self.promises.insert(
                    norm_out,
                    ValidatorPromiseDef {
                        planted_at: np.planted_at.clone(),
                        planned_payoff: np.planned_payoff.clone(),
                    },
                );
            }
            PlanningOperation::CreatePlotLine(pl) => {
                let themes = pl
                    .connected_theme_ids
                    .iter()
                    .map(|r| self.normalize_ref(r))
                    .collect();
                let conflicts = pl
                    .connected_conflict_ids
                    .iter()
                    .map(|r| self.normalize_ref(r))
                    .collect();
                self.plot_lines
                    .insert(norm_out, ValidatorThreadLinks { themes, conflicts });
            }
            PlanningOperation::CreateCharacterArc(arc) => {
                let char_key = self.normalize_ref(&arc.character_id);
                let themes: std::collections::BTreeSet<String> = arc
                    .connected_theme_ids
                    .iter()
                    .map(|r| self.normalize_ref(r))
                    .collect();
                let entry = self.character_arcs.entry(char_key).or_default();
                entry.themes.extend(themes);
                let placements = arc
                    .milestones
                    .iter()
                    .filter_map(|m| m.placement.clone())
                    .collect();
                self.arc_milestones.insert(norm_out, placements);
            }
            PlanningOperation::CreatePacingCurve(curve) => {
                self.pacing_curves.insert(curve.book_number, curve.clone());
            }
            PlanningOperation::PlanChapter(plan) => {
                if let Some(ref pov) = plan.pov_character_id {
                    self.chapter_povs.insert(
                        (plan.book_number, plan.chapter_number),
                        self.normalize_ref(pov),
                    );
                }
            }
            _ => {}
        }
    }

    fn load_from_db(&mut self, conn: &rusqlite::Connection, scope: &PlanningScope) -> Result<()> {
        if let Ok((genre, project_type, rc_json)) = conn.query_row(
            "SELECT genre, project_type, reader_contract FROM project WHERE id=?1",
            [&scope.project_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        ) {
            if self.project_genre.is_none() {
                self.project_genre = Some(genre);
            }
            if self.project_type.is_none() {
                self.project_type = Some(project_type);
            }
            if self.reader_contract.is_none()
                && let Ok(rc) = serde_json::from_str::<PlanningReaderContract>(&rc_json)
            {
                self.reader_contract = Some(rc);
            }
        }
        let lineage =
            resolve_branch_lineage_in_conn(conn, Some(&scope.project_id), &scope.branch_id)?;
        for branch_id in lineage.iter().rev() {
            let mut wr_stmt = conn.prepare(
                "SELECT rule_name, rule_type, description FROM world_rule WHERE project_id=?1 AND branch_id=?2",
            )?;
            let wr_rows = wr_stmt
                .query_map(rusqlite::params![scope.project_id, branch_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (rule_name, rule_type, description) in wr_rows {
                if rule_type.eq_ignore_ascii_case("style") {
                    self.style_world_rules.push(spindle_core::style::StyleRule {
                        rule_name,
                        description,
                    });
                }
            }

            let mut np_stmt = conn.prepare(
                "SELECT id, planted_at, planned_payoff FROM narrative_promise WHERE project_id=?1 AND branch_id=?2",
            )?;
            let np_rows = np_stmt
                .query_map(rusqlite::params![scope.project_id, branch_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, planted_json, payoff_json) in np_rows {
                if let Ok(planted_at) =
                    serde_json::from_str::<PlanningStoryPlacement>(&planted_json)
                {
                    let planned_payoff = payoff_json
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<PlanningStoryPlacement>(s).ok());
                    let key = self.normalize_str(&id);
                    self.promises.entry(key).or_insert(ValidatorPromiseDef {
                        planted_at,
                        planned_payoff,
                    });
                }
            }

            let mut pl_stmt = conn.prepare(
                "SELECT id, connected_theme_ids, connected_conflict_ids FROM plot_line WHERE project_id=?1 AND branch_id=?2",
            )?;
            let pl_rows = pl_stmt
                .query_map(rusqlite::params![scope.project_id, branch_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, themes_json, conflicts_json) in pl_rows {
                let raw_themes: Vec<String> =
                    serde_json::from_str(&themes_json).unwrap_or_default();
                let raw_conflicts: Vec<String> =
                    serde_json::from_str(&conflicts_json).unwrap_or_default();
                let themes = raw_themes.iter().map(|s| self.normalize_str(s)).collect();
                let conflicts = raw_conflicts
                    .iter()
                    .map(|s| self.normalize_str(s))
                    .collect();
                let key = self.normalize_str(&id);
                self.plot_lines
                    .entry(key)
                    .or_insert(ValidatorThreadLinks { themes, conflicts });
            }

            let mut arc_stmt = conn.prepare(
                "SELECT id, character_id, milestones, connected_theme_ids FROM character_arc WHERE project_id=?1 AND branch_id=?2",
            )?;
            let arc_rows = arc_stmt
                .query_map(rusqlite::params![scope.project_id, branch_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, char_id, milestones_json, themes_json) in arc_rows {
                let char_key = self.normalize_str(&char_id);
                let raw_themes: Vec<String> =
                    serde_json::from_str(&themes_json).unwrap_or_default();
                let norm_themes: Vec<String> =
                    raw_themes.iter().map(|s| self.normalize_str(s)).collect();
                let entry = self.character_arcs.entry(char_key).or_default();
                entry.themes.extend(norm_themes);
                if let Ok(ms) =
                    serde_json::from_str::<Vec<PlanningCharacterArcMilestone>>(&milestones_json)
                {
                    let key = self.normalize_str(&id);
                    self.arc_milestones
                        .entry(key)
                        .or_insert_with(|| ms.into_iter().filter_map(|m| m.placement).collect());
                }
            }

            let mut curve_stmt = conn.prepare(
                "SELECT book_number, act_breakpoints, scene_type_density, intensity_points FROM pacing_curve WHERE project_id=?1 AND branch_id=?2",
            )?;
            let curve_rows = curve_stmt
                .query_map(rusqlite::params![scope.project_id, branch_id], |r| {
                    Ok((
                        r.get::<_, i32>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (book_no, ab_json, std_json, ip_json) in curve_rows {
                let act_breakpoints = serde_json::from_str(&ab_json).unwrap_or_default();
                let scene_type_density = serde_json::from_str(&std_json).unwrap_or_default();
                let intensity_points = serde_json::from_str(&ip_json).unwrap_or_default();
                self.pacing_curves
                    .entry(book_no)
                    .or_insert(PlanningCreatePacingCurveInput {
                        book_number: book_no,
                        act_breakpoints,
                        scene_type_density,
                        intensity_points,
                    });
            }

            let mut cp_stmt = conn.prepare(
                "SELECT book_number, chapter_number, pov_character_id FROM chapter_plan WHERE project_id=?1 AND branch_id=?2",
            )?;
            let cp_rows = cp_stmt
                .query_map(rusqlite::params![scope.project_id, branch_id], |r| {
                    Ok((
                        r.get::<_, i32>(0)?,
                        r.get::<_, i32>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (book_no, ch_no, pov_opt) in cp_rows {
                if let Some(pov) = pov_opt {
                    let norm_pov = self.normalize_str(&pov);
                    self.chapter_povs
                        .entry((book_no, ch_no))
                        .or_insert(norm_pov);
                }
            }
        }
        Ok(())
    }
}

fn extract_forbidden_boundary_phrases(boundaries: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in boundaries {
        let lower = raw.trim().to_ascii_lowercase();
        for prefix in ["no ", "avoid ", "forbid ", "never "] {
            if let Some(rest) = lower.strip_prefix(prefix) {
                let phrase = rest
                    .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
                    .to_string();
                if phrase.len() >= 3
                    && !matches!(
                        phrase.as_str(),
                        "explicit" | "mature" | "teen" | "general" | "rating"
                    )
                {
                    out.push((raw.clone(), phrase));
                }
            }
        }
    }
    out
}

fn boundary_rating_cap(boundaries: &[String]) -> Option<spindle_core::models::ContentRating> {
    use spindle_core::models::ContentRating;
    let mut cap: Option<ContentRating> = None;
    let mut tighten = |candidate: ContentRating| {
        if cap
            .as_ref()
            .is_none_or(|cur| rating_rank(&candidate) < rating_rank(cur))
        {
            cap = Some(candidate);
        }
    };
    for raw in boundaries {
        let lower = raw.trim().to_ascii_lowercase();
        if lower.contains("general only")
            || lower.contains("max rating: general")
            || lower.contains("rating ceiling: general")
        {
            tighten(ContentRating::General);
        } else if lower.contains("no mature")
            || lower.contains("avoid mature")
            || lower.contains("max rating: teen")
            || lower.contains("rating ceiling: teen")
        {
            tighten(ContentRating::Teen);
        } else if lower.contains("no explicit")
            || lower.contains("avoid explicit")
            || lower.contains("max rating: mature")
            || lower.contains("rating ceiling: mature")
        {
            tighten(ContentRating::Mature);
        }
    }
    cap
}

fn compute_pov_and_pacing_metrics(
    ctx: &ValidatorContext,
    policy: &PlanningPolicy,
    target_book: i32,
    chapter_povs: &std::collections::BTreeMap<(i32, i32), String>,
    intensity_targets: &[PlannedIntensity],
    findings: &mut Vec<PlanningFinding>,
    emit_policy_findings: bool,
) -> PlanningPacingMetrics {
    let mut metrics = PlanningPacingMetrics::default();
    let book_povs: Vec<(i32, &String)> = chapter_povs
        .iter()
        .filter_map(|((b, c), pov)| (*b == target_book).then_some((*c, pov)))
        .collect();
    let total_pov_chapters = book_povs.len();
    if total_pov_chapters > 0 {
        let mut tracked_keys: std::collections::BTreeSet<String> =
            book_povs.iter().map(|(_, p)| (*p).clone()).collect();
        if let Some(ref pov_policy) = policy.pov_policy {
            if let Some(ref single) = pov_policy.single_pov {
                tracked_keys.insert(ctx.normalize_ref(single));
            }
            for r in &pov_policy.allowed_roster {
                tracked_keys.insert(ctx.normalize_ref(r));
            }
            for k in pov_policy.desired_shares.keys() {
                tracked_keys.insert(ctx.normalize_str(k));
            }
        }
        for (_, pov) in &book_povs {
            *metrics
                .pov_chapter_counts
                .entry((*pov).clone())
                .or_insert(0) += 1;
        }
        for key in &tracked_keys {
            let count = metrics.pov_chapter_counts.get(key).copied().unwrap_or(0);
            metrics
                .pov_shares
                .insert(key.clone(), (count as f64) / (total_pov_chapters as f64));
            let mut max_consec = 0i32;
            let mut cur_consec = 0i32;
            let mut max_absence = 0i32;
            let mut cur_absence = 0i32;
            for (_, actual_pov) in &book_povs {
                if *actual_pov == key {
                    cur_consec += 1;
                    max_consec = max_consec.max(cur_consec);
                    cur_absence = 0;
                } else {
                    cur_absence += 1;
                    max_absence = max_absence.max(cur_absence);
                    cur_consec = 0;
                }
            }
            metrics
                .pov_max_consecutive_chapters
                .insert(key.clone(), max_consec);
            metrics
                .pov_max_absence_chapters
                .insert(key.clone(), max_absence);
        }

        if emit_policy_findings && let Some(ref pov_policy) = policy.pov_policy {
            let severity = if pov_policy.hard_constraint {
                PlanningFindingSeverity::Blocking
            } else {
                PlanningFindingSeverity::Advisory
            };
            if let Some(ref single_ref) = pov_policy.single_pov {
                let expected_pov = ctx.normalize_ref(single_ref);
                for (ch_no, actual_pov) in &book_povs {
                    if **actual_pov != expected_pov {
                        findings.push(PlanningFinding {
                            code: "pov_single_constraint_violated".to_string(),
                            severity,
                            path: "/schedule/pov_plan".to_string(),
                            related_refs: vec![(*actual_pov).clone()],
                            expected: expected_pov.clone(),
                            actual: format!("book {target_book} chapter {ch_no}: {actual_pov}"),
                            repair_hint: format!(
                                "Assign all chapters to single_pov character '{expected_pov}'."
                            ),
                        });
                    }
                }
            }
            if !pov_policy.allowed_roster.is_empty() {
                let allowed: std::collections::BTreeSet<String> = pov_policy
                    .allowed_roster
                    .iter()
                    .map(|r| ctx.normalize_ref(r))
                    .collect();
                for (ch_no, actual_pov) in &book_povs {
                    if !allowed.contains(*actual_pov) {
                        findings.push(PlanningFinding {
                            code: "pov_not_in_allowed_roster".to_string(),
                            severity,
                            path: "/schedule/pov_plan".to_string(),
                            related_refs: vec![(*actual_pov).clone()],
                            expected: format!("one of {:?}", allowed),
                            actual: format!("book {target_book} chapter {ch_no}: {actual_pov}"),
                            repair_hint:
                                "Use only POV characters listed in pov_policy.allowed_roster."
                                    .to_string(),
                        });
                    }
                }
            }
            if let Some(max_consec) = pov_policy.max_consecutive_chapters {
                for (pov_key, &consec) in &metrics.pov_max_consecutive_chapters {
                    if consec > max_consec {
                        findings.push(PlanningFinding {
                            code: "pov_repetition_window_exceeded".to_string(),
                            severity,
                            path: "/schedule/pov_plan".to_string(),
                            related_refs: vec![pov_key.clone()],
                            expected: format!("<= {max_consec} consecutive chapters"),
                            actual: format!("{consec} consecutive chapters for {pov_key}"),
                            repair_hint: format!(
                                "Rotate POV so '{pov_key}' does not appear in more than {max_consec} consecutive chapters."
                            ),
                        });
                    }
                }
            }
            if let Some(max_absence) = pov_policy.max_absence_chapters {
                for (pov_key, &absence) in &metrics.pov_max_absence_chapters {
                    if absence > max_absence {
                        findings.push(PlanningFinding {
                            code: "pov_absence_window_exceeded".to_string(),
                            severity,
                            path: "/schedule/pov_plan".to_string(),
                            related_refs: vec![pov_key.clone()],
                            expected: format!("<= {max_absence} chapters absent"),
                            actual: format!("{absence} chapters absent for {pov_key}"),
                            repair_hint: format!(
                                "Schedule a POV chapter for '{pov_key}' within every {max_absence}-chapter window."
                            ),
                        });
                    }
                }
            }
            if !pov_policy.desired_shares.is_empty() {
                let tolerance = pov_policy.share_tolerance.unwrap_or(0.15);
                for (raw_key, &target_share) in &pov_policy.desired_shares {
                    let pov_key = ctx.normalize_str(raw_key);
                    let actual_share = metrics
                        .pov_shares
                        .get(&pov_key)
                        .copied()
                        .or_else(|| metrics.pov_shares.get(raw_key).copied())
                        .unwrap_or(0.0);
                    if (actual_share - target_share).abs() > tolerance + 1e-9 {
                        findings.push(PlanningFinding {
                            code: "pov_share_deviation".to_string(),
                            severity,
                            path: "/schedule/pov_plan".to_string(),
                            related_refs: vec![pov_key.clone()],
                            expected: format!("{target_share:.2} +/- {tolerance:.2}"),
                            actual: format!("{actual_share:.2}"),
                            repair_hint: format!(
                                "Adjust chapter POV assignments for '{pov_key}' to stay within {tolerance:.2} of desired share {target_share:.2}."
                            ),
                        });
                    }
                }
            }
        }
    }

    let mut book_targets: Vec<&PlannedIntensity> = intensity_targets
        .iter()
        .filter(|t| t.book_number == target_book && (0.0..=1.0).contains(&t.target_intensity))
        .collect();
    book_targets.sort_by_key(|t| (t.book_number, t.chapter_number));
    if let Some(curve) = ctx.pacing_curves.get(&target_book)
        && !curve.intensity_points.is_empty()
        && !book_targets.is_empty()
    {
        let n = book_targets.len();
        let mut sum_abs_dev = 0.0;
        let mut cur_sustained = 0usize;
        let mut max_sustained = 0usize;
        for (idx, target) in book_targets.iter().enumerate() {
            let norm_pos = if n <= 1 {
                0.0
            } else {
                (idx as f64) / ((n - 1) as f64)
            };
            let expected = interpolate_intensity(&curve.intensity_points, norm_pos);
            let dev = (target.target_intensity - expected).abs();
            sum_abs_dev += dev;
            if dev > 0.20 + 1e-9 {
                cur_sustained += 1;
                max_sustained = max_sustained.max(cur_sustained);
            } else {
                cur_sustained = 0;
            }
        }
        metrics.evaluated_items = n;
        metrics.mean_absolute_deviation = sum_abs_dev / (n as f64);
        metrics.longest_sustained_deviation = max_sustained;
        if emit_policy_findings && max_sustained >= 3 {
            let policy_waived = policy
                .pacing_policy
                .as_ref()
                .and_then(|p| p.waiver_reason.as_deref())
                .is_some_and(|w| !w.trim().is_empty());
            let target_waived = book_targets.iter().any(|t| {
                t.note
                    .as_deref()
                    .is_some_and(|n| n.to_ascii_lowercase().contains("waiver"))
            });
            let hard = policy
                .pacing_policy
                .as_ref()
                .is_some_and(|p| p.hard_constraint)
                && !policy_waived
                && !target_waived;
            findings.push(PlanningFinding {
                code: "pacing_curve_sustained_deviation".to_string(),
                severity: if hard {
                    PlanningFindingSeverity::Blocking
                } else {
                    PlanningFindingSeverity::Advisory
                },
                path: "/schedule/intensity_targets".to_string(),
                related_refs: Vec::new(),
                expected: "deviation <= 0.20 (sustained < 3 consecutive items)".to_string(),
                actual: format!(
                    "longest_sustained_deviation={}, mean_absolute_deviation={:.3}",
                    metrics.longest_sustained_deviation, metrics.mean_absolute_deviation
                ),
                repair_hint: "Align planned intensity_targets with the accepted PacingCurve intensity_points or provide an explicit pacing waiver.".to_string(),
            });
        }
    }
    metrics
}

fn validate_stage_input_in_conn(
    conn: &rusqlite::Connection,
    input: &StagePlanningInput,
    rating_ceiling: Option<spindle_core::models::ContentRating>,
) -> Result<Vec<PlanningFinding>> {
    validate_stage_input_in_conn_excluding(conn, input, rating_ceiling, None)
}

fn validate_stage_input_in_conn_excluding(
    conn: &rusqlite::Connection,
    input: &StagePlanningInput,
    rating_ceiling: Option<spindle_core::models::ContentRating>,
    exclude_proposal_id: Option<&str>,
) -> Result<Vec<PlanningFinding>> {
    let s = if exclude_proposal_id.is_some() {
        let s = session(conn, &input.session_id)?;
        check_scope(conn, &s.scope)?;
        if input.scope.is_some() {
            anyhow::ensure!(s.scope == input.scope, "planning scope mismatch");
        }
        s
    } else {
        check_session(conn, &input.session_id, &input.scope)?
    };
    let effective_ceiling = rating_ceiling.or(s.rating_ceiling);
    let mut findings = Vec::new();
    if s.intent_hash != input.intent_hash {
        findings.push(PlanningFinding {
            code: "stale_intent_hash".to_string(),
            severity: PlanningFindingSeverity::Blocking,
            path: "/intent_hash".to_string(),
            related_refs: Vec::new(),
            expected: s.intent_hash.clone(),
            actual: input.intent_hash.clone(),
            repair_hint: "Use the current session intent_hash.".to_string(),
        });
    }
    if input.schema_version != 1 {
        findings.push(PlanningFinding {
            code: "invalid_schema_version".to_string(),
            severity: PlanningFindingSeverity::Blocking,
            path: "/schema_version".to_string(),
            related_refs: Vec::new(),
            expected: "1".to_string(),
            actual: input.schema_version.to_string(),
            repair_hint: "Set schema_version to 1.".to_string(),
        });
    }
    if input.operations.is_empty() || input.operations.len() > 128 {
        findings.push(PlanningFinding {
            code: "invalid_operation_count".to_string(),
            severity: PlanningFindingSeverity::Blocking,
            path: "/operations".to_string(),
            related_refs: Vec::new(),
            expected: "1..=128 operations".to_string(),
            actual: input.operations.len().to_string(),
            repair_hint: "Include between 1 and 128 operations for the requested level."
                .to_string(),
        });
    }
    if input.rationale.trim().is_empty() || input.rationale.len() > 8_000 {
        findings.push(PlanningFinding {
            code: "invalid_rationale".to_string(),
            severity: PlanningFindingSeverity::Blocking,
            path: "/rationale".to_string(),
            related_refs: Vec::new(),
            expected: "1..=8000 non-whitespace bytes".to_string(),
            actual: format!("{} bytes", input.rationale.trim().len()),
            repair_hint: "Provide a non-empty rationale up to 8000 bytes.".to_string(),
        });
    }
    if let Some(ref tr) = input.target_range
        && (tr.book_number < 1
            || tr.chapter_start < 1
            || tr.chapter_end < tr.chapter_start
            || tr.scenes_per_chapter < 1)
    {
        findings.push(PlanningFinding {
            code: "invalid_target_range".to_string(),
            severity: PlanningFindingSeverity::Blocking,
            path: "/target_range".to_string(),
            related_refs: Vec::new(),
            expected: "positive book_number, chapter_start <= chapter_end, scenes_per_chapter >= 1"
                .to_string(),
            actual: format!("{tr:?}"),
            repair_hint: "Specify positive book/chapter bounds and scenes_per_chapter >= 1."
                .to_string(),
        });
    }
    let old = proposals(conn, &s.id)?;
    let mut excluded_ids = std::collections::BTreeSet::new();
    if let Some(ex_id) = exclude_proposal_id {
        excluded_ids.insert(ex_id.to_string());
        loop {
            let before = excluded_ids.len();
            for p in &old {
                if p.artifact
                    .parents
                    .keys()
                    .any(|pid| excluded_ids.contains(pid))
                {
                    excluded_ids.insert(p.id.clone());
                }
            }
            if excluded_ids.len() == before {
                break;
            }
        }
    }
    let creates_project = input
        .operations
        .iter()
        .any(|op| matches!(op.operation, PlanningOperation::CreateProject(_)));
    if input.level == PlanningLevel::Contract
        && creates_project
        && old.iter().any(|p| {
            !excluded_ids.contains(&p.id)
                && p.artifact.level == PlanningLevel::Contract
                && p.status == PlanningProposalStatus::Applied
        })
    {
        findings.push(PlanningFinding {
            code: "accepted_contract_root_immutable".to_string(),
            severity: PlanningFindingSeverity::Blocking,
            path: "/level".to_string(),
            related_refs: Vec::new(),
            expected: "unaccepted contract root".to_string(),
            actual: "contract root already applied".to_string(),
            repair_hint:
                "Cannot stage or regenerate a create_project contract proposal after the root contract has been applied."
                    .to_string(),
        });
    }
    for (id, expected) in &input.parents {
        match old.iter().find(|p| &p.id == id) {
            None => findings.push(PlanningFinding {
                code: "missing_planning_parent".to_string(),
                severity: PlanningFindingSeverity::Blocking,
                path: format!("/parents/{id}"),
                related_refs: vec![id.clone()],
                expected: "existing parent proposal in session".to_string(),
                actual: "missing".to_string(),
                repair_hint: "Reference only parent proposals that exist in this planning session."
                    .to_string(),
            }),
            Some(parent)
                if &parent.payload_hash != expected
                    || !matches!(
                        parent.status,
                        PlanningProposalStatus::Staged | PlanningProposalStatus::Applied
                    ) =>
            {
                findings.push(PlanningFinding {
                    code: "stale_or_inactive_planning_parent".to_string(),
                    severity: PlanningFindingSeverity::Blocking,
                    path: format!("/parents/{id}"),
                    related_refs: vec![id.clone()],
                    expected: format!("active (staged/applied) parent with hash {expected}"),
                    actual: format!("status={:?}, hash={}", parent.status, parent.payload_hash),
                    repair_hint:
                        "Re-select active parent proposals; a parent proposal was rejected, superseded, or modified."
                            .to_string(),
                });
            }
            Some(_) => {}
        }
    }

    // Build transitive ancestor set for semantic context resolution while keeping
    // `available` strictly bound to `input.parents` for reference checks.
    let mut ancestor_ids: std::collections::BTreeSet<String> =
        input.parents.keys().cloned().collect();
    for p in &old {
        if !excluded_ids.contains(&p.id) && p.status == PlanningProposalStatus::Applied {
            ancestor_ids.insert(p.id.clone());
        }
    }
    loop {
        let before = ancestor_ids.len();
        for p in &old {
            if ancestor_ids.contains(&p.id) {
                for parent_id in p.artifact.parents.keys() {
                    ancestor_ids.insert(parent_id.clone());
                }
            }
        }
        if ancestor_ids.len() == before {
            break;
        }
    }

    let receipt_refs = apply::refs_in_conn(conn, &s.id).unwrap_or_default();
    let mut ctx = ValidatorContext::default();
    for (out_ref, db_id) in &receipt_refs {
        ctx.id_to_ref.insert(db_id.clone(), out_ref.clone());
    }
    if let Some(ref scope) = s.scope {
        let _ = ctx.load_from_db(conn, scope);
    }

    let mut available = std::collections::BTreeMap::new();
    let mut reserved = std::collections::BTreeSet::new();
    let mut ancestor_outlined_chapters = std::collections::BTreeSet::new();
    for p in &old {
        if excluded_ids.contains(&p.id) {
            continue;
        }
        if matches!(
            p.status,
            PlanningProposalStatus::Staged | PlanningProposalStatus::Applied
        ) {
            for op in &p.artifact.operations {
                for (key, _) in output_refs(op) {
                    reserved.insert(key);
                }
                if input.parents.contains_key(&p.id) {
                    available.extend(output_refs(op));
                }
                if ancestor_ids.contains(&p.id) {
                    ctx.ingest_operation(&op.output_ref, &op.operation);
                    match &op.operation {
                        PlanningOperation::CreateChapter(ch) => {
                            if let Some(ch_no) = ch.chapter_number {
                                ancestor_outlined_chapters
                                    .insert((ch.book_number.unwrap_or(1), ch_no));
                            }
                        }
                        PlanningOperation::SetChapterOutline(co) if co.chapter_number >= 1 => {
                            ancestor_outlined_chapters
                                .insert((co.book_number.max(1), co.chapter_number));
                        }
                        _ => {}
                    }
                }
            }
            if ancestor_ids.contains(&p.id) {
                ctx.ancestor_promise_events
                    .extend(p.artifact.schedule.promise_events.clone());
                ctx.ancestor_pov_plan
                    .extend(p.artifact.schedule.pov_plan.clone());
                ctx.ancestor_intensity_targets
                    .extend(p.artifact.schedule.intensity_targets.clone());
                for pov in &p.artifact.schedule.pov_plan {
                    if pov.scene_order.is_none() {
                        ctx.chapter_povs.insert(
                            (pov.book_number, pov.chapter_number),
                            ctx.normalize_ref(&pov.pov_character_id),
                        );
                    }
                }
            }
        }
    }

    for item in &input.operations {
        ctx.ingest_operation(&item.output_ref, &item.operation);
    }

    let forbidden_phrases = ctx
        .reader_contract
        .as_ref()
        .map(|rc| extract_forbidden_boundary_phrases(&rc.boundaries))
        .unwrap_or_default();
    let boundary_cap = ctx
        .reader_contract
        .as_ref()
        .and_then(|rc| boundary_rating_cap(&rc.boundaries));
    let style_directive = ctx.reader_contract.as_ref().map(|rc| {
        spindle_core::style::StyleDirective::assemble(
            ctx.project_genre
                .clone()
                .unwrap_or_else(|| "fiction".to_string()),
            ctx.project_type
                .clone()
                .unwrap_or_else(|| "novel".to_string()),
            rc.promise.clone(),
            rc.style_notes.clone(),
            rc.boundaries.clone(),
            ctx.style_world_rules.clone(),
            None,
        )
    });

    let mut seen_plan_chapters = std::collections::BTreeSet::new();
    let effective_horizon = input
        .target_range
        .clone()
        .unwrap_or_else(|| s.horizon.clone());

    for (idx, item) in input.operations.iter().enumerate() {
        let item_path = format!("/operations/{idx}");
        let valid_ref = !item.output_ref.is_empty()
            && item.output_ref.len() <= 128
            && item
                .output_ref
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b));
        if !valid_ref {
            findings.push(PlanningFinding {
                code: "invalid_output_ref".to_string(),
                severity: PlanningFindingSeverity::Blocking,
                path: format!("{item_path}/output_ref"),
                related_refs: vec![item.output_ref.clone()],
                expected: "1..=128 ASCII alphanumeric or -_: characters".to_string(),
                actual: item.output_ref.clone(),
                repair_hint: "Use a non-empty proposal-local identifier such as 'cast:protagonist' or 'chapter:1'.".to_string(),
            });
        }
        let collides = output_refs(item)
            .iter()
            .any(|(k, _)| reserved.contains(k) || available.contains_key(k));
        if collides {
            findings.push(PlanningFinding {
                code: "duplicate_output_ref".to_string(),
                severity: PlanningFindingSeverity::Blocking,
                path: format!("{item_path}/output_ref"),
                related_refs: vec![item.output_ref.clone()],
                expected: "unique output_ref in session".to_string(),
                actual: item.output_ref.clone(),
                repair_hint: "Choose a unique output_ref that does not collide with earlier operations or active proposals.".to_string(),
            });
        }
        if let Err(err) = check_level(input.level, &item.operation, s.scope.is_some()) {
            let msg = err.to_string();
            let code = if msg.contains("realized") {
                "realized_event_in_initial_plan"
            } else {
                "operation_not_allowed_for_level"
            };
            findings.push(PlanningFinding {
                code: code.to_string(),
                severity: PlanningFindingSeverity::Blocking,
                path: format!("{item_path}/operation"),
                related_refs: vec![item.output_ref.clone()],
                expected: format!("valid operation for level {:?}", input.level),
                actual: msg,
                repair_hint: "Emit only operations permitted at this planning level/scope, and keep reached_at/established_in_scene_id/scene_id null and delivered=false.".to_string(),
            });
        }

        match &item.operation {
            PlanningOperation::CreateChapter(ch) => {
                if ch.book_number.is_some_and(|n| n < 1) || ch.chapter_number.is_some_and(|n| n < 1)
                {
                    findings.push(PlanningFinding {
                        code: "non_positive_book_or_chapter_number".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "book_number >= 1 and chapter_number >= 1".to_string(),
                        actual: format!(
                            "book_number={:?}, chapter_number={:?}",
                            ch.book_number, ch.chapter_number
                        ),
                        repair_hint: "Use 1-based positive book and chapter numbers.".to_string(),
                    });
                }
            }
            PlanningOperation::SetBookOutline(bo) => {
                if bo.book_number < 1 && bo.book_id.is_none() {
                    findings.push(PlanningFinding {
                        code: "non_positive_book_or_chapter_number".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/book_number"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "book_number >= 1".to_string(),
                        actual: bo.book_number.to_string(),
                        repair_hint: "Use a positive 1-based book_number.".to_string(),
                    });
                }
            }
            PlanningOperation::SetChapterOutline(co) => {
                if (co.entity_id.is_none() && (co.book_number < 1 || co.chapter_number < 1))
                    || co.book_number < 0
                    || co.chapter_number < 0
                {
                    findings.push(PlanningFinding {
                        code: "non_positive_book_or_chapter_number".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "book_number >= 1 and chapter_number >= 1".to_string(),
                        actual: format!(
                            "book_number={}, chapter_number={}",
                            co.book_number, co.chapter_number
                        ),
                        repair_hint: "Use positive 1-based book and chapter numbers.".to_string(),
                    });
                }
            }
            PlanningOperation::CreateTheme(theme) => {
                if let Some(ref intro) = theme.introduction_point
                    && !intro.has_positive_coordinates()
                {
                    findings.push(PlanningFinding {
                        code: "invalid_story_placement".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/introduction_point"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "positive book_number, chapter_number, and scene_order"
                            .to_string(),
                        actual: format!("{intro:?}"),
                        repair_hint: "Use 1-based positive story placement coordinates."
                            .to_string(),
                    });
                }
                if let Some(ref res) = theme.resolution_point
                    && !res.has_positive_coordinates()
                {
                    findings.push(PlanningFinding {
                        code: "invalid_story_placement".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/resolution_point"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "positive book_number, chapter_number, and scene_order"
                            .to_string(),
                        actual: format!("{res:?}"),
                        repair_hint: "Use 1-based positive story placement coordinates."
                            .to_string(),
                    });
                }
                if let (Some(intro), Some(res)) =
                    (&theme.introduction_point, &theme.resolution_point)
                    && res.cmp_story_order(intro) == std::cmp::Ordering::Less
                {
                    findings.push(PlanningFinding {
                        code: "unsupported_resolution".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/resolution_point"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: format!(
                            "resolution_point >= introduction_point (book {} chapter {})",
                            intro.book_number, intro.chapter_number
                        ),
                        actual: format!("book {} chapter {}", res.book_number, res.chapter_number),
                        repair_hint:
                            "Ensure theme resolution_point does not precede its introduction_point."
                                .to_string(),
                    });
                }
            }
            PlanningOperation::CreateConflict(conflict) => {
                for (c_idx, step) in conflict.try_fail_cycles.iter().enumerate() {
                    if step.attempt_order < 1
                        || (c_idx > 0
                            && step.attempt_order
                                <= conflict.try_fail_cycles[c_idx - 1].attempt_order)
                    {
                        findings.push(PlanningFinding {
                            code: "unsupported_resolution".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{item_path}/operation/input/try_fail_cycles/{c_idx}"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: "strictly increasing positive attempt_order".to_string(),
                            actual: step.attempt_order.to_string(),
                            repair_hint: "Order conflict try_fail_cycles with strictly increasing positive attempt_order.".to_string(),
                        });
                    }
                }
            }
            PlanningOperation::CreateNarrativePromise(np) => {
                if !np.planted_at.has_positive_coordinates() {
                    findings.push(PlanningFinding {
                        code: "invalid_story_placement".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/planted_at"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "positive book_number, chapter_number, and scene_order"
                            .to_string(),
                        actual: format!("{:?}", np.planted_at),
                        repair_hint: "Use 1-based positive story placement coordinates."
                            .to_string(),
                    });
                }
                match &np.planned_payoff {
                    Some(payoff) => {
                        if !payoff.has_positive_coordinates() {
                            findings.push(PlanningFinding {
                                code: "invalid_story_placement".to_string(),
                                severity: PlanningFindingSeverity::Blocking,
                                path: format!("{item_path}/operation/input/planned_payoff"),
                                related_refs: vec![item.output_ref.clone()],
                                expected: "positive book_number, chapter_number, and scene_order"
                                    .to_string(),
                                actual: format!("{payoff:?}"),
                                repair_hint: "Use 1-based positive story placement coordinates."
                                    .to_string(),
                            });
                        } else if payoff.cmp_story_order(&np.planted_at) == std::cmp::Ordering::Less
                        {
                            findings.push(PlanningFinding {
                                code: "promise_payoff_before_plant".to_string(),
                                severity: PlanningFindingSeverity::Blocking,
                                path: format!("{item_path}/operation/input/planned_payoff"),
                                related_refs: vec![item.output_ref.clone()],
                                expected: format!(
                                    "planned_payoff >= planted_at (book {} chapter {})",
                                    np.planted_at.book_number, np.planted_at.chapter_number
                                ),
                                actual: format!(
                                    "book {} chapter {}",
                                    payoff.book_number, payoff.chapter_number
                                ),
                                repair_hint:
                                    "Ensure narrative promise planned_payoff follows planted_at."
                                        .to_string(),
                            });
                        }
                    }
                    None => {
                        let has_plant_note = np
                            .planted_at
                            .note
                            .as_deref()
                            .is_some_and(|n| !n.trim().is_empty());
                        let has_notes = np.notes.iter().any(|n| !n.trim().is_empty());
                        let has_defer_event = input.schedule.promise_events.iter().any(|ev| {
                            ctx.normalize_ref(&ev.promise_ref) == item.output_ref
                                && ev.action == PlannedPromiseAction::Defer
                                && ev
                                    .placement
                                    .note
                                    .as_deref()
                                    .is_some_and(|n| !n.trim().is_empty())
                        });
                        if !has_plant_note && !has_notes && !has_defer_event {
                            findings.push(PlanningFinding {
                                code: "deferred_promise_missing_note_or_destination".to_string(),
                                severity: PlanningFindingSeverity::Blocking,
                                path: format!("{item_path}/operation/input/planned_payoff"),
                                related_refs: vec![item.output_ref.clone()],
                                expected: "planned_payoff placement or explicit deferral note/future-book destination".to_string(),
                                actual: "planned_payoff=null with no deferral note".to_string(),
                                repair_hint: "Provide a planned_payoff placement (which may target a future book) or record an explicit deferral note in planted_at.note, notes, or a defer promise_event.".to_string(),
                            });
                        }
                    }
                }
            }
            PlanningOperation::CreateCharacterArc(arc) => {
                let mut prev_placement: Option<(usize, &PlanningStoryPlacement)> = None;
                for (m_idx, milestone) in arc.milestones.iter().enumerate() {
                    if let Some(ref placement) = milestone.placement {
                        if !placement.has_positive_coordinates() {
                            findings.push(PlanningFinding {
                                code: "invalid_story_placement".to_string(),
                                severity: PlanningFindingSeverity::Blocking,
                                path: format!(
                                    "{item_path}/operation/input/milestones/{m_idx}/placement"
                                ),
                                related_refs: vec![item.output_ref.clone()],
                                expected: "positive book_number, chapter_number, and scene_order"
                                    .to_string(),
                                actual: format!("{placement:?}"),
                                repair_hint: "Use 1-based positive story placement coordinates."
                                    .to_string(),
                            });
                        } else if let Some((_, prev)) = prev_placement
                            && placement.cmp_story_order(prev) == std::cmp::Ordering::Less
                        {
                            findings.push(PlanningFinding {
                                code: "arc_milestones_unordered".to_string(),
                                severity: PlanningFindingSeverity::Blocking,
                                path: format!(
                                    "{item_path}/operation/input/milestones/{m_idx}/placement"
                                ),
                                related_refs: vec![item.output_ref.clone()],
                                expected: format!(
                                    "milestone placement >= book {} chapter {}",
                                    prev.book_number, prev.chapter_number
                                ),
                                actual: format!(
                                    "book {} chapter {}",
                                    placement.book_number, placement.chapter_number
                                ),
                                repair_hint:
                                    "Order character arc milestones chronologically across books, chapters, and scenes."
                                        .to_string(),
                            });
                        }
                        prev_placement = Some((m_idx, placement));
                    }
                }
            }
            PlanningOperation::SetArcPacingConstraints(constraints) => {
                let arc_key = ctx.normalize_ref(&constraints.character_arc_id);
                if let Some(max_prog) = constraints.max_progress_per_chapter
                    && (max_prog.is_nan() || !(0.0..=1.0).contains(&max_prog) || max_prog == 0.0)
                {
                    findings.push(PlanningFinding {
                        code: "arc_pacing_constraint_out_of_range".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/max_progress_per_chapter"),
                        related_refs: vec![arc_key.clone()],
                        expected: "(0.0, 1.0]".to_string(),
                        actual: max_prog.to_string(),
                        repair_hint: "Set max_progress_per_chapter within (0.0, 1.0].".to_string(),
                    });
                }
                for (book_k, &budget_val) in &constraints.per_book_budget {
                    if budget_val.is_nan() || !(0.0..=1.0).contains(&budget_val) {
                        findings.push(PlanningFinding {
                            code: "arc_pacing_constraint_out_of_range".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{item_path}/operation/input/per_book_budget/{book_k}"),
                            related_refs: vec![arc_key.clone()],
                            expected: "0.0..=1.0".to_string(),
                            actual: budget_val.to_string(),
                            repair_hint: "Keep per_book_budget entries within [0.0, 1.0]."
                                .to_string(),
                        });
                    }
                }
                if let Some(spacing) = constraints.milestone_spacing
                    && spacing < 1
                {
                    findings.push(PlanningFinding {
                        code: "arc_pacing_constraint_out_of_range".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/milestone_spacing"),
                        related_refs: vec![arc_key.clone()],
                        expected: "milestone_spacing >= 1".to_string(),
                        actual: spacing.to_string(),
                        repair_hint: "Set milestone_spacing to a positive chapter count (>= 1)."
                            .to_string(),
                    });
                }
                if let Some(placements) = ctx.arc_milestones.get(&arc_key) {
                    if let Some(min_spacing) = constraints.milestone_spacing
                        && min_spacing >= 1
                    {
                        for window in placements.windows(2) {
                            if window[0].book_number == window[1].book_number {
                                let gap = window[1].chapter_number - window[0].chapter_number;
                                if gap < min_spacing {
                                    findings.push(PlanningFinding {
                                        code: "arc_milestone_spacing_violation".to_string(),
                                        severity: PlanningFindingSeverity::Blocking,
                                        path: format!(
                                            "{item_path}/operation/input/milestone_spacing"
                                        ),
                                        related_refs: vec![arc_key.clone()],
                                        expected: format!(
                                            ">= {min_spacing} chapters between milestones"
                                        ),
                                        actual: format!("{gap} chapters between milestones"),
                                        repair_hint: "Space character arc milestones to satisfy milestone_spacing.".to_string(),
                                    });
                                }
                            }
                        }
                    }
                    for (book_k, &budget_val) in &constraints.per_book_budget {
                        if let Ok(book_no) = book_k.parse::<i32>() {
                            let in_book = placements
                                .iter()
                                .filter(|p| p.book_number == book_no)
                                .count();
                            let share_exceeded = placements.len() >= 2
                                && (in_book as f64) / (placements.len() as f64) > budget_val + 1e-9;
                            if (budget_val == 0.0 && in_book > 0) || share_exceeded {
                                findings.push(PlanningFinding {
                                    code: "arc_book_budget_exceeded".to_string(),
                                    severity: PlanningFindingSeverity::Blocking,
                                    path: format!(
                                        "{item_path}/operation/input/per_book_budget/{book_k}"
                                    ),
                                    related_refs: vec![arc_key.clone()],
                                    expected: format!(
                                        "milestone share in book {book_no} <= {budget_val:.2}"
                                    ),
                                    actual: format!(
                                        "{in_book}/{} milestones in book {book_no}",
                                        placements.len()
                                    ),
                                    repair_hint: "Increase per_book_budget or move milestones to another book.".to_string(),
                                });
                            }
                        }
                    }
                }
            }
            PlanningOperation::CreatePacingCurve(curve) => {
                if curve.book_number < 1 {
                    findings.push(PlanningFinding {
                        code: "non_positive_book_or_chapter_number".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/book_number"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "book_number >= 1".to_string(),
                        actual: curve.book_number.to_string(),
                        repair_hint: "Use a positive 1-based book_number.".to_string(),
                    });
                }
                let mut ordered_breakpoints: Vec<(u8, &String, f64)> = Vec::new();
                for (k, &v) in &curve.act_breakpoints {
                    if v.is_nan() || !(0.0..=1.0).contains(&v) {
                        findings.push(PlanningFinding {
                            code: "act_breakpoint_out_of_range".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{item_path}/operation/input/act_breakpoints/{k}"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: "0.0..=1.0".to_string(),
                            actual: v.to_string(),
                            repair_hint: "Keep all act_breakpoints values within [0.0, 1.0]."
                                .to_string(),
                        });
                    } else {
                        ordered_breakpoints.push((act_breakpoint_stage_rank(k), k, v));
                    }
                }
                ordered_breakpoints.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
                for window in ordered_breakpoints.windows(2) {
                    if window[1].2 + 1e-9 < window[0].2 {
                        findings.push(PlanningFinding {
                            code: "act_breakpoints_unordered".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{item_path}/operation/input/act_breakpoints"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: format!(
                                "{} ({:.2}) <= {} ({:.2})",
                                window[0].1, window[0].2, window[1].1, window[1].2
                            ),
                            actual: format!(
                                "{}={} > {}={}",
                                window[0].1, window[0].2, window[1].1, window[1].2
                            ),
                            repair_hint:
                                "Ensure act_breakpoints increase monotonically across acts."
                                    .to_string(),
                        });
                    }
                }
                let mut density_sum = 0.0;
                let mut density_valid = true;
                for (k, &v) in &curve.scene_type_density {
                    if v.is_nan() || !(0.0..=1.0).contains(&v) {
                        density_valid = false;
                        findings.push(PlanningFinding {
                            code: "scene_type_density_out_of_range".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{item_path}/operation/input/scene_type_density/{k}"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: "0.0..=1.0".to_string(),
                            actual: v.to_string(),
                            repair_hint: "Keep all scene_type_density values within [0.0, 1.0]."
                                .to_string(),
                        });
                    } else {
                        density_sum += v;
                    }
                }
                if density_valid
                    && !curve.scene_type_density.is_empty()
                    && (density_sum - 1.0).abs() > 0.15 + 1e-9
                {
                    let waived = s
                        .policy
                        .pacing_policy
                        .as_ref()
                        .and_then(|p| p.waiver_reason.as_deref())
                        .is_some_and(|w| !w.trim().is_empty());
                    let hard = s
                        .policy
                        .pacing_policy
                        .as_ref()
                        .is_some_and(|p| p.hard_constraint)
                        && !waived;
                    findings.push(PlanningFinding {
                        code: "scene_type_density_unnormalized".to_string(),
                        severity: if hard {
                            PlanningFindingSeverity::Blocking
                        } else {
                            PlanningFindingSeverity::Advisory
                        },
                        path: format!("{item_path}/operation/input/scene_type_density"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "sum of scene_type_density within 1.0 +/- 0.15".to_string(),
                        actual: format!("{density_sum:.3}"),
                        repair_hint: "Normalize scene_type_density weights so they sum to ~1.0."
                            .to_string(),
                    });
                }
                for (p_idx, pt) in curve.intensity_points.iter().enumerate() {
                    if pt.position.is_nan()
                        || !(0.0..=1.0).contains(&pt.position)
                        || pt.intensity.is_nan()
                        || !(0.0..=1.0).contains(&pt.intensity)
                    {
                        findings.push(PlanningFinding {
                            code: "intensity_point_out_of_range".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{item_path}/operation/input/intensity_points/{p_idx}"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: "position and intensity in 0.0..=1.0".to_string(),
                            actual: format!("position={}, intensity={}", pt.position, pt.intensity),
                            repair_hint:
                                "Keep intensity_points position and intensity within [0.0, 1.0]."
                                    .to_string(),
                        });
                    }
                    if p_idx > 0 && pt.position + 1e-9 < curve.intensity_points[p_idx - 1].position
                    {
                        findings.push(PlanningFinding {
                            code: "intensity_points_unordered".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{item_path}/operation/input/intensity_points/{p_idx}"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: format!(
                                "position >= {}",
                                curve.intensity_points[p_idx - 1].position
                            ),
                            actual: pt.position.to_string(),
                            repair_hint:
                                "Order intensity_points by non-decreasing normalized position."
                                    .to_string(),
                        });
                    }
                }
            }
            PlanningOperation::PlanChapter(plan) => {
                if plan.book_number < 1 || plan.chapter_number < 1 {
                    findings.push(PlanningFinding {
                        code: "non_positive_book_or_chapter_number".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "book_number >= 1 and chapter_number >= 1".to_string(),
                        actual: format!(
                            "book_number={}, chapter_number={}",
                            plan.book_number, plan.chapter_number
                        ),
                        repair_hint: "Use positive 1-based book_number and chapter_number."
                            .to_string(),
                    });
                }
                if !seen_plan_chapters.insert((plan.book_number, plan.chapter_number)) {
                    findings.push(PlanningFinding {
                        code: "duplicate_chapter_manifest".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: format!(
                            "single PlanChapter for book {} chapter {}",
                            plan.book_number, plan.chapter_number
                        ),
                        actual: "duplicate chapter manifest in proposal".to_string(),
                        repair_hint:
                            "Combine scene manifests so each (book_number, chapter_number) is planned once per proposal."
                                .to_string(),
                    });
                }
                if plan.synopsis.trim().is_empty() {
                    findings.push(PlanningFinding {
                        code: "empty_chapter_synopsis".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/synopsis"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "non-empty chapter synopsis".to_string(),
                        actual: "empty".to_string(),
                        repair_hint: "Provide a non-empty synopsis for every planned chapter."
                            .to_string(),
                    });
                }
                let syn_lower = plan.synopsis.to_ascii_lowercase();
                for (raw_boundary, phrase) in &forbidden_phrases {
                    if syn_lower.contains(phrase) {
                        findings.push(PlanningFinding {
                            code: "boundary_conflict".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{item_path}/operation/input/synopsis"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: format!("respect reader_contract boundary '{raw_boundary}'"),
                            actual: format!("synopsis contains forbidden element '{phrase}'"),
                            repair_hint: format!(
                                "Remove '{phrase}' from the chapter synopsis to honor boundary '{raw_boundary}'."
                            ),
                        });
                    }
                }
                if input.level == PlanningLevel::Scene && plan.scenes.is_empty() {
                    findings.push(PlanningFinding {
                        code: "empty_scene_manifest".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/scenes"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: "non-empty scenes array at Scene level".to_string(),
                        actual: "0 scenes".to_string(),
                        repair_hint:
                            "Populate concrete scene manifests for every planned chapter at Scene level."
                                .to_string(),
                    });
                }
                if input.level == PlanningLevel::Scene
                    && !plan.scenes.is_empty()
                    && effective_horizon.scenes_per_chapter >= 1
                    && plan.scenes.len() != effective_horizon.scenes_per_chapter as usize
                {
                    findings.push(PlanningFinding {
                        code: "scene_count_mismatch".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/scenes"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: format!("{} scenes", effective_horizon.scenes_per_chapter),
                        actual: format!("{} scenes", plan.scenes.len()),
                        repair_hint: format!(
                            "Provide exactly {} scenes for book {} chapter {} to match target_range.scenes_per_chapter.",
                            effective_horizon.scenes_per_chapter, plan.book_number, plan.chapter_number
                        ),
                    });
                }

                // Check chapter POV consistency against schedule.pov_plan
                if let Some(ref ch_pov) = plan.pov_character_id {
                    let ch_pov_key = ctx.normalize_ref(ch_pov);
                    for sched_pov in input
                        .schedule
                        .pov_plan
                        .iter()
                        .chain(ctx.ancestor_pov_plan.iter())
                    {
                        if sched_pov.book_number == plan.book_number
                            && sched_pov.chapter_number == plan.chapter_number
                            && sched_pov.scene_order.is_none()
                        {
                            let sched_key = ctx.normalize_ref(&sched_pov.pov_character_id);
                            if sched_key != ch_pov_key {
                                findings.push(PlanningFinding {
                                    code: "chapter_pov_schedule_mismatch".to_string(),
                                    severity: PlanningFindingSeverity::Blocking,
                                    path: format!("{item_path}/operation/input/pov_character_id"),
                                    related_refs: vec![ch_pov_key.clone(), sched_key.clone()],
                                    expected: format!(
                                        "pov_character_id matching schedule.pov_plan ({sched_key})"
                                    ),
                                    actual: ch_pov_key.clone(),
                                    repair_hint: "Align PlanChapter.pov_character_id with the chapter's PlannedPov in schedule.pov_plan.".to_string(),
                                });
                            }
                        }
                    }
                }

                // Check unrelated thread targets when target_plot_line_ids are specified
                if !plan.target_plot_line_ids.is_empty() {
                    let targeted_plots: Vec<&ValidatorThreadLinks> = plan
                        .target_plot_line_ids
                        .iter()
                        .filter_map(|r| ctx.plot_lines.get(&ctx.normalize_ref(r)))
                        .collect();
                    if !targeted_plots.is_empty() {
                        let pov_links = plan
                            .pov_character_id
                            .as_ref()
                            .and_then(|pov| ctx.character_arcs.get(&ctx.normalize_ref(pov)));
                        let any_plot_themes = targeted_plots.iter().any(|pl| !pl.themes.is_empty());
                        if any_plot_themes {
                            let mut allowed_themes: std::collections::BTreeSet<String> =
                                targeted_plots
                                    .iter()
                                    .flat_map(|pl| pl.themes.iter().cloned())
                                    .collect();
                            if let Some(links) = pov_links {
                                allowed_themes.extend(links.themes.iter().cloned());
                            }
                            for (t_idx, theme_ref) in plan.target_theme_ids.iter().enumerate() {
                                let t_key = ctx.normalize_ref(theme_ref);
                                if !allowed_themes.contains(&t_key) {
                                    findings.push(PlanningFinding {
                                        code: "unrelated_thread_target".to_string(),
                                        severity: PlanningFindingSeverity::Blocking,
                                        path: format!(
                                            "{item_path}/operation/input/target_theme_ids/{t_idx}"
                                        ),
                                        related_refs: vec![t_key.clone()],
                                        expected: format!(
                                            "theme connected to targeted plot line(s) or POV arc ({:?})",
                                            allowed_themes
                                        ),
                                        actual: t_key,
                                        repair_hint: "Target only themes connected to the chapter's target_plot_line_ids or POV character arc, or connect the theme to the plot line.".to_string(),
                                    });
                                }
                            }
                        }
                        let any_plot_conflicts =
                            targeted_plots.iter().any(|pl| !pl.conflicts.is_empty());
                        if any_plot_conflicts {
                            let mut allowed_conflicts: std::collections::BTreeSet<String> =
                                targeted_plots
                                    .iter()
                                    .flat_map(|pl| pl.conflicts.iter().cloned())
                                    .collect();
                            if let Some(links) = pov_links {
                                allowed_conflicts.extend(links.conflicts.iter().cloned());
                            }
                            for (c_idx, conflict_ref) in plan.target_conflict_ids.iter().enumerate()
                            {
                                let c_key = ctx.normalize_ref(conflict_ref);
                                if !allowed_conflicts.contains(&c_key) {
                                    findings.push(PlanningFinding {
                                        code: "unrelated_thread_target".to_string(),
                                        severity: PlanningFindingSeverity::Blocking,
                                        path: format!(
                                            "{item_path}/operation/input/target_conflict_ids/{c_idx}"
                                        ),
                                        related_refs: vec![c_key.clone()],
                                        expected: format!(
                                            "conflict connected to targeted plot line(s) or POV arc ({:?})",
                                            allowed_conflicts
                                        ),
                                        actual: c_key,
                                        repair_hint: "Target only conflicts connected to the chapter's target_plot_line_ids or POV character arc, or connect the conflict to the plot line.".to_string(),
                                    });
                                }
                            }
                        }
                    }
                }

                if !plan.scenes.is_empty() {
                    let expected_orders: Vec<i32> = (1..=plan.scenes.len() as i32).collect();
                    let actual_orders: Vec<i32> =
                        plan.scenes.iter().map(|sc| sc.scene_order).collect();
                    if actual_orders != expected_orders {
                        findings.push(PlanningFinding {
                            code: "non_contiguous_scene_orders".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{item_path}/operation/input/scenes"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: format!("contiguous scene_order {:?}", expected_orders),
                            actual: format!("{:?}", actual_orders),
                            repair_hint:
                                "Number scenes contiguously starting at 1 (1..=scenes.len())."
                                    .to_string(),
                        });
                    }
                }

                for (s_idx, scene) in plan.scenes.iter().enumerate() {
                    let scene_path = format!("{item_path}/operation/input/scenes/{s_idx}");
                    if scene.summary.trim().is_empty() {
                        findings.push(PlanningFinding {
                            code: "empty_scene_summary".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{scene_path}/summary"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: "non-empty scene summary".to_string(),
                            actual: "empty".to_string(),
                            repair_hint: "Provide a non-empty summary for every planned scene."
                                .to_string(),
                        });
                    }
                    if scene.purpose.trim().is_empty() {
                        findings.push(PlanningFinding {
                            code: "empty_scene_purpose".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{scene_path}/purpose"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: "non-empty scene purpose".to_string(),
                            actual: "empty".to_string(),
                            repair_hint:
                                "Provide a concrete state-changing purpose for every planned scene."
                                    .to_string(),
                        });
                    } else if is_noop_scene_purpose(&scene.purpose) {
                        findings.push(PlanningFinding {
                            code: "scene_missing_state_change".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{scene_path}/purpose"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: "purpose describing a concrete narrative state change"
                                .to_string(),
                            actual: scene.purpose.clone(),
                            repair_hint:
                                "Replace placeholder/no-op scene purpose with a concrete narrative turn or state change."
                                    .to_string(),
                        });
                    }
                    if plan.target_conflict_ids.is_empty() {
                        let purp_lower = scene.purpose.to_ascii_lowercase();
                        let sum_lower = scene.summary.to_ascii_lowercase();
                        if purp_lower.contains("resolve conflict")
                            || purp_lower.contains("resolves conflict")
                            || purp_lower.contains("resolves the conflict")
                            || purp_lower.contains("conflict resolution")
                            || sum_lower.contains("resolves conflict")
                            || sum_lower.contains("resolves the conflict")
                        {
                            findings.push(PlanningFinding {
                                code: "unsupported_resolution".to_string(),
                                severity: PlanningFindingSeverity::Blocking,
                                path: format!("{scene_path}/purpose"),
                                related_refs: vec![item.output_ref.clone()],
                                expected: "non-empty target_conflict_ids when resolving a conflict"
                                    .to_string(),
                                actual: "target_conflict_ids is empty".to_string(),
                                repair_hint: "Link the conflict in PlanChapter.target_conflict_ids before claiming conflict resolution in a scene.".to_string(),
                            });
                        }
                    }
                    if scene.character_ids.is_empty() {
                        findings.push(PlanningFinding {
                            code: "empty_scene_characters".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{scene_path}/character_ids"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: "at least 1 character_id in scene cast".to_string(),
                            actual: "0 character_ids".to_string(),
                            repair_hint:
                                "Populate scene.character_ids with the characters participating in the scene."
                                    .to_string(),
                        });
                    }
                    if scene.location_id.is_none() {
                        findings.push(PlanningFinding {
                            code: "missing_scene_location".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{scene_path}/location_id"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: "explicit location_id reference".to_string(),
                            actual: "null".to_string(),
                            repair_hint: "Set an explicit location_id on every planned scene; do not embed location metadata only in summary/purpose text.".to_string(),
                        });
                    }
                    match scene.content_rating.as_ref() {
                        None => {
                            findings.push(PlanningFinding {
                                code: "missing_scene_content_rating".to_string(),
                                severity: PlanningFindingSeverity::Blocking,
                                path: format!("{scene_path}/content_rating"),
                                related_refs: vec![item.output_ref.clone()],
                                expected: "explicit content_rating (general, teen, mature, or explicit)".to_string(),
                                actual: "null".to_string(),
                                repair_hint: "Set an explicit content_rating on every planned scene; do not rely on text inference.".to_string(),
                            });
                        }
                        Some(scene_rating) => {
                            if let Some(ref ceiling) = effective_ceiling
                                && rating_rank(scene_rating) > rating_rank(ceiling)
                            {
                                findings.push(PlanningFinding {
                                    code: "scene_rating_exceeds_ceiling".to_string(),
                                    severity: PlanningFindingSeverity::Blocking,
                                    path: format!("{scene_path}/content_rating"),
                                    related_refs: vec![item.output_ref.clone()],
                                    expected: format!("<= {ceiling:?}"),
                                    actual: format!("{scene_rating:?}"),
                                    repair_hint: "Lower the planned scene content_rating within the session's rating_ceiling.".to_string(),
                                });
                            }
                            if let Some(ref b_cap) = boundary_cap
                                && rating_rank(scene_rating) > rating_rank(b_cap)
                            {
                                findings.push(PlanningFinding {
                                    code: "boundary_conflict".to_string(),
                                    severity: PlanningFindingSeverity::Blocking,
                                    path: format!("{scene_path}/content_rating"),
                                    related_refs: vec![item.output_ref.clone()],
                                    expected: format!(
                                        "content_rating <= {b_cap:?} per reader_contract boundaries"
                                    ),
                                    actual: format!("{scene_rating:?}"),
                                    repair_hint: "Lower scene content_rating to respect reader_contract boundaries.".to_string(),
                                });
                            }
                        }
                    }

                    // Check scene summary & purpose against reader_contract forbidden boundaries
                    let scene_text_lower = format!(
                        "{} {}",
                        scene.summary.to_ascii_lowercase(),
                        scene.purpose.to_ascii_lowercase()
                    );
                    for (raw_boundary, phrase) in &forbidden_phrases {
                        if scene_text_lower.contains(phrase) {
                            findings.push(PlanningFinding {
                                code: "boundary_conflict".to_string(),
                                severity: PlanningFindingSeverity::Blocking,
                                path: scene_path.clone(),
                                related_refs: vec![item.output_ref.clone()],
                                expected: format!(
                                    "respect reader_contract boundary '{raw_boundary}'"
                                ),
                                actual: format!("scene contains forbidden element '{phrase}'"),
                                repair_hint: format!(
                                    "Revise scene summary/purpose to remove '{phrase}' and honor boundary '{raw_boundary}'."
                                ),
                            });
                        }
                    }

                    // Check POV character presence in scene cast
                    let scene_sched_pov = input
                        .schedule
                        .pov_plan
                        .iter()
                        .chain(ctx.ancestor_pov_plan.iter())
                        .find(|p| {
                            p.book_number == plan.book_number
                                && p.chapter_number == plan.chapter_number
                                && p.scene_order == Some(scene.scene_order)
                        })
                        .map(|p| ctx.normalize_ref(&p.pov_character_id));
                    let ch_sched_pov = input
                        .schedule
                        .pov_plan
                        .iter()
                        .chain(ctx.ancestor_pov_plan.iter())
                        .find(|p| {
                            p.book_number == plan.book_number
                                && p.chapter_number == plan.chapter_number
                                && p.scene_order.is_none()
                        })
                        .map(|p| ctx.normalize_ref(&p.pov_character_id));
                    let effective_scene_pov = scene_sched_pov
                        .or_else(|| plan.pov_character_id.as_ref().map(|r| ctx.normalize_ref(r)))
                        .or(ch_sched_pov);
                    if let Some(pov_key) = effective_scene_pov
                        && !scene.character_ids.is_empty()
                        && !scene
                            .character_ids
                            .iter()
                            .any(|c| ctx.normalize_ref(c) == pov_key)
                    {
                        findings.push(PlanningFinding {
                            code: "pov_character_missing_from_scene_cast".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("{scene_path}/character_ids"),
                            related_refs: vec![pov_key.clone()],
                            expected: format!("scene character_ids includes POV '{pov_key}'"),
                            actual: format!(
                                "{:?}",
                                scene
                                    .character_ids
                                    .iter()
                                    .map(|c| ctx.normalize_ref(c))
                                    .collect::<Vec<_>>()
                            ),
                            repair_hint: format!(
                                "Include POV character '{pov_key}' in scene.character_ids."
                            ),
                        });
                    }
                }

                // Style warnings (Advisory) from StyleDirective
                if let Some(ref directive) = style_directive
                    && !directive.is_empty()
                    && !plan.scenes.is_empty()
                {
                    let entries: Vec<spindle_core::models::PlanChapterSceneInput> = plan
                        .scenes
                        .iter()
                        .map(|sc| spindle_core::models::PlanChapterSceneInput {
                            scene_order: sc.scene_order,
                            summary: sc.summary.clone(),
                            beat_structure: sc.beat_structure.clone(),
                            character_ids: sc
                                .character_ids
                                .iter()
                                .map(|r| ctx.normalize_ref(r))
                                .collect(),
                            location_id: sc.location_id.as_ref().map(|r| ctx.normalize_ref(r)),
                            content_rating: sc.content_rating.clone(),
                            purpose: sc.purpose.clone(),
                            research_required: sc.research_required,
                            research_tags: sc.research_tags.clone(),
                            explicit_query: sc.explicit_query.clone(),
                        })
                        .collect();
                    for warning in
                        crate::sqlite::service::validate_chapter_plan_style(directive, &entries)
                    {
                        findings.push(PlanningFinding {
                            code: "plan_chapter_style_warning".to_string(),
                            severity: PlanningFindingSeverity::Advisory,
                            path: format!("{item_path}/operation/input/scenes"),
                            related_refs: vec![item.output_ref.clone()],
                            expected: "adherence to style_notes and style world rules".to_string(),
                            actual: warning,
                            repair_hint:
                                "Adjust scene summary or purpose to avoid forbidden style patterns."
                                    .to_string(),
                        });
                    }
                }
            }
            _ => {}
        }

        let op_val = serde_json::to_value(&item.operation)?;
        collect_ref_findings(
            conn,
            &op_val,
            &format!("{item_path}/operation"),
            &available,
            &s.scope,
            None,
            &mut findings,
        );
        available.extend(output_refs(item));
    }

    // Target range chapter coverage check at Scene level
    if input.level == PlanningLevel::Scene {
        if let Some(ref tr) = input.target_range {
            if tr.book_number >= 1 && tr.chapter_start >= 1 && tr.chapter_end >= tr.chapter_start {
                for ch in tr.chapter_start..=tr.chapter_end {
                    if !seen_plan_chapters.contains(&(tr.book_number, ch)) {
                        findings.push(PlanningFinding {
                            code: "missing_target_chapter_manifest".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: "/operations".to_string(),
                            related_refs: Vec::new(),
                            expected: format!(
                                "PlanChapter manifest for book {} chapter {}",
                                tr.book_number, ch
                            ),
                            actual: "missing".to_string(),
                            repair_hint: format!(
                                "Include a PlanChapter operation with {} scenes for book {} chapter {}.",
                                tr.scenes_per_chapter, tr.book_number, ch
                            ),
                        });
                    }
                }
            }
        } else if !ancestor_outlined_chapters.is_empty() {
            for &(book_no, ch_no) in &ancestor_outlined_chapters {
                if book_no == effective_horizon.book_number
                    && ch_no >= effective_horizon.chapter_start
                    && ch_no <= effective_horizon.chapter_end
                    && !seen_plan_chapters.contains(&(book_no, ch_no))
                {
                    findings.push(PlanningFinding {
                        code: "missing_target_chapter_manifest".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: "/operations".to_string(),
                        related_refs: Vec::new(),
                        expected: format!(
                            "PlanChapter manifest for outlined book {book_no} chapter {ch_no}"
                        ),
                        actual: "missing".to_string(),
                        repair_hint: format!(
                            "Include a PlanChapter operation for outlined book {book_no} chapter {ch_no}."
                        ),
                    });
                }
            }
        }
    }

    if !input.schedule.is_empty() {
        let sched_val = serde_json::to_value(&input.schedule)?;
        collect_ref_findings(
            conn,
            &sched_val,
            "/schedule",
            &available,
            &s.scope,
            None,
            &mut findings,
        );

        for (idx, pov) in input.schedule.pov_plan.iter().enumerate() {
            if pov.book_number < 1
                || pov.chapter_number < 1
                || pov.scene_order.is_some_and(|s| s < 1)
            {
                findings.push(PlanningFinding {
                    code: "invalid_story_placement".to_string(),
                    severity: PlanningFindingSeverity::Blocking,
                    path: format!("/schedule/pov_plan/{idx}"),
                    related_refs: vec![ctx.normalize_ref(&pov.pov_character_id)],
                    expected: "positive book_number, chapter_number, and scene_order".to_string(),
                    actual: format!(
                        "book={}, chapter={}, scene_order={:?}",
                        pov.book_number, pov.chapter_number, pov.scene_order
                    ),
                    repair_hint: "Use 1-based positive coordinates in schedule.pov_plan."
                        .to_string(),
                });
            } else if pov.scene_order.is_none() {
                ctx.chapter_povs.insert(
                    (pov.book_number, pov.chapter_number),
                    ctx.normalize_ref(&pov.pov_character_id),
                );
            }
        }

        // Promise schedule checks
        let all_promise_events: Vec<&PlannedPromiseEvent> = ctx
            .ancestor_promise_events
            .iter()
            .chain(input.schedule.promise_events.iter())
            .collect();
        let mut plant_by_promise: std::collections::BTreeMap<String, PlanningStoryPlacement> = ctx
            .promises
            .iter()
            .map(|(k, v)| (k.clone(), v.planted_at.clone()))
            .collect();
        let mut payoff_by_promise: std::collections::BTreeMap<String, PlanningStoryPlacement> = ctx
            .promises
            .iter()
            .filter_map(|(k, v)| v.planned_payoff.as_ref().map(|p| (k.clone(), p.clone())))
            .collect();
        for ev in &all_promise_events {
            let key = ctx.normalize_ref(&ev.promise_ref);
            match ev.action {
                PlannedPromiseAction::Plant => {
                    plant_by_promise
                        .entry(key)
                        .or_insert_with(|| ev.placement.clone());
                }
                PlannedPromiseAction::Payoff => {
                    payoff_by_promise.insert(key, ev.placement.clone());
                }
                _ => {}
            }
        }

        for (idx, ev) in input.schedule.promise_events.iter().enumerate() {
            let key = ctx.normalize_ref(&ev.promise_ref);
            if !ev.placement.has_positive_coordinates() {
                findings.push(PlanningFinding {
                    code: "invalid_story_placement".to_string(),
                    severity: PlanningFindingSeverity::Blocking,
                    path: format!("/schedule/promise_events/{idx}/placement"),
                    related_refs: vec![key],
                    expected: "positive book_number, chapter_number, and scene_order".to_string(),
                    actual: format!("{:?}", ev.placement),
                    repair_hint: "Use 1-based positive story placement coordinates.".to_string(),
                });
                continue;
            }
            match ev.action {
                PlannedPromiseAction::Plant => {}
                PlannedPromiseAction::Payoff => {
                    if let Some(planted_at) = plant_by_promise.get(&key)
                        && ev.placement.cmp_story_order(planted_at) == std::cmp::Ordering::Less
                    {
                        findings.push(PlanningFinding {
                            code: "promise_payoff_before_plant".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("/schedule/promise_events/{idx}/placement"),
                            related_refs: vec![key.clone()],
                            expected: format!(
                                "placement >= book {} chapter {}",
                                planted_at.book_number, planted_at.chapter_number
                            ),
                            actual: format!(
                                "book {} chapter {}",
                                ev.placement.book_number, ev.placement.chapter_number
                            ),
                            repair_hint:
                                "Ensure a promise payoff placement never precedes its planted_at placement."
                                    .to_string(),
                        });
                    }
                }
                PlannedPromiseAction::Advance => {
                    if let Some(planted_at) = plant_by_promise.get(&key)
                        && ev.placement.cmp_story_order(planted_at) == std::cmp::Ordering::Less
                    {
                        findings.push(PlanningFinding {
                            code: "promise_advance_outside_window".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("/schedule/promise_events/{idx}/placement"),
                            related_refs: vec![key.clone()],
                            expected: format!(
                                "advance >= plant (book {} chapter {})",
                                planted_at.book_number, planted_at.chapter_number
                            ),
                            actual: format!(
                                "book {} chapter {}",
                                ev.placement.book_number, ev.placement.chapter_number
                            ),
                            repair_hint:
                                "Schedule promise advance/reinforce beats between setup (plant) and payoff."
                                    .to_string(),
                        });
                    }
                    if let Some(payoff_at) = payoff_by_promise.get(&key)
                        && ev.placement.cmp_story_order(payoff_at) == std::cmp::Ordering::Greater
                    {
                        findings.push(PlanningFinding {
                            code: "promise_advance_outside_window".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("/schedule/promise_events/{idx}/placement"),
                            related_refs: vec![key.clone()],
                            expected: format!(
                                "advance <= payoff (book {} chapter {})",
                                payoff_at.book_number, payoff_at.chapter_number
                            ),
                            actual: format!(
                                "book {} chapter {}",
                                ev.placement.book_number, ev.placement.chapter_number
                            ),
                            repair_hint:
                                "Schedule promise advance/reinforce beats on or before payoff."
                                    .to_string(),
                        });
                    }
                }
                PlannedPromiseAction::Defer => {
                    let has_note = ev
                        .placement
                        .note
                        .as_deref()
                        .is_some_and(|n| !n.trim().is_empty());
                    let has_future_dest = ctx
                        .promises
                        .get(&key)
                        .and_then(|d| d.planned_payoff.as_ref())
                        .is_some_and(|p| {
                            p.book_number > effective_horizon.book_number
                                || (p.book_number == effective_horizon.book_number
                                    && p.chapter_number > effective_horizon.chapter_end)
                        });
                    if !has_note && !has_future_dest {
                        findings.push(PlanningFinding {
                            code: "deferred_promise_missing_note_or_destination".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: format!("/schedule/promise_events/{idx}/placement"),
                            related_refs: vec![key.clone()],
                            expected:
                                "non-empty placement.note or future-book planned_payoff destination"
                                    .to_string(),
                            actual: "defer event with empty note and no future destination"
                                .to_string(),
                            repair_hint:
                                "Provide a non-empty note on the defer event or set planned_payoff to a future book/chapter."
                                    .to_string(),
                        });
                    } else if ctx
                        .promises
                        .get(&key)
                        .and_then(|d| d.planned_payoff.as_ref())
                        .is_some_and(|p| {
                            p.book_number == effective_horizon.book_number
                                && p.chapter_number >= effective_horizon.chapter_start
                                && p.chapter_number <= effective_horizon.chapter_end
                        })
                    {
                        findings.push(PlanningFinding {
                            code: "promise_unresolved_deadline".to_string(),
                            severity: PlanningFindingSeverity::Advisory,
                            path: format!("/schedule/promise_events/{idx}/placement"),
                            related_refs: vec![key],
                            expected: "in-horizon payoff or updated promise deadline".to_string(),
                            actual: "in-horizon promise deferred".to_string(),
                            repair_hint:
                                "Verify that deferring this in-horizon promise deadline is intentional."
                                    .to_string(),
                        });
                    }
                }
            }
        }

        if !input.schedule.promise_events.is_empty() {
            let scheduled_keys: std::collections::BTreeSet<String> = all_promise_events
                .iter()
                .map(|ev| ctx.normalize_ref(&ev.promise_ref))
                .collect();
            for (promise_key, def) in &ctx.promises {
                let plant_in_horizon = def.planted_at.book_number == effective_horizon.book_number
                    && def.planted_at.chapter_number >= effective_horizon.chapter_start
                    && def.planted_at.chapter_number <= effective_horizon.chapter_end;
                let payoff_in_horizon = def.planned_payoff.as_ref().is_some_and(|p| {
                    p.book_number == effective_horizon.book_number
                        && p.chapter_number >= effective_horizon.chapter_start
                        && p.chapter_number <= effective_horizon.chapter_end
                });
                if (plant_in_horizon || payoff_in_horizon) && !scheduled_keys.contains(promise_key)
                {
                    findings.push(PlanningFinding {
                        code: "unscheduled_in_horizon_promise".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: "/schedule/promise_events".to_string(),
                        related_refs: vec![promise_key.clone()],
                        expected: format!(
                            "scheduled promise beat for in-horizon promise '{promise_key}'"
                        ),
                        actual: "unscheduled".to_string(),
                        repair_hint: format!(
                            "Add a plant/advance/payoff/defer entry in schedule.promise_events for '{promise_key}'."
                        ),
                    });
                }
            }
        }

        for (idx, intensity) in input.schedule.intensity_targets.iter().enumerate() {
            if intensity.book_number < 1 || intensity.chapter_number < 1 {
                findings.push(PlanningFinding {
                    code: "invalid_story_placement".to_string(),
                    severity: PlanningFindingSeverity::Blocking,
                    path: format!("/schedule/intensity_targets/{idx}"),
                    related_refs: Vec::new(),
                    expected: "book_number >= 1 and chapter_number >= 1".to_string(),
                    actual: format!(
                        "book={}, chapter={}",
                        intensity.book_number, intensity.chapter_number
                    ),
                    repair_hint:
                        "Use 1-based positive book and chapter numbers in intensity_targets."
                            .to_string(),
                });
            }
            if intensity.target_intensity.is_nan()
                || !(0.0..=1.0).contains(&intensity.target_intensity)
            {
                findings.push(PlanningFinding {
                    code: "intensity_out_of_range".to_string(),
                    severity: PlanningFindingSeverity::Blocking,
                    path: format!("/schedule/intensity_targets/{idx}/target_intensity"),
                    related_refs: Vec::new(),
                    expected: "0.0..=1.0".to_string(),
                    actual: intensity.target_intensity.to_string(),
                    repair_hint: "Keep target_intensity within [0.0, 1.0].".to_string(),
                });
            }
        }
    }

    let combined_intensity: Vec<PlannedIntensity> = ctx
        .ancestor_intensity_targets
        .iter()
        .chain(input.schedule.intensity_targets.iter())
        .cloned()
        .collect();
    let emit_rotation_or_pacing = matches!(
        input.level,
        PlanningLevel::SeriesArc | PlanningLevel::Chapter | PlanningLevel::Scene
    ) || !input.schedule.pov_plan.is_empty()
        || !input.schedule.intensity_targets.is_empty();
    let _ = compute_pov_and_pacing_metrics(
        &ctx,
        &s.policy,
        effective_horizon.book_number.max(1),
        &ctx.chapter_povs,
        &combined_intensity,
        &mut findings,
        emit_rotation_or_pacing,
    );

    Ok(findings)
}

#[derive(Debug)]
pub(crate) enum PlanningAttemptClaim {
    Existing(Box<PlanningAdvanceReport>),
    Claimed {
        attempt_id: String,
        claim_token: String,
        active_guard: PlanningActiveAttemptGuard,
        created_at: i64,
        session: Box<PlanningSession>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct PlanningCallReservation {
    pub estimated_input_tokens: u64,
    pub reserved_output_tokens: u64,
    pub reserved_total_tokens: u64,
    pub reserved_cost_usd: Option<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct PlanningSessionMetadataUpdate {
    pub session_id: String,
    pub scope: Option<PlanningScope>,
    pub horizon: PlanningTargetRange,
    pub budget: PlanningBudget,
    pub rating_ceiling: Option<spindle_core::models::ContentRating>,
    pub allow_review_backend: bool,
    pub last_outcome: PlanningSessionOutcome,
}

#[derive(Debug, Clone)]
pub(crate) struct PlanningStagedCommitCandidate {
    pub input: StagePlanningInput,
    pub receipt: PlanningGenerationReceipt,
    pub findings: Vec<PlanningFinding>,
    pub fallback_host_request: PlanningHostRequest,
}

fn check_budget_in_conn(
    budget: &PlanningBudget,
    usage: &PlanningBudgetUsage,
    estimated_input_tokens: u64,
) -> std::result::Result<PlanningCallReservation, String> {
    if budget.max_output_tokens_per_call == 0 {
        return Err("max_output_tokens_per_call is 0".to_string());
    }
    if usage.model_calls.saturating_add(1) > budget.max_model_calls {
        return Err(format!(
            "call ceiling reached ({}/{} model calls used)",
            usage.model_calls, budget.max_model_calls
        ));
    }
    let next_input = usage.input_tokens.saturating_add(estimated_input_tokens);
    if next_input > budget.max_input_tokens {
        return Err(format!(
            "input token ceiling exceeded (need {}, max {})",
            next_input, budget.max_input_tokens
        ));
    }
    let reserved_output = u64::from(budget.max_output_tokens_per_call);
    let next_output = usage.output_tokens.saturating_add(reserved_output);
    if next_output > budget.max_output_tokens {
        return Err(format!(
            "output token ceiling cannot cover max_output_tokens_per_call (used {} + reserve {} > max {})",
            usage.output_tokens, reserved_output, budget.max_output_tokens
        ));
    }
    let reserved_total = estimated_input_tokens.saturating_add(reserved_output);
    let next_total = usage.total_tokens.saturating_add(reserved_total);
    if next_total > budget.max_total_tokens {
        return Err(format!(
            "total token ceiling cannot cover call (used {} + input {} + reserve {} > max {})",
            usage.total_tokens, estimated_input_tokens, reserved_output, budget.max_total_tokens
        ));
    }
    let reserved_cost_usd = match (
        budget.input_cost_per_million_usd,
        budget.output_cost_per_million_usd,
    ) {
        (Some(in_price), Some(out_price)) => Some(
            ((estimated_input_tokens as f64) * in_price + (reserved_output as f64) * out_price)
                / 1_000_000.0,
        ),
        _ => None,
    };
    if let Some(max_cost_usd) = budget.max_cost_usd {
        let (Some(in_price), Some(out_price)) = (
            budget.input_cost_per_million_usd,
            budget.output_cost_per_million_usd,
        ) else {
            return Err(
                "max_cost_usd ceiling is set but input_cost_per_million_usd / output_cost_per_million_usd are not configured"
                    .to_string(),
            );
        };
        let call_reserve_cost = reserved_cost_usd.unwrap_or(0.0);
        let conservative_prior_output =
            if usage.cost_status == PlanningCostStatus::UnknownMissingProviderUsage {
                usage
                    .output_tokens
                    .max(u64::from(usage.model_calls).saturating_mul(reserved_output))
            } else {
                usage.output_tokens
            };
        let token_floor_cost = ((usage.input_tokens as f64) * in_price
            + (conservative_prior_output as f64) * out_price)
            / 1_000_000.0;
        let current_cost = usage
            .reserved_cost_usd
            .or(usage.estimated_cost_usd)
            .unwrap_or(0.0)
            .max(token_floor_cost);
        if current_cost + call_reserve_cost > max_cost_usd + 1e-12 {
            return Err(format!(
                "USD cost ceiling cannot cover call (used ${current_cost:.6} + reserve ${call_reserve_cost:.6} > max ${max_cost_usd:.6})"
            ));
        }
    }
    Ok(PlanningCallReservation {
        estimated_input_tokens,
        reserved_output_tokens: reserved_output,
        reserved_total_tokens: reserved_total,
        reserved_cost_usd,
    })
}

impl Repository {
    pub async fn validate_planning_stage_input(
        &self,
        input: StagePlanningInput,
        rating_ceiling: Option<spindle_core::models::ContentRating>,
    ) -> Result<Vec<PlanningFinding>> {
        self.pool()
            .read(move |conn| Ok(validate_stage_input_in_conn(conn, &input, rating_ceiling)))
            .await?
    }

    pub(crate) async fn revalidate_staged_planning_proposal(
        &self,
        session_id: &str,
        proposal_id: &str,
    ) -> Result<Vec<PlanningFinding>> {
        let session_id = session_id.to_owned();
        let proposal_id = proposal_id.to_owned();
        self.pool()
            .read(move |conn| {
                Ok((|| -> Result<Vec<PlanningFinding>> {
                    let s = session(conn, &session_id)?;
                    let all = proposals(conn, &s.id)?;
                    let p = all
                        .iter()
                        .find(|p| p.id == proposal_id)
                        .context("proposal not in planning session")?;
                    let applied: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM planning_operation_receipt WHERE proposal_id=?1 AND outcome='applied'",
                        [&p.id],
                        |r| r.get(0),
                    )?;
                    if applied > 0 || p.status != PlanningProposalStatus::Staged {
                        return Ok(p.findings.clone());
                    }
                    validate_stage_input_in_conn_excluding(
                        conn,
                        &p.artifact,
                        None,
                        Some(&p.id),
                    )
                })())
            })
            .await?
    }

    pub async fn evaluate_planning_readiness(
        &self,
        session_id: &str,
        target_range_override: Option<PlanningTargetRange>,
    ) -> Result<(
        PlanningSession,
        PlanningTargetRange,
        Vec<PlanningFinding>,
        PlanningPacingMetrics,
    )> {
        let session_id = session_id.to_owned();
        self.pool()
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    let s = session(conn, &session_id)?;
                    check_scope(conn, &s.scope)?;
                    let target_range =
                        target_range_override.unwrap_or_else(|| s.horizon.clone());
                    let all = proposals(conn, &s.id)?;
                    let mut findings = Vec::new();

                    // Any unapplied staged proposals in the session are blocking for final drafting readiness.
                    for p in &all {
                        if p.status == PlanningProposalStatus::Staged {
                            findings.push(PlanningFinding {
                                code: "unapplied_staged_proposal".to_string(),
                                severity: PlanningFindingSeverity::Blocking,
                                path: format!("/proposals/{}", p.id),
                                related_refs: vec![p.id.clone()],
                                expected: "all proposals in session applied or decided"
                                    .to_string(),
                                actual: format!("proposal {} ({:?}) is still staged", p.id, p.artifact.level),
                                repair_hint:
                                    "Accept, reject, or supersede staged proposals before starting an authoring run."
                                        .to_string(),
                            });
                        }
                        if p.status == PlanningProposalStatus::Applied {
                            for f in &p.findings {
                                if !findings.contains(f) {
                                    findings.push(f.clone());
                                }
                            }
                        }
                    }

                    if s.projection_stale {
                        findings.push(PlanningFinding {
                            code: "stale_planning_projection".to_string(),
                            severity: PlanningFindingSeverity::Blocking,
                            path: "/session/projection_stale".to_string(),
                            related_refs: Vec::new(),
                            expected: "reconciled planning projection".to_string(),
                            actual: s
                                .stale_reason
                                .clone()
                                .unwrap_or_else(|| "projection_stale=true".to_string()),
                            repair_hint:
                                "Accept or supersede the staged reconciliation proposal before drafting."
                                    .to_string(),
                        });
                    }

                    let receipt_refs = apply::refs_in_conn(conn, &s.id).unwrap_or_default();
                    let mut ctx = ValidatorContext::default();
                    for (out_ref, db_id) in &receipt_refs {
                        ctx.id_to_ref.insert(db_id.clone(), out_ref.clone());
                    }
                    if let Some(ref scope) = s.scope {
                        ctx.load_from_db(conn, scope)?;
                    }
                    let mut all_intensity = Vec::new();
                    for p in &all {
                        if p.status == PlanningProposalStatus::Applied {
                            for op in &p.artifact.operations {
                                ctx.ingest_operation(&op.output_ref, &op.operation);
                            }
                            all_intensity.extend(p.artifact.schedule.intensity_targets.clone());
                            for pov in &p.artifact.schedule.pov_plan {
                                if pov.scene_order.is_none() {
                                    ctx.chapter_povs.insert(
                                        (pov.book_number, pov.chapter_number),
                                        ctx.normalize_ref(&pov.pov_character_id),
                                    );
                                }
                            }
                        }
                    }

                    // Verify persisted SQLite chapter plans across target_range
                    if let Some(ref scope) = s.scope {
                        let lineage = resolve_branch_lineage_in_conn(
                            conn,
                            Some(&scope.project_id),
                            &scope.branch_id,
                        )?;
                        for ch_no in target_range.chapter_start..=target_range.chapter_end {
                            let mut found_plan: Option<(String, Option<String>, String)> = None;
                            for branch_id in &lineage {
                                if let Ok(row) = conn.query_row(
                                    "SELECT synopsis, pov_character_id, scenes FROM chapter_plan WHERE project_id=?1 AND branch_id=?2 AND book_number=?3 AND chapter_number=?4",
                                    rusqlite::params![
                                        scope.project_id,
                                        branch_id,
                                        target_range.book_number,
                                        ch_no
                                    ],
                                    |r| {
                                        Ok((
                                            r.get::<_, String>(0)?,
                                            r.get::<_, Option<String>>(1)?,
                                            r.get::<_, String>(2)?,
                                        ))
                                    },
                                ) {
                                    found_plan = Some(row);
                                    break;
                                }
                            }
                            match found_plan {
                                None => {
                                    findings.push(PlanningFinding {
                                        code: "missing_target_chapter_manifest".to_string(),
                                        severity: PlanningFindingSeverity::Blocking,
                                        path: format!(
                                            "/chapter_plans/{}/{}",
                                            target_range.book_number, ch_no
                                        ),
                                        related_refs: Vec::new(),
                                        expected: format!(
                                            "persisted chapter_plan for book {} chapter {}",
                                            target_range.book_number, ch_no
                                        ),
                                        actual: "missing".to_string(),
                                        repair_hint: format!(
                                            "Plan and apply scene manifests for book {} chapter {}.",
                                            target_range.book_number, ch_no
                                        ),
                                    });
                                }
                                Some((synopsis, pov_opt, scenes_json)) => {
                                    if let Some(pov) = pov_opt {
                                        ctx.chapter_povs.insert(
                                            (target_range.book_number, ch_no),
                                            ctx.normalize_str(&pov),
                                        );
                                    }
                                    if synopsis.trim().is_empty() {
                                        findings.push(PlanningFinding {
                                            code: "empty_chapter_synopsis".to_string(),
                                            severity: PlanningFindingSeverity::Blocking,
                                            path: format!(
                                                "/chapter_plans/{}/{}/synopsis",
                                                target_range.book_number, ch_no
                                            ),
                                            related_refs: Vec::new(),
                                            expected: "non-empty chapter synopsis".to_string(),
                                            actual: "empty".to_string(),
                                            repair_hint:
                                                "Provide a non-empty synopsis for the chapter plan."
                                                    .to_string(),
                                        });
                                    }
                                    let scenes: Vec<spindle_core::models::PlanChapterSceneInput> =
                                        serde_json::from_str(&scenes_json).unwrap_or_default();
                                    if scenes.is_empty() {
                                        findings.push(PlanningFinding {
                                            code: "empty_scene_manifest".to_string(),
                                            severity: PlanningFindingSeverity::Blocking,
                                            path: format!(
                                                "/chapter_plans/{}/{}/scenes",
                                                target_range.book_number, ch_no
                                            ),
                                            related_refs: Vec::new(),
                                            expected: format!(
                                                "{} planned scenes",
                                                target_range.scenes_per_chapter
                                            ),
                                            actual: "0 scenes".to_string(),
                                            repair_hint:
                                                "Populate scene manifests for the chapter before drafting."
                                                    .to_string(),
                                        });
                                    } else if target_range.scenes_per_chapter >= 1
                                        && scenes.len()
                                            != target_range.scenes_per_chapter as usize
                                    {
                                        findings.push(PlanningFinding {
                                            code: "scene_count_mismatch".to_string(),
                                            severity: PlanningFindingSeverity::Blocking,
                                            path: format!(
                                                "/chapter_plans/{}/{}/scenes",
                                                target_range.book_number, ch_no
                                            ),
                                            related_refs: Vec::new(),
                                            expected: format!(
                                                "{} scenes",
                                                target_range.scenes_per_chapter
                                            ),
                                            actual: format!("{} scenes", scenes.len()),
                                            repair_hint: format!(
                                                "Update chapter {} scene manifest count to match target_range.scenes_per_chapter ({}).",
                                                ch_no, target_range.scenes_per_chapter
                                            ),
                                        });
                                    }
                                }
                            }
                        }
                    }

                    let metrics = compute_pov_and_pacing_metrics(
                        &ctx,
                        &s.policy,
                        target_range.book_number.max(1),
                        &ctx.chapter_povs,
                        &all_intensity,
                        &mut findings,
                        true,
                    );

                    Ok((s, target_range, findings, metrics))
                })())
            })
            .await?
    }

    pub async fn reconcile_planning_sessions_for_amendment(
        &self,
        amendment: &super::records::StoredPlanAmendment,
        effective_payload: &Value,
        active_branch_id: &str,
        decided_by: &str,
    ) -> Result<()> {
        let amendment = amendment.clone();
        let effective_payload = effective_payload.clone();
        let active_branch_id = active_branch_id.to_owned();
        let decided_by = decided_by.to_owned();
        self.pool()
            .write(move |conn| {
                Ok((|| -> Result<()> {
                    let tx = conn.transaction()?;
                    let session_ids = {
                        let mut stmt = tx.prepare(
                            "SELECT id FROM planning_session WHERE project_id=?1 AND branch_id=?2 ORDER BY created_at, id",
                        )?;
                        stmt.query_map(
                            rusqlite::params![amendment.project_id, active_branch_id],
                            |r| r.get::<_, String>(0),
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    if session_ids.is_empty() {
                        return Ok(());
                    }

                    let now = timestamp_to_micros(chrono::Utc::now());
                    let target_book = amendment.book_number;
                    let target_chapter = amendment.target_chapter;

                    for sid in session_ids {
                        let mut s = session(&tx, &sid)?;
                        let mut all = proposals(&tx, &sid)?;

                        // 1. Identify any Staged proposals that target the amended chapter or promise
                        let mut conflicted_ids = std::collections::BTreeSet::new();
                        for p in &all {
                            if p.status != PlanningProposalStatus::Staged {
                                continue;
                            }
                            let touches_target = p.artifact.operations.iter().any(|op| {
                                match &op.operation {
                                    PlanningOperation::PlanChapter(plan) => {
                                        Some(plan.book_number) == Some(target_book)
                                            && Some(plan.chapter_number) == target_chapter
                                    }
                                    PlanningOperation::SetChapterOutline(co) => {
                                        Some(co.book_number) == Some(target_book)
                                            && Some(co.chapter_number) == target_chapter
                                    }
                                    PlanningOperation::CreateChapter(ch) => {
                                        ch.book_number.unwrap_or(1) == target_book
                                            && ch.chapter_number == target_chapter
                                    }
                                    PlanningOperation::CreateNarrativePromise(_) => {
                                        amendment.amendment_class == "promise_followup"
                                    }
                                    _ => false,
                                }
                            });
                            if touches_target {
                                conflicted_ids.insert(p.id.clone());
                            }
                        }
                        loop {
                            let before = conflicted_ids.len();
                            for p in &all {
                                if p.status == PlanningProposalStatus::Staged
                                    && p.artifact
                                        .parents
                                        .keys()
                                        .any(|pid| conflicted_ids.contains(pid))
                                {
                                    conflicted_ids.insert(p.id.clone());
                                }
                            }
                            if conflicted_ids.len() == before {
                                break;
                            }
                        }

                        for p in &mut all {
                            if conflicted_ids.contains(&p.id) {
                                p.status = PlanningProposalStatus::Superseded;
                                p.decided_by = Some(decided_by.clone());
                                p.decided_at = Some(now);
                                p.decision_note = Some(format!(
                                    "Superseded by applied plan amendment {} ({})",
                                    amendment.id, amendment.amendment_class
                                ));
                                save(&tx, p)?;
                            } else if p.status == PlanningProposalStatus::Staged {
                                p.base_state = apply::base_state(&tx, &s.scope, &p.artifact)?;
                                save(&tx, p)?;
                            }
                        }

                        // 2. Check if any Applied proposal in this session had planner schedule
                        // projections (promise_events or intensity_targets) or chapter manifests
                        // affected by this amendment.
                        let has_schedule_projection = all.iter().any(|p| {
                            p.status == PlanningProposalStatus::Applied
                                && (!p.artifact.schedule.promise_events.is_empty()
                                    || !p.artifact.schedule.intensity_targets.is_empty()
                                    || !p.artifact.schedule.pov_plan.is_empty())
                        });
                        let affects_schedule = amendment.amendment_class == "promise_followup"
                            || (has_schedule_projection
                                && matches!(
                                    amendment.amendment_class.as_str(),
                                    "scene_add"
                                        | "scene_drop"
                                        | "scene_replace"
                                        | "scene_reorder"
                                        | "pov_change"
                                        | "thread_promote"
                                ));

                        if affects_schedule {
                            s.projection_stale = true;
                            s.stale_reason = Some(format!(
                                "Plan amendment {} ({}) modified book {} chapter {:?}; planner schedule projection requires reconciliation",
                                amendment.id,
                                amendment.amendment_class,
                                target_book,
                                target_chapter
                            ));
                            tx.execute(
                                "UPDATE planning_session SET data=?2 WHERE id=?1",
                                rusqlite::params![s.id, serde_json::to_string(&s)?],
                            )?;

                            // If the amendment targeted a chapter with a persisted chapter_plan,
                            // stage a reconciliation proposal in the session so the operator can
                            // review and accept or supersede the reconciled chapter projection.
                            if let Some(ch_no) = target_chapter
                                && let Ok((synopsis, pov_opt, themes_json, conflicts_json, plots_json, scenes_json)) =
                                    tx.query_row(
                                        "SELECT synopsis, pov_character_id, target_theme_ids, target_conflict_ids, target_plot_line_ids, scenes FROM chapter_plan WHERE project_id=?1 AND branch_id=?2 AND book_number=?3 AND chapter_number=?4",
                                        rusqlite::params![
                                            amendment.project_id,
                                            active_branch_id,
                                            target_book,
                                            ch_no
                                        ],
                                        |r| {
                                            Ok((
                                                r.get::<_, String>(0)?,
                                                r.get::<_, Option<String>>(1)?,
                                                r.get::<_, String>(2)?,
                                                r.get::<_, String>(3)?,
                                                r.get::<_, String>(4)?,
                                                r.get::<_, String>(5)?,
                                            ))
                                        },
                                    )
                            {
                                let raw_themes: Vec<String> =
                                    serde_json::from_str(&themes_json).unwrap_or_default();
                                let raw_conflicts: Vec<String> =
                                    serde_json::from_str(&conflicts_json).unwrap_or_default();
                                let raw_plots: Vec<String> =
                                    serde_json::from_str(&plots_json).unwrap_or_default();
                                let raw_scenes: Vec<spindle_core::models::PlanChapterSceneInput> =
                                    serde_json::from_str(&scenes_json).unwrap_or_default();
                                let planned_scenes: Vec<PlanningPlanChapterSceneInput> = raw_scenes
                                    .into_iter()
                                    .map(|sc| PlanningPlanChapterSceneInput {
                                        scene_order: sc.scene_order,
                                        summary: sc.summary,
                                        beat_structure: sc.beat_structure,
                                        character_ids: sc
                                            .character_ids
                                            .into_iter()
                                            .map(PlanningRef::ExistingRecordId)
                                            .collect(),
                                        location_id: sc
                                            .location_id
                                            .map(PlanningRef::ExistingRecordId),
                                        content_rating: sc.content_rating,
                                        purpose: sc.purpose,
                                        research_required: sc.research_required,
                                        research_tags: sc.research_tags,
                                        explicit_query: sc.explicit_query,
                                    })
                                    .collect();
                                let out_ref = format!(
                                    "reconcile:ch{target_book}_{ch_no}:{}",
                                    amendment.id.replace(':', "_")
                                );
                                let recon_level = if planned_scenes.is_empty() {
                                    PlanningLevel::Chapter
                                } else {
                                    PlanningLevel::Scene
                                };
                                let wanted_parent = match recon_level {
                                    PlanningLevel::Chapter => PlanningLevel::Book,
                                    PlanningLevel::Scene => PlanningLevel::Chapter,
                                    _ => PlanningLevel::Book,
                                };
                                let mut recon_parents = std::collections::BTreeMap::new();
                                if let Some(parent_prop) = all
                                    .iter()
                                    .rev()
                                    .find(|p| {
                                        p.status == PlanningProposalStatus::Applied
                                            && p.artifact.level == wanted_parent
                                    })
                                    .or_else(|| {
                                        all.iter().rev().find(|p| {
                                            p.status == PlanningProposalStatus::Applied
                                                && p.artifact.level == PlanningLevel::Book
                                        })
                                    })
                                {
                                    recon_parents
                                        .insert(parent_prop.id.clone(), parent_prop.payload_hash.clone());
                                }
                                let stage_input = StagePlanningInput {
                                    session_id: s.id.clone(),
                                    scope: s.scope.clone(),
                                    intent_hash: s.intent_hash.clone(),
                                    schema_version: 1,
                                    level: recon_level,
                                    parents: recon_parents,
                                    operations: vec![PlanningOperationItem {
                                        output_ref: out_ref,
                                        operation: PlanningOperation::PlanChapter(
                                            PlanningPlanChapterInput {
                                                book_number: target_book,
                                                chapter_number: ch_no,
                                                pov_character_id: pov_opt
                                                    .map(PlanningRef::ExistingRecordId),
                                                synopsis,
                                                target_theme_ids: raw_themes
                                                    .into_iter()
                                                    .map(PlanningRef::ExistingRecordId)
                                                    .collect(),
                                                target_conflict_ids: raw_conflicts
                                                    .into_iter()
                                                    .map(PlanningRef::ExistingRecordId)
                                                    .collect(),
                                                target_plot_line_ids: raw_plots
                                                    .into_iter()
                                                    .map(PlanningRef::ExistingRecordId)
                                                    .collect(),
                                                scenes: planned_scenes,
                                            },
                                        ),
                                    }],
                                    rationale: format!(
                                        "Reconcile planner projection after applied amendment {} ({}): {}",
                                        amendment.id,
                                        amendment.amendment_class,
                                        effective_payload
                                    ),
                                    target_range: None,
                                    schedule: PlanningSchedule::default(),
                                    assumptions: vec![format!(
                                        "Reconciled from applied plan amendment {}",
                                        amendment.id
                                    )],
                                    author_questions: Vec::new(),
                                };
                                let base_state =
                                    apply::base_state(&tx, &s.scope, &stage_input)?;
                                let recon_prop = PlanningProposal {
                                    id: mint_id("planning_proposal"),
                                    payload_hash: hash(&stage_input)?,
                                    artifact: stage_input,
                                    status: PlanningProposalStatus::Staged,
                                    policy_snapshot: s.policy.clone(),
                                    base_state,
                                    generation_receipt: None,
                                    findings: Vec::new(),
                                    decided_by: None,
                                    decision_note: None,
                                    decided_at: None,
                                    created_at: now,
                                };
                                tx.execute(
                                    "INSERT INTO planning_proposal(id,session_id,payload_hash,status,data,created_at) VALUES(?1,?2,?3,'staged',?4,?5)",
                                    rusqlite::params![
                                        recon_prop.id,
                                        s.id,
                                        recon_prop.payload_hash,
                                        serde_json::to_string(&recon_prop)?,
                                        recon_prop.created_at
                                    ],
                                )?;
                            }
                        }
                    }

                    tx.commit()?;
                    Ok(())
                })())
            })
            .await?
    }

    pub async fn find_planning_generation_attempt(
        &self,
        session_id: &str,
        retry_key: &str,
    ) -> Result<Option<PlanningAdvanceReport>> {
        let session_id = session_id.to_owned();
        let retry_key = retry_key.to_owned();
        let db_path = self.pool().db_path().to_path_buf();
        self.pool()
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT outcome, claim_token, lease_expires_at, data FROM planning_generation_attempt WHERE session_id=?1 AND retry_key=?2 LIMIT 1",
                )?;
                let mut rows = stmt.query(rusqlite::params![session_id, retry_key])?;
                let Some(row) = rows.next()? else {
                    return Ok(Ok(None));
                };
                let outcome: String = row.get(0)?;
                let claim_token: Option<String> = row.get(1)?;
                let lease_expires_at: Option<i64> = row.get(2)?;
                let data: String = row.get(3)?;
                Ok((|| -> Result<_> {
                    let now = timestamp_to_micros(chrono::Utc::now());
                    let lease_unexpired = lease_expires_at.is_none_or(|exp| now < exp);
                    if outcome == "in_progress"
                        && lease_unexpired
                        && super::is_planning_attempt_lock_held(&db_path, claim_token.as_deref())?
                    {
                        return Ok(None);
                    }
                    Ok(Some(serde_json::from_str(&data)?))
                })())
            })
            .await?
    }

    pub async fn list_planning_generation_attempts(
        &self,
        session_id: &str,
        scope: Option<PlanningScope>,
    ) -> Result<Vec<PlanningAdvanceReport>> {
        let session_id = session_id.to_owned();
        let db_path = self.pool().db_path().to_path_buf();
        self.pool()
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    check_session(conn, &session_id, &scope)?;
                    let now = timestamp_to_micros(chrono::Utc::now());
                    let mut stmt = conn.prepare(
                        "SELECT outcome, claim_token, lease_expires_at, data FROM planning_generation_attempt WHERE session_id=?1 ORDER BY created_at, id",
                    )?;
                    let rows = stmt
                        .query_map([&session_id], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, Option<String>>(1)?,
                                r.get::<_, Option<i64>>(2)?,
                                r.get::<_, String>(3)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    let mut out = Vec::new();
                    for (outcome, claim_token, lease_expires_at, data) in rows {
                        let lease_unexpired = lease_expires_at.is_none_or(|exp| now < exp);
                        if outcome == "in_progress"
                            && lease_unexpired
                            && super::is_planning_attempt_lock_held(
                                &db_path,
                                claim_token.as_deref(),
                            )?
                        {
                            continue;
                        }
                        out.push(serde_json::from_str(&data)?);
                    }
                    Ok(out)
                })())
            })
            .await?
    }

    pub(crate) async fn claim_planning_generation_attempt(
        &self,
        session_id: &str,
        scope: Option<PlanningScope>,
        retry_key: Option<String>,
    ) -> Result<PlanningAttemptClaim> {
        let session_id = session_id.to_owned();
        let db_path = self.pool().db_path().to_path_buf();
        self.pool()
            .write(move |conn| {
                Ok((|| -> Result<PlanningAttemptClaim> {
                    let tx = conn.transaction()?;
                    let s = check_session(&tx, &session_id, &scope)?;
                    let now = timestamp_to_micros(chrono::Utc::now());
                    let lease_ttl_micros: i64 = 300_000_000;
                    if let Some(rk) = retry_key.as_deref() {
                        type ExistingAttemptRow =
                            (String, String, Option<String>, Option<i64>, String);
                        let existing: Option<ExistingAttemptRow> = tx
                            .query_row(
                                "SELECT id, outcome, claim_token, lease_expires_at, data FROM planning_generation_attempt WHERE session_id=?1 AND retry_key=?2 LIMIT 1",
                                rusqlite::params![session_id, rk],
                                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                            )
                            .optional_inner()?;
                        if let Some((existing_id, outcome, existing_token, lease_expires_at, data)) = existing {
                            if outcome != "in_progress" {
                                let report: PlanningAdvanceReport = serde_json::from_str(&data)?;
                                return Ok(PlanningAttemptClaim::Existing(Box::new(report)));
                            }
                            let lease_unexpired =
                                lease_expires_at.is_none_or(|exp| now < exp);
                            anyhow::ensure!(
                                !(lease_unexpired && super::is_planning_attempt_lock_held(&db_path, existing_token.as_deref())?),
                                "planning generation attempt for retry_key '{rk}' is already in progress"
                            );
                            let claim_token = mint_id("planning_claim");
                            let active_guard = super::acquire_planning_attempt_lock(&db_path, &claim_token)?;
                            let created_at = now;
                            let next_lease = created_at + lease_ttl_micros;
                            let recovered_report = PlanningAdvanceReport {
                                attempt_id: existing_id.clone(),
                                session_id: session_id.clone(),
                                level: PlanningLevel::Contract,
                                retry_key: retry_key.clone(),
                                outcome: PlanningSessionOutcome::NeedsRoute,
                                proposal: None,
                                generation_receipt: None,
                                findings: Vec::new(),
                                author_questions: Vec::new(),
                                host_request: None,
                                route_problem: None,
                                usage: s.usage.clone(),
                                budget: s.budget.clone(),
                                message: Some(
                                    "generation attempt was interrupted before completion; any reserved call budget was retained"
                                        .to_string(),
                                ),
                                created_at,
                            };
                            tx.execute(
                                "UPDATE planning_generation_attempt SET claim_token=?2, lease_expires_at=?3, data=?4, created_at=?5 WHERE id=?1 AND outcome='in_progress'",
                                rusqlite::params![
                                    existing_id,
                                    claim_token,
                                    next_lease,
                                    serde_json::to_string(&recovered_report)?,
                                    created_at
                                ],
                            )?;
                            tx.commit()?;
                            return Ok(PlanningAttemptClaim::Claimed {
                                attempt_id: existing_id,
                                claim_token,
                                active_guard,
                                created_at,
                                session: Box::new(s),
                            });
                        }
                    }
                    let attempt_id = mint_id("planning_generation_attempt");
                    let claim_token = mint_id("planning_claim");
                    let active_guard = super::acquire_planning_attempt_lock(&db_path, &claim_token)?;
                    let created_at = now;
                    let next_lease = created_at + lease_ttl_micros;
                    let initial_report = PlanningAdvanceReport {
                        attempt_id: attempt_id.clone(),
                        session_id: session_id.clone(),
                        level: PlanningLevel::Contract,
                        retry_key: retry_key.clone(),
                        outcome: PlanningSessionOutcome::NeedsRoute,
                        proposal: None,
                        generation_receipt: None,
                        findings: Vec::new(),
                        author_questions: Vec::new(),
                        host_request: None,
                        route_problem: None,
                        usage: s.usage.clone(),
                        budget: s.budget.clone(),
                        message: Some(
                            "generation attempt was interrupted before completion; any reserved call budget was retained"
                                .to_string(),
                        ),
                        created_at,
                    };
                    tx.execute(
                        "INSERT INTO planning_generation_attempt(id,session_id,level,retry_key,outcome,proposal_id,claim_token,lease_expires_at,data,created_at) VALUES(?1,?2,'contract',?3,'in_progress',NULL,?4,?5,?6,?7)",
                        rusqlite::params![
                            attempt_id,
                            session_id,
                            retry_key,
                            claim_token,
                            next_lease,
                            serde_json::to_string(&initial_report)?,
                            created_at
                        ],
                    )?;
                    tx.commit()?;
                    Ok(PlanningAttemptClaim::Claimed {
                        attempt_id,
                        claim_token,
                        active_guard,
                        created_at,
                        session: Box::new(s),
                    })
                })())
            })
            .await?
    }

    pub(crate) async fn abort_planning_generation_attempt(
        &self,
        attempt_id: &str,
        claim_token: &str,
    ) -> Result<()> {
        let attempt_id = attempt_id.to_owned();
        let claim_token = claim_token.to_owned();
        self.pool()
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM planning_generation_attempt WHERE id=?1 AND outcome='in_progress' AND claim_token=?2",
                    rusqlite::params![attempt_id, claim_token],
                )?;
                Ok(())
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reserve_planning_model_call(
        &self,
        attempt_id: &str,
        claim_token: &str,
        level: PlanningLevel,
        session_id: &str,
        scope: Option<PlanningScope>,
        budget: PlanningBudget,
        estimated_input_tokens: u64,
    ) -> Result<
        std::result::Result<
            (
                PlanningCallReservation,
                PlanningBudgetUsage,
                PlanningBudgetUsage,
            ),
            (String, PlanningBudgetUsage),
        >,
    > {
        let attempt_id = attempt_id.to_owned();
        let claim_token = claim_token.to_owned();
        let session_id = session_id.to_owned();
        self.pool()
            .write(move |conn| {
                Ok((|| -> Result<_> {
                    let tx = conn.transaction()?;
                    let existing_attempt: Option<(String, Option<String>, String)> = tx
                        .query_row(
                            "SELECT outcome, claim_token, data FROM planning_generation_attempt WHERE id=?1",
                            [&attempt_id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                        )
                        .optional_inner()?;
                    let Some((existing_outcome, existing_token, existing_data)) = existing_attempt
                    else {
                        anyhow::bail!("planning generation claim '{attempt_id}' does not exist");
                    };
                    anyhow::ensure!(
                        existing_outcome == "in_progress"
                            && existing_token.as_deref() == Some(claim_token.as_str()),
                        "stale or already finalized planning generation claim"
                    );
                    let mut s = check_session(&tx, &session_id, &scope)?;
                    let usage_before = s.usage.clone();
                    match check_budget_in_conn(&budget, &s.usage, estimated_input_tokens) {
                        Err(reason) => Ok(Err((reason, s.usage))),
                        Ok(res) => {
                            s.usage.model_calls = s.usage.model_calls.saturating_add(1);
                            s.usage.input_tokens = s
                                .usage
                                .input_tokens
                                .saturating_add(res.estimated_input_tokens);
                            s.usage.output_tokens = s
                                .usage
                                .output_tokens
                                .saturating_add(res.reserved_output_tokens);
                            s.usage.total_tokens = s
                                .usage
                                .total_tokens
                                .saturating_add(res.reserved_total_tokens);
                            if let Some(rc) = res.reserved_cost_usd {
                                s.usage.reserved_cost_usd =
                                    Some(s.usage.reserved_cost_usd.unwrap_or(0.0) + rc);
                            }
                            s.budget = budget;
                            tx.execute(
                                "UPDATE planning_session SET data=?2 WHERE id=?1",
                                rusqlite::params![s.id, serde_json::to_string(&s)?],
                            )?;
                            if let Ok(mut pending_report) =
                                serde_json::from_str::<PlanningAdvanceReport>(&existing_data)
                            {
                                pending_report.level = level;
                                pending_report.usage = s.usage.clone();
                                pending_report.budget = s.budget.clone();
                                let level_str = serde_json::to_value(level)?
                                    .as_str()
                                    .unwrap_or("contract")
                                    .to_string();
                                let next_lease = timestamp_to_micros(chrono::Utc::now()) + 300_000_000;
                                tx.execute(
                                    "UPDATE planning_generation_attempt SET level=?2, lease_expires_at=?3, data=?4 WHERE id=?1 AND outcome='in_progress' AND claim_token=?5",
                                    rusqlite::params![
                                        attempt_id,
                                        level_str,
                                        next_lease,
                                        serde_json::to_string(&pending_report)?,
                                        claim_token
                                    ],
                                )?;
                            }
                            tx.commit()?;
                            Ok(Ok((res, usage_before, s.usage)))
                        }
                    }
                })())
            })
            .await?
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reconcile_planning_model_call(
        &self,
        session_id: &str,
        scope: Option<PlanningScope>,
        reservation: PlanningCallReservation,
        actual_input_tokens: u64,
        actual_output_tokens: u64,
        actual_total_tokens: u64,
        call_estimated_cost_usd: Option<f64>,
        conservative_call_cost_usd: Option<f64>,
        call_cost_status: PlanningCostStatus,
    ) -> Result<PlanningBudgetUsage> {
        let session_id = session_id.to_owned();
        self.pool()
            .write(move |conn| {
                Ok((|| -> Result<PlanningBudgetUsage> {
                    let tx = conn.transaction()?;
                    let mut s = check_session(&tx, &session_id, &scope)?;
                    s.usage.input_tokens = s
                        .usage
                        .input_tokens
                        .saturating_sub(reservation.estimated_input_tokens)
                        .saturating_add(actual_input_tokens);
                    s.usage.output_tokens = s
                        .usage
                        .output_tokens
                        .saturating_sub(reservation.reserved_output_tokens)
                        .saturating_add(actual_output_tokens);
                    s.usage.total_tokens = s
                        .usage
                        .total_tokens
                        .saturating_sub(reservation.reserved_total_tokens)
                        .saturating_add(actual_total_tokens);
                    if let Some(rc) = reservation.reserved_cost_usd {
                        let base = (s.usage.reserved_cost_usd.unwrap_or(0.0) - rc).max(0.0);
                        s.usage.reserved_cost_usd = if base > 0.0 { Some(base) } else { None };
                    }
                    if let Some(cc) = conservative_call_cost_usd {
                        s.usage.reserved_cost_usd =
                            Some(s.usage.reserved_cost_usd.unwrap_or(0.0) + cc);
                    }
                    match (
                        s.usage.cost_status,
                        call_cost_status,
                        call_estimated_cost_usd,
                    ) {
                        (_, PlanningCostStatus::UnknownMissingProviderUsage, _)
                        | (PlanningCostStatus::UnknownMissingProviderUsage, _, _) => {
                            s.usage.estimated_cost_usd = None;
                            s.usage.cost_status = PlanningCostStatus::UnknownMissingProviderUsage;
                        }
                        (_, PlanningCostStatus::UnknownUnpriced, _) => {
                            s.usage.estimated_cost_usd = None;
                            s.usage.cost_status = PlanningCostStatus::UnknownUnpriced;
                        }
                        (_, PlanningCostStatus::EstimatedFromConfiguredPrices, Some(c)) => {
                            s.usage.estimated_cost_usd =
                                Some(s.usage.estimated_cost_usd.unwrap_or(0.0) + c);
                            s.usage.cost_status = PlanningCostStatus::EstimatedFromConfiguredPrices;
                        }
                        _ => {}
                    }
                    tx.execute(
                        "UPDATE planning_session SET data=?2 WHERE id=?1",
                        rusqlite::params![s.id, serde_json::to_string(&s)?],
                    )?;
                    tx.commit()?;
                    Ok(s.usage)
                })())
            })
            .await?
    }

    pub(crate) async fn release_planning_model_call_reservation(
        &self,
        session_id: &str,
        scope: Option<PlanningScope>,
        reservation: PlanningCallReservation,
    ) -> Result<()> {
        let session_id = session_id.to_owned();
        self.pool()
            .write(move |conn| {
                Ok((|| -> Result<()> {
                    let tx = conn.transaction()?;
                    let mut s = check_session(&tx, &session_id, &scope)?;
                    s.usage.model_calls = s.usage.model_calls.saturating_sub(1);
                    s.usage.input_tokens = s
                        .usage
                        .input_tokens
                        .saturating_sub(reservation.estimated_input_tokens);
                    s.usage.output_tokens = s
                        .usage
                        .output_tokens
                        .saturating_sub(reservation.reserved_output_tokens);
                    s.usage.total_tokens = s
                        .usage
                        .total_tokens
                        .saturating_sub(reservation.reserved_total_tokens);
                    if let Some(rc) = reservation.reserved_cost_usd {
                        let base = (s.usage.reserved_cost_usd.unwrap_or(0.0) - rc).max(0.0);
                        s.usage.reserved_cost_usd = if base > 0.0 { Some(base) } else { None };
                    }
                    tx.execute(
                        "UPDATE planning_session SET data=?2 WHERE id=?1",
                        rusqlite::params![s.id, serde_json::to_string(&s)?],
                    )?;
                    tx.commit()?;
                    Ok(())
                })())
            })
            .await?
    }

    pub async fn describe_planning_scope_digest(
        &self,
        scope: Option<PlanningScope>,
    ) -> Result<Vec<String>> {
        let Some(scope) = scope else {
            return Ok(Vec::new());
        };
        self.pool()
            .read(move |conn| {
                Ok((|| -> Result<Vec<String>> {
                    let mut out = Vec::new();
                    let project_row: Option<(String, String, String, String)> = conn
                        .query_row(
                            "SELECT name, project_type, genre, reader_contract FROM project WHERE id=?1",
                            [&scope.project_id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                        )
                        .ok();
                    if let Some((name, project_type, genre, reader_contract)) = project_row {
                        out.push(format!(
                            "project {} ({name}, type={project_type}, genre={genre}, reader_contract={reader_contract})",
                            scope.project_id
                        ));
                    }
                    let lineage_cte = "WITH RECURSIVE lineage(id,parent_branch_id) AS (SELECT id,parent_branch_id FROM bible_branch WHERE id=?2 AND project_id=?1 UNION SELECT b.id,b.parent_branch_id FROM bible_branch b JOIN lineage l ON b.id=l.parent_branch_id WHERE b.project_id=?1)";
                    for (table, detail_expr, is_project_scoped) in [
                        (
                            "book",
                            "'book_number=' || CAST(book_number AS TEXT) || ', title=' || COALESCE(title, '')",
                            true,
                        ),
                        (
                            "chapter",
                            "'book=' || CAST(book_number AS TEXT) || ', chapter=' || CAST(chapter_number AS TEXT) || ', title=' || COALESCE(title, '')",
                            true,
                        ),
                        (
                            "character",
                            "name || ', role=' || role || ', summary=' || summary",
                            false,
                        ),
                        (
                            "location",
                            "name || ', kind=' || kind || ', summary=' || summary",
                            false,
                        ),
                        (
                            "world_rule",
                            "rule_name || ' (' || rule_type || '): ' || description",
                            false,
                        ),
                        (
                            "plot_line",
                            "name || ' (' || plot_type || '): ' || summary || ', convergence=' || convergence_points",
                            false,
                        ),
                        (
                            "conflict",
                            "name || ' (' || conflict_type || '): stakes=' || stakes || ', escalation=' || escalation_stages || ', cycles=' || try_fail_cycles",
                            false,
                        ),
                        (
                            "theme",
                            "theme_statement || ' | ' || thesis_antithesis",
                            false,
                        ),
                        ("motif", "name || ': ' || description", false),
                        (
                            "narrative_promise",
                            "promise_type || ': ' || description || ', planted=' || planted_at || ', payoff=' || COALESCE(planned_payoff, 'null')",
                            false,
                        ),
                        (
                            "character_arc",
                            "character_id || ' (' || arc_type || '): ' || starting_state || ' -> ' || ending_state || ', milestones=' || milestones",
                            false,
                        ),
                        (
                            "pacing_config",
                            "'books=' || CAST(total_planned_books AS TEXT) || ', chapters=' || CAST(avg_chapters_per_book AS TEXT) || ', scenes=' || CAST(avg_scenes_per_chapter AS TEXT) || ', model=' || tension_model",
                            false,
                        ),
                        (
                            "pacing_curve",
                            "'book=' || CAST(book_number AS TEXT) || ', breakpoints=' || act_breakpoints || ', density=' || scene_type_density",
                            false,
                        ),
                    ] {
                        let sql = if is_project_scoped {
                            format!(
                                "SELECT id, {detail_expr} FROM {table} WHERE project_id=?1 ORDER BY id LIMIT 32"
                            )
                        } else {
                            format!(
                                "{lineage_cte} SELECT id, {detail_expr} FROM {table} WHERE project_id=?1 AND branch_id IN (SELECT id FROM lineage) ORDER BY id LIMIT 32"
                            )
                        };
                        let mut stmt = conn.prepare(&sql)?;
                        let rows = if is_project_scoped {
                            stmt.query_map([&scope.project_id], |r| {
                                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                        } else {
                            stmt.query_map(
                                [&scope.project_id, &scope.branch_id],
                                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                            )?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                        };
                        for (id, label) in rows {
                            out.push(format!("{table} {id} ({label})"));
                        }
                    }
                    let book_outline_sql = format!(
                        "{lineage_cte} SELECT o.id, 'book=' || CAST(b.book_number AS TEXT) || ', content=' || o.content FROM book_outline o JOIN book b ON b.id=o.book_id WHERE b.project_id=?1 AND o.branch_id IN (SELECT id FROM lineage) ORDER BY b.book_number, o.id LIMIT 32"
                    );
                    let mut book_outline_stmt = conn.prepare(&book_outline_sql)?;
                    for row in book_outline_stmt.query_map(
                        [&scope.project_id, &scope.branch_id],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                    )? {
                        let (id, detail) = row?;
                        out.push(format!("book_outline {id} ({detail})"));
                    }
                    let chapter_outline_sql = format!(
                        "{lineage_cte} SELECT o.id, 'book=' || CAST(c.book_number AS TEXT) || ', chapter=' || CAST(c.chapter_number AS TEXT) || ', content=' || o.content || ', beats=' || o.beats FROM chapter_outline o JOIN chapter c ON c.id=o.chapter_id WHERE c.project_id=?1 AND o.branch_id IN (SELECT id FROM lineage) ORDER BY c.book_number, c.chapter_number, o.id LIMIT 32"
                    );
                    let mut chapter_outline_stmt = conn.prepare(&chapter_outline_sql)?;
                    for row in chapter_outline_stmt.query_map(
                        [&scope.project_id, &scope.branch_id],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                    )? {
                        let (id, detail) = row?;
                        out.push(format!("chapter_outline {id} ({detail})"));
                    }
                    Ok(out)
                })())
            })
            .await?
    }

    pub(crate) async fn commit_planning_generation(
        &self,
        claim_token: &str,
        meta: PlanningSessionMetadataUpdate,
        staged: Option<PlanningStagedCommitCandidate>,
        mut report: PlanningAdvanceReport,
    ) -> Result<PlanningAdvanceReport> {
        let claim_token = claim_token.to_owned();
        self.pool()
            .write(move |conn| {
                Ok((|| -> Result<PlanningAdvanceReport> {
                    let tx = conn.transaction()?;
                    let existing_attempt: Option<(String, Option<String>)> = tx
                        .query_row(
                            "SELECT outcome, claim_token FROM planning_generation_attempt WHERE id=?1",
                            [&report.attempt_id],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional_inner()?;
                    let Some((existing_outcome, existing_token)) = existing_attempt else {
                        anyhow::bail!(
                            "planning generation claim '{}' does not exist",
                            report.attempt_id
                        );
                    };
                    anyhow::ensure!(
                        existing_outcome == "in_progress"
                            && existing_token.as_deref() == Some(claim_token.as_str()),
                        "stale or already finalized planning generation claim"
                    );
                    let mut live_session = check_session(&tx, &meta.session_id, &meta.scope)?;
                    live_session.horizon = meta.horizon;
                    live_session.budget = meta.budget;
                    live_session.rating_ceiling = meta.rating_ceiling;
                    live_session.allow_review_backend = meta.allow_review_backend;
                    live_session.last_outcome = Some(meta.last_outcome);
                    report.usage = live_session.usage.clone();
                    report.budget = live_session.budget.clone();
                    let proposal_id = if let Some(mut candidate) = staged {
                        let tx_findings = validate_stage_input_in_conn(
                            &tx,
                            &candidate.input,
                            live_session.rating_ceiling.clone(),
                        )?;
                        let has_blocking = tx_findings
                            .iter()
                            .any(|f| f.severity == PlanningFindingSeverity::Blocking);
                        if has_blocking {
                            candidate.receipt.repaired = false;
                            candidate.fallback_host_request.prior_findings = tx_findings.clone();
                            live_session.last_outcome =
                                Some(PlanningSessionOutcome::ModelOutputRejected);
                            report.outcome = PlanningSessionOutcome::ModelOutputRejected;
                            report.proposal = None;
                            report.generation_receipt = Some(candidate.receipt);
                            report.findings = tx_findings;
                            report.author_questions = Vec::new();
                            report.host_request = Some(candidate.fallback_host_request);
                            report.message = Some(
                                "staging invariants failed during transactional commit".to_string(),
                            );
                            None
                        } else {
                            let merged_findings = if tx_findings.is_empty() {
                                candidate.findings
                            } else {
                                tx_findings
                            };
                            let captured_base_state =
                                apply::base_state(&tx, &live_session.scope, &candidate.input)?;
                            let p = PlanningProposal {
                                id: mint_id("planning_proposal"),
                                payload_hash: hash(&candidate.input)?,
                                artifact: candidate.input,
                                status: PlanningProposalStatus::Staged,
                                policy_snapshot: live_session.policy.clone(),
                                base_state: captured_base_state,
                                generation_receipt: Some(candidate.receipt.clone()),
                                findings: merged_findings.clone(),
                                decided_by: None,
                                decision_note: None,
                                decided_at: None,
                                created_at: report.created_at,
                            };
                            tx.execute(
                                "INSERT INTO planning_proposal(id,session_id,payload_hash,status,data,created_at) VALUES(?1,?2,?3,'staged',?4,?5)",
                                rusqlite::params![
                                    p.id,
                                    p.artifact.session_id,
                                    p.payload_hash,
                                    serde_json::to_string(&p)?,
                                    p.created_at
                                ],
                            )?;
                            let pid = p.id.clone();
                            report.findings = merged_findings;
                            report.proposal = Some(p);
                            Some(pid)
                        }
                    } else {
                        None
                    };
                    tx.execute(
                        "UPDATE planning_session SET data=?2 WHERE id=?1",
                        rusqlite::params![
                            live_session.id,
                            serde_json::to_string(&live_session)?
                        ],
                    )?;
                    let level_str = serde_json::to_value(report.level)?
                        .as_str()
                        .unwrap_or("contract")
                        .to_string();
                    let outcome_str = serde_json::to_value(report.outcome)?
                        .as_str()
                        .unwrap_or("staged")
                        .to_string();
                    let updated = tx.execute(
                        "UPDATE planning_generation_attempt SET level=?2, retry_key=?3, outcome=?4, proposal_id=?5, claim_token=NULL, lease_expires_at=NULL, data=?6, created_at=?7 WHERE id=?1 AND outcome='in_progress' AND claim_token=?8",
                        rusqlite::params![
                            report.attempt_id,
                            level_str,
                            report.retry_key,
                            outcome_str,
                            proposal_id,
                            serde_json::to_string(&report)?,
                            report.created_at,
                            claim_token
                        ],
                    )?;
                    anyhow::ensure!(
                        updated == 1,
                        "stale or already finalized planning generation claim"
                    );
                    tx.commit()?;
                    Ok(report)
                })())
            })
            .await?
    }
}
