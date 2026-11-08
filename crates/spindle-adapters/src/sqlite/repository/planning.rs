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
        self.pool()
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT data FROM planning_generation_attempt WHERE session_id=?1 AND retry_key=?2 LIMIT 1",
                )?;
                let mut rows = stmt.query(rusqlite::params![session_id, retry_key])?;
                let Some(row) = rows.next()? else {
                    return Ok(Ok(None));
                };
                let data: String = row.get(0)?;
                Ok((|| -> Result<_> { Ok(Some(serde_json::from_str(&data)?)) })())
            })
            .await?
    }

    pub async fn list_planning_generation_attempts(
        &self,
        session_id: &str,
        scope: Option<PlanningScope>,
    ) -> Result<Vec<PlanningAdvanceReport>> {
        let session_id = session_id.to_owned();
        self.pool()
            .read(move |conn| {
                Ok((|| -> Result<_> {
                    check_session(conn, &session_id, &scope)?;
                    let mut stmt = conn.prepare(
                        "SELECT data FROM planning_generation_attempt WHERE session_id=?1 ORDER BY created_at, id",
                    )?;
                    let rows = stmt
                        .query_map([&session_id], |r| r.get::<_, String>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    rows.into_iter()
                        .map(|d| Ok(serde_json::from_str(&d)?))
                        .collect()
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
                    let project_row: Option<(String, String)> = conn
                        .query_row(
                            "SELECT name, genre FROM project WHERE id=?1",
                            [&scope.project_id],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .ok();
                    if let Some((name, genre)) = project_row {
                        out.push(format!("project {} ({name}, genre={genre})", scope.project_id));
                    }
                    for (table, label_col) in [
                        ("book", "book_number"),
                        ("chapter", "chapter_number"),
                        ("character", "name"),
                        ("location", "name"),
                        ("world_rule", "rule_name"),
                        ("plot_line", "name"),
                        ("conflict", "name"),
                        ("theme", "theme_statement"),
                        ("motif", "name"),
                        ("narrative_promise", "description"),
                        ("character_arc", "arc_type"),
                    ] {
                        let sql = if matches!(table, "book" | "chapter") {
                            format!(
                                "SELECT id, CAST({label_col} AS TEXT) FROM {table} WHERE project_id=?1 ORDER BY id LIMIT 32"
                            )
                        } else {
                            format!(
                                "SELECT id, CAST({label_col} AS TEXT) FROM {table} WHERE project_id=?1 AND branch_id=?2 ORDER BY id LIMIT 32"
                            )
                        };
                        let mut stmt = conn.prepare(&sql)?;
                        let rows = if matches!(table, "book" | "chapter") {
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
                    Ok(out)
                })())
            })
            .await?
    }

    pub async fn commit_planning_generation(
        &self,
        updated_session: PlanningSession,
        staged: Option<(
            StagePlanningInput,
            PlanningGenerationReceipt,
            Vec<PlanningFinding>,
        )>,
        mut report: PlanningAdvanceReport,
    ) -> Result<PlanningAdvanceReport> {
        self.pool()
            .write(move |conn| {
                Ok((|| -> Result<PlanningAdvanceReport> {
                    let tx = conn.transaction()?;
                    check_session(&tx, &updated_session.id, &updated_session.scope)?;
                    let proposal_id = if let Some((input, receipt, findings)) = staged {
                        let captured_base_state =
                            apply::base_state(&tx, &updated_session.scope, &input)?;
                        let p = PlanningProposal {
                            id: mint_id("planning_proposal"),
                            payload_hash: hash(&input)?,
                            artifact: input,
                            status: PlanningProposalStatus::Staged,
                            policy_snapshot: updated_session.policy.clone(),
                            base_state: captured_base_state,
                            generation_receipt: Some(receipt),
                            findings,
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
                        report.proposal = Some(p);
                        Some(pid)
                    } else {
                        None
                    };
                    tx.execute(
                        "UPDATE planning_session SET data=?2 WHERE id=?1",
                        rusqlite::params![
                            updated_session.id,
                            serde_json::to_string(&updated_session)?
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
                    tx.execute(
                        "INSERT INTO planning_generation_attempt(id,session_id,level,retry_key,outcome,proposal_id,data,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                        rusqlite::params![
                            report.attempt_id,
                            report.session_id,
                            level_str,
                            report.retry_key,
                            outcome_str,
                            proposal_id,
                            serde_json::to_string(&report)?,
                            report.created_at
                        ],
                    )?;
                    tx.commit()?;
                    Ok(report)
                })())
            })
            .await?
    }
}
