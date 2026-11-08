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
    let captured_base_state = apply::base_state(&tx, &s.scope, &input)?;
    let p = PlanningProposal {
        id: mint_id("planning_proposal"),
        payload_hash: hash(&input)?,
        artifact: input,
        status: PlanningProposalStatus::Staged,
        policy_snapshot: s.policy,
        base_state: captured_base_state,
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

fn output_refs(item: &PlanningOperationItem) -> Vec<(String, &'static str)> {
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
