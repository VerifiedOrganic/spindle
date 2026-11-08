use super::*;
use spindle_core::models::CreateProjectOutput;
use std::collections::BTreeMap;

pub(crate) fn resolve_operation(
    item: &PlanningOperationItem,
    scope: &Option<PlanningScope>,
    refs: &BTreeMap<String, String>,
) -> Result<(String, Value)> {
    let operation = serde_json::to_value(&item.operation)?;
    let name = operation["operation"]
        .as_str()
        .context("planning operation name")?
        .to_string();
    let mut input = operation["input"].clone();
    fn resolve(v: &mut Value, refs: &BTreeMap<String, String>) -> Result<()> {
        match v {
            Value::Object(m) => {
                for (key, v) in m {
                    if matches!(key.as_str(), "emotional_state" | "base_emotions") {
                        continue;
                    }
                    if key.ends_with("_id") || key.ends_with("_ids") || key == "controlling_faction"
                    {
                        fn one(v: &mut Value, refs: &BTreeMap<String, String>) -> Result<()> {
                            if v.is_null() {
                                return Ok(());
                            }
                            if let Value::Array(a) = v {
                                for v in a {
                                    one(v, refs)?;
                                }
                                return Ok(());
                            }
                            let r: PlanningRef = serde_json::from_value(v.clone())?;
                            *v = Value::String(match r {
                                PlanningRef::ExistingRecordId(id) => id,
                                PlanningRef::ProposalRef(key) => refs
                                    .get(&key)
                                    .with_context(|| {
                                        format!("unresolved proposal reference {key}")
                                    })?
                                    .clone(),
                            });
                            Ok(())
                        }
                        one(v, refs)?;
                    } else {
                        resolve(v, refs)?;
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    resolve(v, refs)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    resolve(&mut input, refs)?;
    if !matches!(
        item.operation,
        PlanningOperation::CreateProject(_)
            | PlanningOperation::CreateRelationship(_)
            | PlanningOperation::UpdateReaderContract(_)
    ) {
        input
            .as_object_mut()
            .context("planning input object")?
            .insert(
                "project_id".into(),
                Value::String(
                    scope
                        .as_ref()
                        .context("operation requires accepted project contract")?
                        .project_id
                        .clone(),
                ),
            );
    }
    Ok((name, input))
}

fn rows(conn: &rusqlite::Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<Value> {
    use rusqlite::types::ValueRef;
    let mut stmt = conn.prepare(sql)?;
    let names = stmt
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let values = stmt
        .query_map(params, |r| {
            let mut obj = serde_json::Map::new();
            for (index, name) in names.iter().enumerate() {
                let value = match r.get_ref(index)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => Value::from(n),
                    ValueRef::Real(n) => Value::from(n),
                    ValueRef::Text(v) => Value::String(String::from_utf8_lossy(v).into_owned()),
                    ValueRef::Blob(_) => {
                        return Err(rusqlite::Error::InvalidColumnType(
                            index,
                            name.clone(),
                            rusqlite::types::Type::Blob,
                        ));
                    }
                };
                obj.insert(name.clone(), value);
            }
            Ok(Value::Object(obj))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Value::Array(values))
}

pub(super) fn operation_state(
    conn: &rusqlite::Connection,
    scope: &Option<PlanningScope>,
    item: &PlanningOperationItem,
    index: usize,
    refs: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Value>> {
    let mut state = BTreeMap::new();
    fn collect(
        v: &Value,
        ids: &mut std::collections::BTreeSet<String>,
        refs: &BTreeMap<String, String>,
    ) {
        match v {
            Value::Object(m)
                if m.get("kind").and_then(Value::as_str) == Some("existing_record_id") =>
            {
                if let Some(id) = m.get("value").and_then(Value::as_str) {
                    ids.insert(id.to_string());
                }
            }
            Value::Object(m) if m.get("kind").and_then(Value::as_str) == Some("proposal_ref") => {
                if let Some(id) = m
                    .get("value")
                    .and_then(Value::as_str)
                    .and_then(|key| refs.get(key))
                {
                    ids.insert(id.clone());
                }
            }
            Value::Object(m) => {
                for (k, v) in m {
                    if !matches!(k.as_str(), "emotional_state" | "base_emotions") {
                        collect(v, ids, refs);
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    collect(v, ids, refs);
                }
            }
            _ => {}
        }
    }
    let mut ids = std::collections::BTreeSet::new();
    collect(&serde_json::to_value(item)?, &mut ids, refs);
    for id in ids {
        let table = id.split(':').next().unwrap_or("");
        anyhow::ensure!(
            [
                "character",
                "location",
                "world_rule",
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
        state.insert(
            format!("{index}:row:{id}"),
            rows(conn, &format!("SELECT * FROM {table} WHERE id=?1"), &[&id])?,
        );
    }
    let Some(scope) = scope else {
        return Ok(state);
    };
    state.insert(
        "context:reader_contract".into(),
        rows(
            conn,
            "SELECT reader_contract,genre,project_type FROM project WHERE id=?1",
            &[&scope.project_id],
        )?,
    );
    // Targets use stable per-operation keys, so progress on another chapter
    // does not invalidate this chapter's preflight snapshot.
    let input = match resolve_operation(item, &Some(scope.clone()), refs) {
        Ok((_, i)) => i,
        Err(_) => return Ok(state),
    };
    let project = &scope.project_id;
    let branch = &scope.branch_id;
    let n = |key: &str| input[key].as_i64().unwrap_or(0);
    let mut target = |name: &str, sql: &str, params: &[&dyn rusqlite::ToSql]| -> Result<()> {
        let value = if matches!(name, "chapter_plan" | "chapter_outline" | "book_outline") {
            visible_rows(conn, scope, sql, params)?
        } else {
            rows(conn, sql, params)?
        };
        state.insert(format!("{index}:target:{name}"), value);
        Ok(())
    };
    use PlanningOperation::*;
    match &item.operation {
        UpdateReaderContract(_) => {
            target("project", "SELECT * FROM project WHERE id=?1", &[project])?
        }
        CreateChapter(_) => {
            let book_id = input["book_id"].as_str().unwrap_or("");
            target(
                "chapter",
                "SELECT c.* FROM chapter c JOIN book b ON b.id=c.book_id WHERE c.project_id=?1 AND (b.id=?2 OR (?2='' AND b.book_number=?3)) AND c.chapter_number=?4",
                &[project, &book_id, &n("book_number"), &n("chapter_number")],
            )?;
        }
        CreateBook(_) => target(
            "book_allocation",
            "SELECT * FROM book WHERE project_id=?1 ORDER BY book_number",
            &[project],
        )?,
        CreatePacingConfig(_) => target(
            "pacing_config",
            "SELECT * FROM pacing_config WHERE project_id=?1 AND branch_id=?2",
            &[project, branch],
        )?,
        CreatePacingCurve(_) => target(
            "pacing_curve",
            "SELECT * FROM pacing_curve WHERE project_id=?1 AND branch_id=?2 AND book_number=?3",
            &[project, branch, &n("book_number")],
        )?,
        PlanChapter(_) => {
            target(
                "chapter_plan",
                "SELECT * FROM chapter_plan WHERE project_id=?1 AND branch_id=?2 AND book_number=?3 AND chapter_number=?4",
                &[project, branch, &n("book_number"), &n("chapter_number")],
            )?;
            target(
                "chapter_outline",
                "SELECT o.* FROM chapter_outline o JOIN chapter c ON c.id=o.chapter_id WHERE c.project_id=?1 AND o.branch_id=?2 AND c.book_number=?3 AND c.chapter_number=?4",
                &[project, branch, &n("book_number"), &n("chapter_number")],
            )?;
        }
        SetBookOutline(_) => {
            let book_id = input["book_id"].as_str().unwrap_or("");
            target(
                "book_outline",
                "SELECT o.* FROM book_outline o JOIN book b ON b.id=o.book_id WHERE b.project_id=?1 AND o.branch_id=?2 AND (b.id=?3 OR (?3='' AND b.book_number=?4))",
                &[project, branch, &book_id, &n("book_number")],
            )?;
        }
        SetChapterOutline(_) => {
            let id = input["entity_id"].as_str().unwrap_or("");
            target(
                "chapter_outline",
                "SELECT o.* FROM chapter_outline o JOIN chapter c ON c.id=o.chapter_id WHERE c.project_id=?1 AND o.branch_id=?2 AND (c.id=?3 OR (?3='' AND c.book_number=?4 AND c.chapter_number=?5))",
                &[
                    project,
                    branch,
                    &id,
                    &n("book_number"),
                    &n("chapter_number"),
                ],
            )?;
        }
        SetArcPacingConstraints(_) => {
            let arc = input["character_arc_id"].as_str().context("arc id")?;
            target(
                "pacing_tracker",
                "SELECT * FROM pacing_tracker WHERE project_id=?1 AND branch_id=?2 AND character_arc_id=?3",
                &[project, branch, &arc],
            )?;
        }
        CreateRelationship(_) => {
            let a = input["character_a_id"].as_str().context("character a")?;
            let b = input["character_b_id"].as_str().context("character b")?;
            target(
                "relationship",
                "SELECT * FROM relates_to WHERE branch_id=?1 AND ((in_id=?2 AND out_id=?3) OR (in_id=?3 AND out_id=?2))",
                &[branch, &a, &b],
            )?;
        }
        _ => {}
    }
    Ok(state)
}
pub(super) fn base_state(
    conn: &rusqlite::Connection,
    scope: &Option<PlanningScope>,
    input: &StagePlanningInput,
) -> Result<BTreeMap<String, String>> {
    let mut state = BTreeMap::new();
    let refs = refs_in_conn(conn, &input.session_id)?;
    for (index, item) in input.operations.iter().enumerate() {
        for (key, value) in operation_state(conn, scope, item, index, &refs)? {
            state.insert(key, hash(&value)?);
        }
    }
    Ok(state)
}

impl Repository {
    pub(crate) async fn planning_operation_state(
        &self,
        scope: Option<PlanningScope>,
        item: PlanningOperationItem,
        index: usize,
        refs: BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, Value>> {
        self.pool()
            .read(move |conn| Ok(operation_state(conn, &scope, &item, index, &refs)))
            .await?
    }
    pub(crate) async fn planning_receipts(
        &self,
        session_id: &str,
    ) -> Result<Vec<PlanningOperationReceipt>> {
        let id = session_id.to_owned();
        self.pool().read(move |conn| {
            let mut stmt=conn.prepare("SELECT r.proposal_id,r.operation_index,r.input_hash,r.returned_ids,r.prior_state,r.outcome,r.error,r.recorded_at,r.actor,r.result,r.decision_note FROM planning_operation_receipt r JOIN planning_proposal p ON p.id=r.proposal_id WHERE p.session_id=?1 ORDER BY r.recorded_at,r.proposal_id,r.operation_index")?;
            stmt.query_map([id],read_receipt)?.collect::<rusqlite::Result<Vec<_>>>()
        }).await
    }
    pub(crate) async fn store_planning_receipt(
        &self,
        receipt: PlanningOperationReceipt,
        root: Option<CreateProjectOutput>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.pool().in_application_transaction(),
            "planning receipts require the application transaction boundary"
        );
        self.pool().write(move |conn|Ok((||->Result<()> {
            let committed:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM planning_operation_receipt WHERE proposal_id=?1 AND operation_index=?2 AND outcome='applied')",rusqlite::params![receipt.proposal_id,receipt.operation_index as i64],|r|r.get(0))?;
            if committed {
                if receipt.outcome==PlanningOperationOutcome::Failed {return Ok(());}
                anyhow::bail!("planning operation already committed");
            }
            if let Some(root)=root {
                let proposal: String=conn.query_row("SELECT session_id FROM planning_proposal WHERE id=?1",[&receipt.proposal_id],|r|r.get(0))?;
                let mut s=session(conn,&proposal)?;
                anyhow::ensure!(s.scope.is_none(),"planning root already attached");
                s.scope=Some(PlanningScope{project_id:root.project_id.clone(),branch_id:root.branch_id.clone()});
                conn.execute("UPDATE planning_session SET project_id=?2,branch_id=?3,data=?4 WHERE id=?1",rusqlite::params![s.id,root.project_id,root.branch_id,serde_json::to_string(&s)?])?;
            }
            let outcome=if receipt.outcome==PlanningOperationOutcome::Applied {"applied"}else{"failed"};
            conn.execute("INSERT INTO planning_operation_receipt(proposal_id,operation_index,input_hash,returned_ids,prior_state,outcome,error,recorded_at,actor,result,decision_note) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(proposal_id,operation_index) DO UPDATE SET input_hash=excluded.input_hash,returned_ids=excluded.returned_ids,prior_state=excluded.prior_state,outcome=excluded.outcome,error=excluded.error,recorded_at=excluded.recorded_at,actor=excluded.actor,result=excluded.result,decision_note=excluded.decision_note",rusqlite::params![receipt.proposal_id,receipt.operation_index as i64,receipt.input_hash,serde_json::to_string(&receipt.returned_ids)?,serde_json::to_string(&receipt.prior_state)?,outcome,receipt.error,receipt.recorded_at,receipt.actor,serde_json::to_string(&receipt.result)?,receipt.decision_note])?;
            if receipt.outcome==PlanningOperationOutcome::Applied {backfill_children(conn,&receipt)?;}
            Ok(())
        })())).await??;
        Ok(())
    }
    pub(crate) async fn finish_planning_proposal(
        &self,
        id: String,
        actor: String,
        note: String,
    ) -> Result<PlanningProposal> {
        self.pool().write(move |conn|Ok((||->Result<_> {
            let data:String=conn.query_row("SELECT data FROM planning_proposal WHERE id=?1",[&id],|r|r.get(0))?;
            let mut p:PlanningProposal=serde_json::from_str(&data)?;
            if p.status==PlanningProposalStatus::Applied {return Ok(p);}
            let done:i64=conn.query_row("SELECT COUNT(*) FROM planning_operation_receipt WHERE proposal_id=?1 AND outcome='applied'",[&id],|r|r.get(0))?;
            anyhow::ensure!(done==p.artifact.operations.len() as i64,"planning proposal has incomplete operations");
            p.status=PlanningProposalStatus::Applied;p.decided_by=Some(actor);p.decision_note=Some(note);p.decided_at=Some(timestamp_to_micros(chrono::Utc::now()));save(conn,&p)?;Ok(p)
        })())).await?
    }
    pub(crate) async fn store_planning_attempt(
        &self,
        session_id: String,
        actor: String,
        report: PlanningApplyReport,
    ) -> Result<()> {
        self.pool().write(move |conn| {
            conn.execute("INSERT INTO planning_apply_attempt(id,session_id,actor,data,created_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET data=excluded.data",rusqlite::params![report.attempt_id,session_id,actor,serde_json::to_string(&report).map_err(|e|rusqlite::Error::ToSqlConversionFailure(e.into()))?,timestamp_to_micros(chrono::Utc::now())])?;Ok(())
        }).await
    }
    pub(crate) async fn queue_planning_embedding(
        &self,
        project: &str,
        branch: &str,
        id: &str,
        doc: &SearchDocument,
    ) -> Result<()> {
        let (project, branch, id, doc) = (
            project.to_owned(),
            branch.to_owned(),
            id.to_owned(),
            doc.clone(),
        );
        self.pool().write(move |conn| {conn.execute("INSERT INTO planning_embedding_refresh(project_id,branch_id,entity_id,entity_table,title,excerpt,content) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(project_id,branch_id,entity_id) DO UPDATE SET entity_table=excluded.entity_table,title=excluded.title,excerpt=excluded.excerpt,content=excluded.content",rusqlite::params![project,branch,id,doc.entity_table,doc.title,doc.excerpt,doc.content])?;Ok(())}).await
    }
    pub(crate) async fn flush_planning_embeddings(&self, project: &str) -> Result<usize> {
        let project = project.to_owned();
        let query_project = project.clone();
        let docs=self.pool().read(move |conn| {let mut stmt=conn.prepare("SELECT branch_id,entity_id,entity_table,title,excerpt,content FROM planning_embedding_refresh WHERE project_id=?1")?;stmt.query_map([query_project],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,SearchDocument{entity_table:r.get(2)?,title:r.get(3)?,excerpt:r.get(4)?,content:r.get(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()}).await?;
        for (branch, id, doc) in docs {
            if self
                .refresh_search_embedding_for_entity(&project, &branch, &id, &doc)
                .await
                .is_ok()
            {
                let project = project.clone();
                self.pool().write(move |conn| {conn.execute("DELETE FROM planning_embedding_refresh WHERE project_id=?1 AND branch_id=?2 AND entity_id=?3 AND content=?4 AND title=?5 AND excerpt=?6",rusqlite::params![project,branch,id,doc.content,doc.title,doc.excerpt])?;Ok(())}).await?;
            }
        }
        self.pool()
            .read(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM planning_embedding_refresh WHERE project_id=?1",
                    [project],
                    |r| r.get::<_, i64>(0),
                )
            })
            .await
            .map(|n| n as usize)
    }
}

fn refs_in_conn(conn: &rusqlite::Connection, id: &str) -> Result<BTreeMap<String, String>> {
    let mut stmt=conn.prepare("SELECT r.returned_ids FROM planning_operation_receipt r JOIN planning_proposal p ON p.id=r.proposal_id WHERE p.session_id=?1 AND r.outcome='applied' ORDER BY r.recorded_at,r.proposal_id,r.operation_index")?;
    let values = stmt
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut refs = BTreeMap::new();
    for json in values {
        refs.extend(serde_json::from_str::<BTreeMap<String, String>>(&json)?);
    }
    Ok(refs)
}
fn backfill_children(
    conn: &rusqlite::Connection,
    receipt: &PlanningOperationReceipt,
) -> Result<()> {
    let session_id: String = conn.query_row(
        "SELECT session_id FROM planning_proposal WHERE id=?1",
        [&receipt.proposal_id],
        |r| r.get(0),
    )?;
    let s = session(conn, &session_id)?;
    let all = proposals(conn, &session_id)?;
    let source_hash = all
        .iter()
        .find(|p| p.id == receipt.proposal_id)
        .context("missing receipt proposal")?
        .payload_hash
        .clone();
    let before_books = receipt
        .prior_state
        .get(format!(
            "{}:target:book_allocation",
            receipt.operation_index
        ))
        .map(hash)
        .transpose()?;
    let allocated_book = matches!(receipt.result, Some(PlanningAppliedOutput::CreateBook(_)));
    let changed_contract = matches!(
        receipt.result,
        Some(PlanningAppliedOutput::UpdateReaderContract(_))
    );
    let before_contract = receipt
        .prior_state
        .get("context:reader_contract")
        .map(hash)
        .transpose()?;
    for mut p in all {
        if p.status != PlanningProposalStatus::Staged {
            continue;
        }
        let owns_allocation = p.id == receipt.proposal_id
            || p.artifact.parents.get(&receipt.proposal_id) == Some(&source_hash);
        for (key, value) in base_state(conn, &s.scope, &p.artifact)? {
            let later = p.id != receipt.proposal_id
                || key
                    .split(':')
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                    .is_some_and(|i| i > receipt.operation_index);
            let refresh_contract = changed_contract
                && p.id != receipt.proposal_id
                && owns_allocation
                && key == "context:reader_contract"
                && before_contract
                    .as_ref()
                    .is_some_and(|before| p.base_state.get(&key) == Some(before));
            let advance_book_allocation = allocated_book
                && owns_allocation
                && later
                && key.ends_with(":target:book_allocation")
                && before_books
                    .as_ref()
                    .is_some_and(|before| p.base_state.get(&key) == Some(before));
            if refresh_contract || advance_book_allocation {
                p.base_state.insert(key, value);
            } else {
                p.base_state.entry(key).or_insert(value);
            }
        }
        save(conn, &p)?;
    }
    Ok(())
}

impl Repository {
    pub(crate) async fn planning_scope_guard(
        &self,
        scope: Option<PlanningScope>,
        input: Value,
        level: PlanningLevel,
        freezes_project: bool,
    ) -> Result<()> {
        let Some(scope) = scope else {
            return Ok(());
        };
        self.pool()
            .read(move |conn| Ok(scope_guard(conn, scope, input, level, freezes_project)))
            .await?
    }
    pub(crate) async fn latest_planning_attempt(
        &self,
        id: &str,
    ) -> Result<Option<PlanningApplyReport>> {
        let id = id.to_owned();
        self.pool().read(move |conn|Ok((||->Result<_> {
   let data:Option<String>=conn.query_row("SELECT data FROM planning_apply_attempt WHERE session_id=?1 ORDER BY created_at DESC,id DESC LIMIT 1",[id],|r|r.get(0)).optional_inner()?;
   data.map(|s|serde_json::from_str(&s).map_err(anyhow::Error::from)).transpose()
  })())).await?
    }
    pub(crate) async fn planning_embedding_count(
        &self,
        scope: Option<PlanningScope>,
    ) -> Result<usize> {
        let Some(scope) = scope else {
            return Ok(0);
        };
        self.pool()
            .read(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM planning_embedding_refresh WHERE project_id=?1",
                    [scope.project_id],
                    |r| r.get::<_, i64>(0),
                )
            })
            .await
            .map(|n| n as usize)
    }
}

fn read_receipt(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanningOperationReceipt> {
    fn parse<T: serde::de::DeserializeOwned>(json: &str, index: usize) -> rusqlite::Result<T> {
        serde_json::from_str(json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                error.into(),
            )
        })
    }
    let outcome: String = row.get(5)?;
    let prior: Option<String> = row.get(4)?;
    Ok(PlanningOperationReceipt {
        proposal_id: row.get(0)?,
        operation_index: row.get::<_, i64>(1)? as usize,
        input_hash: row.get(2)?,
        returned_ids: parse(&row.get::<_, String>(3)?, 3)?,
        prior_state: prior
            .map(|s| parse(&s, 4))
            .transpose()?
            .unwrap_or(Value::Null),
        outcome: if outcome == "applied" {
            PlanningOperationOutcome::Applied
        } else {
            PlanningOperationOutcome::Failed
        },
        error: row.get(6)?,
        recorded_at: row.get(7)?,
        actor: row.get(8)?,
        result: parse(&row.get::<_, String>(9)?, 9)?,
        decision_note: row.get(10)?,
    })
}

fn scope_guard(
    conn: &rusqlite::Connection,
    scope: PlanningScope,
    input: Value,
    level: PlanningLevel,
    freezes_project: bool,
) -> Result<()> {
    check_scope(conn, &Some(scope.clone()))?;
    let active: String = conn.query_row(
        "SELECT active_branch_id FROM project WHERE id=?1",
        [&scope.project_id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(
        active == scope.branch_id,
        "planning branch is no longer active; select the session branch explicitly"
    );
    let active_run:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM authoring_run WHERE project_id=?1 AND active_branch_id=?2 AND status NOT IN ('completed','cancelled','abandoned'))",rusqlite::params![scope.project_id,scope.branch_id],|r|r.get(0))?;
    let frozen_run = if freezes_project {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM authoring_run WHERE project_id=?1 AND status NOT IN ('completed','cancelled','abandoned'))",[&scope.project_id],|r|r.get::<_,bool>(0))?
    } else {
        active_run
    };
    anyhow::ensure!(
        !frozen_run || matches!(level, PlanningLevel::Chapter | PlanningLevel::Scene),
        "initial planning roots are frozen by an authoring run"
    );
    let project = &scope.project_id;
    let branch = &scope.branch_id;
    fn validate_ids(
        conn: &rusqlite::Connection,
        v: &Value,
        project: &str,
        branch: &str,
    ) -> Result<()> {
        match v {
            Value::Object(m) => {
                for (key, v) in m {
                    if matches!(
                        key.as_str(),
                        "project_id" | "emotional_state" | "base_emotions"
                    ) {
                        continue;
                    }
                    if key.ends_with("_id") || key.ends_with("_ids") || key == "controlling_faction"
                    {
                        let ids: Vec<&str> = if let Some(a) = v.as_array() {
                            a.iter().filter_map(Value::as_str).collect()
                        } else {
                            v.as_str().into_iter().collect()
                        };
                        for id in ids {
                            let table = id.split(':').next().unwrap_or("");
                            anyhow::ensure!(
                                [
                                    "character",
                                    "location",
                                    "theme",
                                    "conflict",
                                    "plot_line",
                                    "character_arc",
                                    "book",
                                    "chapter",
                                    "faction"
                                ]
                                .contains(&table),
                                "unsupported planning reference type"
                            );
                            let sql = if matches!(table, "book" | "chapter") {
                                format!(
                                    "SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?1 AND project_id=?2 AND ?3 IS NOT NULL)"
                                )
                            } else {
                                format!(
                                    "WITH RECURSIVE lineage(id,parent_branch_id) AS (SELECT id,parent_branch_id FROM bible_branch WHERE id=?3 AND project_id=?2 UNION SELECT b.id,b.parent_branch_id FROM bible_branch b JOIN lineage l ON b.id=l.parent_branch_id WHERE b.project_id=?2) SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?1 AND project_id=?2 AND branch_id IN (SELECT id FROM lineage))"
                                )
                            };
                            let visible: bool = conn.query_row(
                                &sql,
                                rusqlite::params![id, project, branch],
                                |r| r.get(0),
                            )?;
                            anyhow::ensure!(
                                visible,
                                "planning reference no longer visible on selected branch"
                            );
                        }
                    } else {
                        validate_ids(conn, v, project, branch)?;
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    validate_ids(conn, v, project, branch)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    validate_ids(conn, &input, project, branch)?;
    let has_locator = input["book_id"].is_string() || input["entity_id"].is_string();
    if let Some(number) = input["book_number"].as_i64() {
        anyhow::ensure!(
            number > 0 || (number == 0 && has_locator),
            "planning book_number must be positive"
        );
    }
    if let Some(number) = input["chapter_number"].as_i64() {
        anyhow::ensure!(
            number > 0 || (number == 0 && input["entity_id"].is_string()),
            "planning chapter_number must be positive"
        );
    }
    if let Some(id) = input["book_id"].as_str() {
        let actual: i64 = conn.query_row(
            "SELECT book_number FROM book WHERE id=?1 AND project_id=?2",
            rusqlite::params![id, project],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            actual > 0
                && input["book_number"]
                    .as_i64()
                    .filter(|n| *n > 0)
                    .is_none_or(|n| n == actual),
            "book_id and book_number disagree"
        );
    }
    if matches!(level, PlanningLevel::Chapter | PlanningLevel::Scene) {
        let (book, chapter) = if let Some(id) = input["entity_id"].as_str() {
            anyhow::ensure!(
                input["entity_type"]
                    .as_str()
                    .is_none_or(|kind| kind == "chapter"),
                "chapter outline must target a chapter"
            );
            let actual = conn.query_row(
                "SELECT book_number,chapter_number FROM chapter WHERE id=?1 AND project_id=?2",
                rusqlite::params![id, project],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )?;
            anyhow::ensure!(
                input["book_number"]
                    .as_i64()
                    .filter(|n| *n > 0)
                    .is_none_or(|n| n == actual.0)
                    && input["chapter_number"]
                        .as_i64()
                        .filter(|n| *n > 0)
                        .is_none_or(|n| n == actual.1),
                "chapter id and placement disagree"
            );
            actual
        } else {
            let book = if let Some(id) = input["book_id"].as_str() {
                let actual: i64 = conn.query_row(
                    "SELECT book_number FROM book WHERE id=?1 AND project_id=?2",
                    rusqlite::params![id, project],
                    |r| r.get(0),
                )?;
                anyhow::ensure!(
                    input["book_number"].as_i64().is_none_or(|n| n == actual),
                    "book_id and book_number disagree"
                );
                actual
            } else {
                input["book_number"]
                    .as_i64()
                    .context("planning requires explicit book selection")?
            };
            (
                book,
                input["chapter_number"]
                    .as_i64()
                    .context("planning requires explicit positive chapter_number")?,
            )
        };
        let accepted_book: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM book WHERE project_id=?1 AND book_number=?2)",
            rusqlite::params![project, book],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            accepted_book,
            "book must be accepted before chapter planning"
        );
        anyhow::ensure!(
            book > 0 && chapter > 0,
            "planning book/chapter numbers must be positive"
        );
        let drafted:bool=conn.query_row("WITH RECURSIVE lineage(id,parent_branch_id) AS (SELECT id,parent_branch_id FROM bible_branch WHERE id=?1 AND project_id=?2 UNION SELECT b.id,b.parent_branch_id FROM bible_branch b JOIN lineage l ON b.id=l.parent_branch_id WHERE b.project_id=?2) SELECT EXISTS(SELECT 1 FROM scene WHERE project_id=?2 AND branch_id IN (SELECT id FROM lineage) AND book_number=?3 AND chapter_number=?4)",rusqlite::params![branch,project,book,chapter],|r|r.get(0))?;
        anyhow::ensure!(
            !drafted,
            "planning cannot overwrite a chapter with persisted prose"
        );
        let mut planned = false;
        for visible_branch in resolve_branch_lineage_in_conn(conn, Some(project), branch)? {
            planned=conn.query_row("SELECT EXISTS(SELECT 1 FROM chapter_plan WHERE project_id=?1 AND branch_id=?2 AND book_number=?3 AND chapter_number=?4)",rusqlite::params![project,visible_branch,book,chapter],|r|r.get(0))?;
            if planned {
                break;
            }
        }
        let cached_plan:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM authoring_run r JOIN authoring_run_chapter c ON c.authoring_run_id=r.id WHERE r.project_id=?1 AND r.active_branch_id=?2 AND r.book_number=?3 AND c.chapter_number=?4 AND c.planned<>0 AND r.status NOT IN ('completed','cancelled','abandoned'))",rusqlite::params![project,branch,book,chapter],|r|r.get(0))?;
        planned |= cached_plan;
        anyhow::ensure!(
            !active_run || !planned,
            "an authoring run already has this chapter planned; use human-ratified plan amendments"
        );
    }
    Ok(())
}

fn visible_rows(
    conn: &rusqlite::Connection,
    scope: &PlanningScope,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Value> {
    for branch in resolve_branch_lineage_in_conn(conn, Some(&scope.project_id), &scope.branch_id)? {
        let mut args = params.to_vec();
        args[1] = &branch;
        let value = rows(conn, sql, &args)?;
        if !value.as_array().is_some_and(Vec::is_empty) {
            return Ok(value);
        }
    }
    Ok(Value::Array(Vec::new()))
}
