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
            last_outcome: None,
            created_at: timestamp_to_micros(chrono::Utc::now()),
        };
        self.pool()
            .write(move |conn| Ok(begin_in_transaction(conn, s)))
            .await?
    }

    pub async fn get_planning_session(&self, id: &str) -> Result<PlanningSession> {
        let id = id.to_owned();
        self.pool().read(move |conn| Ok(session(conn, &id))).await?
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
    let captured_base_state = apply::base_state(&tx, &s.scope, &input)?;
    let p = PlanningProposal {
        id: mint_id("planning_proposal"),
        payload_hash: hash(&input)?,
        artifact: input,
        status: PlanningProposalStatus::Staged,
        policy_snapshot: s.policy,
        base_state: captured_base_state,
        generation_receipt: None,
        findings: Vec::new(),
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

fn placement_key(p: &PlanningStoryPlacement) -> (i32, i32, i32) {
    (p.book_number, p.chapter_number, p.scene_order.unwrap_or(0))
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

fn validate_stage_input_in_conn(
    conn: &rusqlite::Connection,
    input: &StagePlanningInput,
    rating_ceiling: Option<spindle_core::models::ContentRating>,
) -> Result<Vec<PlanningFinding>> {
    let s = check_session(conn, &input.session_id, &input.scope)?;
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
    let old = proposals(conn, &s.id)?;
    if input.level == PlanningLevel::Contract
        && old.iter().any(|p| {
            p.artifact.level == PlanningLevel::Contract
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
                "Cannot stage or regenerate a contract proposal after the root contract has been applied."
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
    let mut available = std::collections::BTreeMap::new();
    let mut reserved = std::collections::BTreeSet::new();
    let mut planted_promises = std::collections::BTreeMap::new();
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
                    if let PlanningOperation::CreateNarrativePromise(np) = &op.operation {
                        planted_promises.insert(op.output_ref.clone(), np.planted_at.clone());
                    }
                }
            }
        }
    }
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
        if let PlanningOperation::CreateNarrativePromise(np) = &item.operation {
            planted_promises.insert(item.output_ref.clone(), np.planted_at.clone());
        }
        if let (Some(ceiling), PlanningOperation::PlanChapter(plan)) =
            (effective_ceiling.as_ref(), &item.operation)
        {
            for (s_idx, scene) in plan.scenes.iter().enumerate() {
                if let Some(scene_rating) = scene.content_rating.as_ref()
                    && rating_rank(scene_rating) > rating_rank(ceiling)
                {
                    findings.push(PlanningFinding {
                        code: "scene_rating_exceeds_ceiling".to_string(),
                        severity: PlanningFindingSeverity::Blocking,
                        path: format!("{item_path}/operation/input/scenes/{s_idx}/content_rating"),
                        related_refs: vec![item.output_ref.clone()],
                        expected: format!("<= {ceiling:?}"),
                        actual: format!("{scene_rating:?}"),
                        repair_hint: "Lower the planned scene content_rating within the session's rating_ceiling.".to_string(),
                    });
                }
            }
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
        for ev in &input.schedule.promise_events {
            if ev.action == PlannedPromiseAction::Plant
                && let PlanningRef::ProposalRef(k) = &ev.promise_ref
            {
                planted_promises
                    .entry(k.clone())
                    .or_insert_with(|| ev.placement.clone());
            }
        }
        for (idx, ev) in input.schedule.promise_events.iter().enumerate() {
            if ev.action == PlannedPromiseAction::Payoff
                && let PlanningRef::ProposalRef(k) = &ev.promise_ref
                && let Some(planted_at) = planted_promises.get(k)
                && placement_key(&ev.placement) < placement_key(planted_at)
            {
                findings.push(PlanningFinding {
                    code: "promise_payoff_before_plant".to_string(),
                    severity: PlanningFindingSeverity::Blocking,
                    path: format!("/schedule/promise_events/{idx}/placement"),
                    related_refs: vec![k.clone()],
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
        for (idx, intensity) in input.schedule.intensity_targets.iter().enumerate() {
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
