use super::*;
use spindle_core::models::ReaderContract;
async fn fresh_service_local() -> (tempfile::TempDir, SqliteSpindleService) {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = crate::sqlite::SqlitePool::open(&tmp.path().join("planning.db"))
        .await
        .unwrap();
    let repo = Repository::with_model_router(
        pool,
        tmp.path().join("data"),
        crate::ai::ModelRouter::local_only(),
    );
    (tmp, SqliteSpindleService::new(repo))
}
use std::collections::BTreeMap;

fn artifact(s: &PlanningSession, key: &str) -> StagePlanningInput {
    StagePlanningInput {
        session_id: s.id.clone(),
        scope: s.scope.clone(),
        intent_hash: s.intent_hash.clone(),
        schema_version: 1,
        level: PlanningLevel::Contract,
        parents: BTreeMap::new(),
        operations: vec![PlanningOperationItem {
            output_ref: key.into(),
            operation: PlanningOperation::CreateProject(PlanningCreateProjectInput {
                name: "Harbor".into(),
                project_type: "novel".into(),
                genre: "mystery".into(),
                reader_contract: PlanningReaderContract {
                    promise: "Find the heir".into(),
                    style_notes: vec![],
                    boundaries: vec![],
                },
            }),
        }],
        rationale: "Review the promise before creating the project".into(),
    }
}
fn decision(s: &PlanningSession, p: &PlanningProposal) -> DecidePlanningInput {
    DecidePlanningInput {
        session_id: s.id.clone(),
        scope: s.scope.clone(),
        expected_hashes: BTreeMap::from([(p.id.clone(), p.payload_hash.clone())]),
        decision: PlanningDecision::Reject,
        actor: "author".into(),
        note: "Author decision for this proposal".into(),
    }
}
#[tokio::test]
async fn stage_reopen_decide_never_writes_bible() {
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "A missing heir returns".into(),
            scope: None,
        })
        .await
        .unwrap();
    assert!(s.policy.gates.is_empty());
    let p = svc
        .stage_planning(artifact(&s, "project:root"))
        .await
        .unwrap();
    assert_eq!(p.status, PlanningProposalStatus::Staged);
    assert!(svc.list_projects().await.unwrap().projects.is_empty());
    let reopened = SqliteSpindleService::new(svc.repository.clone());
    assert_eq!(
        reopened
            .get_planning_session(&s.id)
            .await
            .unwrap()
            .intent_hash,
        s.intent_hash
    );
    assert_eq!(
        reopened.list_planning_proposals(&s.id, None).await.unwrap()[0].payload_hash,
        p.payload_hash
    );
    let result = reopened.decide_planning(decision(&s, &p)).await.unwrap();
    assert_eq!(result[0].status, PlanningProposalStatus::Rejected);
    assert_eq!(result[0].decided_by.as_deref(), Some("author"));
    assert!(result[0].decided_at.is_some());
    assert!(reopened.decide_planning(decision(&s, &p)).await.is_err());
    assert!(svc.list_projects().await.unwrap().projects.is_empty());
    let count: i64 = svc
        .repository
        .pool()
        .read(|c| {
            c.query_row("SELECT COUNT(*) FROM planning_operation_receipt", [], |r| {
                r.get(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(count, 0);
}
#[tokio::test]
async fn batch_preflight_and_human_authority_guards() {
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "A harbor mystery".into(),
            scope: None,
        })
        .await
        .unwrap();
    let p = svc
        .stage_planning(artifact(&s, "project:root"))
        .await
        .unwrap();
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    d.actor = "policy:detail:1".into();
    assert!(
        svc.decide_planning(d)
            .await
            .unwrap_err()
            .to_string()
            .contains("human")
    );
    let mut d = decision(&s, &p);
    d.expected_hashes
        .insert("planning_proposal:missing".into(), "bad".into());
    assert!(svc.decide_planning(d).await.is_err());
    let mut d = decision(&s, &p);
    d.expected_hashes.insert(p.id.clone(), "stale".into());
    assert!(svc.decide_planning(d).await.is_err());
    let mut d = decision(&s, &p);
    d.actor = "policy:detail:1".into();
    assert!(svc.decide_planning(d).await.is_err());
    assert_eq!(
        svc.list_planning_proposals(&s.id, None).await.unwrap()[0].status,
        PlanningProposalStatus::Staged
    );
}
#[tokio::test]
async fn dependency_hashes_refs_and_descendant_invalidation() {
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "An heir".into(),
            scope: None,
        })
        .await
        .unwrap();
    let root = svc
        .stage_planning(artifact(&s, "project:root"))
        .await
        .unwrap();
    let mut child = artifact(&s, "book:next");
    child.level = PlanningLevel::Book;
    child
        .parents
        .insert(root.id.clone(), root.payload_hash.clone());
    child.operations[0].operation = PlanningOperation::CreateBook(PlanningCreateBookInput {
        title: Some("Book".into()),
    });
    let mut bad = child.clone();
    bad.parents.insert(root.id.clone(), "stale".into());
    assert!(svc.stage_planning(bad).await.is_err());
    let child = svc.stage_planning(child).await.unwrap();
    let mut grand = artifact(&s, "chapter:next");
    grand.level = PlanningLevel::Chapter;
    grand
        .parents
        .insert(child.id.clone(), child.payload_hash.clone());
    grand.operations[0].operation = PlanningOperation::CreateChapter(PlanningCreateChapterInput {
        book_number: None,
        book_id: Some(PlanningRef::ProposalRef("book:next".into())),
        chapter_number: Some(2),
        title: None,
    });
    let mut bad = grand.clone();
    bad.parents.clear();
    assert!(svc.stage_planning(bad).await.is_err());
    let grand = svc.stage_planning(grand).await.unwrap();
    let changed = svc.decide_planning(decision(&s, &root)).await.unwrap();
    assert_eq!(changed.len(), 3);
    for p in changed {
        assert_eq!(
            p.status,
            if p.id == root.id {
                PlanningProposalStatus::Rejected
            } else {
                PlanningProposalStatus::Superseded
            }
        );
    }
    assert!(svc.stage_planning(grand.artifact).await.is_err());
}
#[tokio::test]
async fn intent_version_duplicate_and_scope_guards() {
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Harbor".into(),
            scope: None,
        })
        .await
        .unwrap();
    let mut a = artifact(&s, "root");
    a.schema_version = 2;
    assert!(svc.stage_planning(a).await.is_err());
    let mut a = artifact(&s, "root");
    a.intent_hash = "stale".into();
    assert!(svc.stage_planning(a).await.is_err());
    let mut a = artifact(&s, "root");
    a.operations.push(a.operations[0].clone());
    assert!(svc.stage_planning(a).await.is_err());
    let p = svc.stage_planning(artifact(&s, "root")).await.unwrap();
    assert!(svc.stage_planning(p.artifact).await.is_err());
    assert!(
        svc.list_planning_proposals(
            &s.id,
            Some(PlanningScope {
                project_id: "project:wrong".into(),
                branch_id: "bible_branch:wrong".into()
            })
        )
        .await
        .is_err()
    );
    assert!(
        svc.begin_planning(BeginPlanningInput {
            premise: "existing".into(),
            scope: Some(PlanningScope {
                project_id: "project:missing".into(),
                branch_id: "bible_branch:missing".into()
            })
        })
        .await
        .is_err()
    );
}
#[test]
fn operation_allowlist_and_references_are_closed() {
    assert!(
        serde_json::from_value::<PlanningOperation>(
            serde_json::json!({"operation":"update_entity","input":{}})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<PlanningRef>(serde_json::json!("character:invented")).is_err()
    );
    assert!(serde_json::from_value::<PlanningOperation>(serde_json::json!({"operation":"create_book","input":{"title":"Book","project_id":"project:invented"}})).is_err());
}

#[tokio::test]
async fn existing_project_refs_require_matching_project_branch_and_type() {
    let (_tmp, svc) = fresh_service_local().await;
    let make = |name: &str| CreateProjectInput {
        name: name.into(),
        project_type: "novel".into(),
        genre: "mystery".into(),
        reader_contract: ReaderContract {
            promise: "Discovery".into(),
            style_notes: vec![],
            boundaries: vec![],
        },
    };
    let project = svc.create_project(make("One")).await.unwrap();
    let other = svc.create_project(make("Two")).await.unwrap();
    assert!(
        svc.begin_planning(BeginPlanningInput {
            premise: "Bad scope".into(),
            scope: Some(PlanningScope {
                project_id: project.project_id.clone(),
                branch_id: other.branch_id.clone()
            })
        })
        .await
        .is_err()
    );
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Next chapter".into(),
            scope: Some(PlanningScope {
                project_id: project.project_id.clone(),
                branch_id: project.branch_id.clone(),
            }),
        })
        .await
        .unwrap();
    let mut a = artifact(&s, "chapter:next");
    a.level = PlanningLevel::Chapter;
    a.operations[0].operation = PlanningOperation::CreateChapter(PlanningCreateChapterInput {
        book_number: Some(1),
        book_id: Some(PlanningRef::ExistingRecordId(other.book_id)),
        chapter_number: Some(2),
        title: None,
    });
    assert!(svc.stage_planning(a.clone()).await.is_err());
    if let PlanningOperation::CreateChapter(ref mut c) = a.operations[0].operation {
        c.book_id = Some(PlanningRef::ExistingRecordId(project.book_id));
    }
    svc.stage_planning(a).await.unwrap();
    let char_input:CreateCharacterInput=serde_json::from_value(serde_json::json!({"project_id":project.project_id,"name":"Ada","summary":"Seeker","role":"lead","realm":null,"initial_state":null})).unwrap();
    let character = svc.create_character(char_input).await.unwrap();
    let mut rel = artifact(&s, "relationship:friends");
    rel.level = PlanningLevel::CastWorld;
    rel.operations[0].operation =
        PlanningOperation::CreateRelationship(PlanningCreateRelationshipInput {
            character_a_id: PlanningRef::ExistingRecordId(character.character_id.clone()),
            character_b_id: PlanningRef::ExistingRecordId(character.character_id),
            relationship_type: "friends".into(),
            initial_trust: 2,
            initial_tension: 1,
            dynamics: vec![],
        });
    svc.stage_planning(rel.clone()).await.unwrap();
    rel.operations[0].output_ref = "relationship:bad".into();
    if let PlanningOperation::CreateRelationship(ref mut r) = rel.operations[0].operation {
        r.character_b_id = PlanningRef::ExistingRecordId(project.chapter_id);
    }
    assert!(svc.stage_planning(rel).await.is_err());
}

#[tokio::test]
async fn planning_v0049_upgrades_v0048_without_changing_existing_project() {
    let tmp = tempfile::TempDir::new().unwrap();
    let _warm = crate::sqlite::SqlitePool::open(&tmp.path().join("warm.db"))
        .await
        .unwrap();
    let path = tmp.path().join("legacy.db");
    {
        let mut c = rusqlite::Connection::open(&path).unwrap();
        crate::sqlite::migrations::runner()
            .set_target(refinery::Target::Version(48))
            .run(&mut c)
            .unwrap();
        c.execute("INSERT INTO project(id,name,project_type,genre,reader_contract,created_at,updated_at) VALUES('project:legacy','Legacy','novel','mystery','{\"promise\":\"p\",\"style_notes\":[],\"boundaries\":[]}',1,1)",[]).unwrap();
    }
    let pool = crate::sqlite::SqlitePool::open(&path).await.unwrap();
    let name: String = pool
        .read(|c| {
            c.query_row(
                "SELECT name FROM project WHERE id='project:legacy'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(name, "Legacy");
    let tables:i64=pool.read(|c|c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('planning_session','planning_proposal','planning_operation_receipt')",[],|r|r.get(0))).await.unwrap();
    assert_eq!(tables, 3);
    assert_eq!(spindle_core::models::PLAN_AMENDMENT_CLASSES.len(), 8);
}

#[tokio::test]
async fn branch_refs_allow_ancestors_and_reject_descendant_only_records() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "Branch test".into(),
            project_type: "novel".into(),
            genre: "mystery".into(),
            reader_contract: ReaderContract {
                promise: "Clues".into(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .await
        .unwrap();
    let character = |name: &str| {
        serde_json::from_value::<CreateCharacterInput>(serde_json::json!({"project_id":project.project_id,"name":name,"summary":"Seeker","role":"lead","realm":null,"initial_state":null})).unwrap()
    };
    let ancestor = svc.create_character(character("Ancestor")).await.unwrap();
    let branch = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            name: "Child".into(),
            branch_type: "experiment".into(),
            description: None,
            parent_branch_id: Some(project.branch_id.clone()),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: branch.branch_id.clone(),
    })
    .await
    .unwrap();
    let child = svc.create_character(character("Child only")).await.unwrap();
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Inherited cast".into(),
            scope: Some(PlanningScope {
                project_id: project.project_id.clone(),
                branch_id: branch.branch_id,
            }),
        })
        .await
        .unwrap();
    let relation = |s: &PlanningSession, key: &str, a: &str, b: &str| {
        let mut p = artifact(s, key);
        p.level = PlanningLevel::CastWorld;
        p.operations[0].operation =
            PlanningOperation::CreateRelationship(PlanningCreateRelationshipInput {
                character_a_id: PlanningRef::ExistingRecordId(a.into()),
                character_b_id: PlanningRef::ExistingRecordId(b.into()),
                relationship_type: "rivals".into(),
                initial_trust: 0,
                initial_tension: 3,
                dynamics: vec![],
            });
        p
    };
    svc.stage_planning(relation(
        &s,
        "relationship:inherited",
        &ancestor.character_id,
        &child.character_id,
    ))
    .await
    .unwrap();
    let root = svc
        .begin_planning(BeginPlanningInput {
            premise: "Root cast".into(),
            scope: Some(PlanningScope {
                project_id: project.project_id,
                branch_id: project.branch_id,
            }),
        })
        .await
        .unwrap();
    assert!(
        svc.stage_planning(relation(
            &root,
            "relationship:invisible",
            &ancestor.character_id,
            &child.character_id
        ))
        .await
        .is_err()
    );
}

#[tokio::test]
async fn human_acceptance_attaches_root_and_retry_does_not_duplicate_project() {
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "A lost heir returns".into(),
            scope: None,
        })
        .await
        .unwrap();
    let p = svc
        .stage_planning(artifact(&s, "project:root"))
        .await
        .unwrap();
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    let result = svc.decide_planning(d.clone()).await.unwrap();
    assert_eq!(result[0].status, PlanningProposalStatus::Applied);
    let attached = svc.get_planning_session(&s.id).await.unwrap();
    assert!(attached.scope.is_some());
    d.scope = attached.scope;
    let retry = svc.decide_planning(d).await.unwrap();
    assert_eq!(retry[0].id, p.id);
    assert_eq!(svc.list_projects().await.unwrap().projects.len(), 1);
}

fn planned(key: &str, name: &str, input: serde_json::Value) -> PlanningOperationItem {
    serde_json::from_value(
        serde_json::json!({"output_ref":key,"operation":{"operation":name,"input":input}}),
    )
    .unwrap()
}
fn reference(key: &str) -> serde_json::Value {
    serde_json::json!({"kind":"proposal_ref","value":key})
}
async fn bundle(
    svc: &SqliteSpindleService,
    s: &PlanningSession,
    level: PlanningLevel,
    parents: &[&PlanningProposal],
    items: Vec<PlanningOperationItem>,
) -> PlanningProposal {
    let mut a = artifact(s, "unused");
    a.level = level;
    a.operations = items;
    a.parents = parents
        .iter()
        .map(|p| (p.id.clone(), p.payload_hash.clone()))
        .collect();
    svc.stage_planning(a).await.unwrap()
}
async fn accept(
    svc: &SqliteSpindleService,
    s: &PlanningSession,
    p: &PlanningProposal,
) -> PlanningApplyReport {
    let mut d = decision(s, p);
    d.decision = PlanningDecision::Accept;
    let r = svc.apply_planning(d).await.unwrap();
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(r.complete);
    r
}
async fn accepted_root(svc: &SqliteSpindleService) -> (PlanningSession, PlanningProposal) {
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "A lost heir returns to the harbor".into(),
            scope: None,
        })
        .await
        .unwrap();
    let p = svc
        .stage_planning(artifact(&s, "project:root"))
        .await
        .unwrap();
    accept(svc, &s, &p).await;
    (svc.get_planning_session(&s.id).await.unwrap(), p)
}

#[tokio::test]
async fn all_human_fixture_applies_allowlist_and_reads_six_complete_manifests() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    let cast=bundle(&svc,&s,PlanningLevel::CastWorld,&[&root],vec![
  planned("cast:ada","create_character",json!({"name":"Ada","summary":"Harbor archivist","role":"protagonist"})),
  planned("cast:ben","create_character",json!({"name":"Ben","summary":"Missing heir","role":"opposition"})),
  planned("place:harbor","create_location",json!({"name":"Harbor","kind":"city","summary":"A tidal port"})),
  planned("rule:tides","create_world_rule",json!({"rule_name":"Harbor tides","rule_type":"physics","description":"The tidal gate opens at dawn"})),
  planned("relationship:rivals","create_relationship",json!({"character_a_id":reference("cast:ada"),"character_b_id":reference("cast:ben"),"relationship_type":"rivals","initial_trust":0,"initial_tension":3})),
 ]).await;
    accept(&svc, &s, &cast).await;
    let arc=bundle(&svc,&s,PlanningLevel::SeriesArc,&[&cast],vec![
  planned("theme:trust","create_theme",json!({"theme_statement":"Trust requires risk","thesis_antithesis":"Safety versus connection"})),
  planned("conflict:heir","create_conflict",json!({"name":"The missing heir","conflict_type":"mystery","stakes":"Ada's home"})),
  planned("plot:heir","create_plot_line",json!({"name":"Find the heir","plot_type":"main","summary":"Trace a forged ledger","connected_theme_ids":[reference("theme:trust")],"connected_conflict_ids":[reference("conflict:heir")]})),
  planned("motif:seal","create_motif",json!({"name":"Broken seal","description":"An inherited signet","connected_theme_ids":[reference("theme:trust")]})),
  planned("promise:seal","create_narrative_promise",json!({"promise_type":"mystery","description":"Who forged the seal?","planted_at":{"book_number":1,"chapter_number":1},"planned_payoff":{"book_number":1,"chapter_number":3}})),
  planned("arc:ada","create_character_arc",json!({"character_id":reference("cast:ada"),"arc_type":"growth","starting_state":"Alone","ending_state":"Trusting","thematic_purpose":"Risk connection","connected_theme_ids":[reference("theme:trust")],"milestones":[{"label":"Ask for help","description":"Ada trusts Ben","placement":{"book_number":1,"chapter_number":2}}]})),
  planned("pacing:series","create_pacing_config",json!({"total_planned_books":2,"avg_chapters_per_book":3,"avg_scenes_per_chapter":2,"tension_model":"rising"})),
  planned("pacing:book","create_pacing_curve",json!({"book_number":1,"act_breakpoints":{"first":0.25,"second":0.75},"scene_type_density":{"action":0.5,"sequel":0.5},"intensity_points":[{"position":0.0,"intensity":0.2},{"position":1.0,"intensity":0.8}]})),
  planned("pacing:ada","set_arc_pacing_constraints",json!({"character_arc_id":reference("arc:ada"),"per_book_budget":{"1":0.7},"max_progress_per_chapter":0.3})),
 ]).await;
    accept(&svc, &s, &arc).await;
    let book=bundle(&svc,&s,PlanningLevel::Book,&[&root,&arc],vec![
  planned("outline:book","set_book_outline",json!({"book_id":reference("project:root:book"),"book_number":1,"content":"Ada finds the ledger, risks trust, and exposes the forgery."})),
  planned("book:future","create_book",json!({"title":"The open sea"})),
  planned("outline:future","set_book_outline",json!({"book_id":reference("book:future"),"book_number":2,"content":"Coarse future destination: take the ledger overseas."})),
 ]).await;
    accept(&svc, &s, &book).await;
    let mut chapter_items = Vec::new();
    for number in 1..=3 {
        chapter_items.push(planned(&format!("chapter:{number}"),"create_chapter",json!({"book_id":reference("project:root:book"),"book_number":1,"chapter_number":number,"title":format!("Ledger turn {number}")})));
        chapter_items.push(planned(&format!("outline:chapter:{number}"),"set_chapter_outline",json!({"entity_id":reference(&format!("chapter:{number}")),"book_number":1,"chapter_number":number,"content":"A clue forces a costly choice","beats":[{"order":1,"summary":"Find a clue","status":"planned"}]})));
    }
    let chapters = bundle(
        &svc,
        &s,
        PlanningLevel::Chapter,
        &[&root, &book],
        chapter_items,
    )
    .await;
    accept(&svc, &s, &chapters).await;
    let mut plans = Vec::new();
    for number in 1..=3 {
        plans.push(planned(&format!("plan:{number}"),"plan_chapter",json!({"book_number":1,"chapter_number":number,"pov_character_id":reference("cast:ada"),"synopsis":format!("Turn {number}: the forgery forces a costly choice"),"target_theme_ids":[reference("theme:trust")],"target_conflict_ids":[reference("conflict:heir")],"target_plot_line_ids":[reference("plot:heir")],"scenes":[
   {"scene_order":1,"summary":"Ada follows the ledger to the tidal gate","purpose":"Discover the forged seal","character_ids":[reference("cast:ada"),reference("cast:ben")],"location_id":reference("place:harbor"),"content_rating":"general","beat_structure":["clue","choice"],"research_required":true,"research_tags":["tidal gates"]},
   {"scene_order":2,"summary":"Ada wagers her signet on Ben's testimony","purpose":"Force a trust decision","character_ids":[reference("cast:ada"),reference("cast:ben")],"location_id":reference("place:harbor"),"content_rating":"general","beat_structure":["cost","change"]}
  ]})));
    }
    let manifests = bundle(
        &svc,
        &s,
        PlanningLevel::Scene,
        &[&chapters, &cast, &arc],
        plans,
    )
    .await;
    let report = accept(&svc, &s, &manifests).await;
    let scope = s.scope.clone().unwrap();
    let mut count = 0;
    for receipt in report.operations {
        let id = receipt.returned_ids.values().next().unwrap();
        let plan = svc.repository.get_chapter_plan(id).await.unwrap();
        assert_eq!(plan.project_id, scope.project_id);
        assert_eq!(plan.scenes.len(), 2);
        count += plan.scenes.len();
        for scene in &plan.scenes {
            assert_eq!(scene.character_ids.len(), 2);
            assert!(scene.location_id.is_some());
            assert!(scene.content_rating.is_some());
            assert!(!scene.purpose.is_empty());
        }
        assert_eq!(plan.scenes[0].research_required, Some(true));
        assert!(!receipt.prior_state.as_object().unwrap().is_empty());
        assert!(receipt.result.is_some());
    }
    assert_eq!(count, 6);
    let drafts: i64 = svc
        .repository
        .pool()
        .read(|c| c.query_row("SELECT COUNT(*) FROM scene", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(drafts, 0, "planning writes intent, never prose");
    let books = svc
        .repository
        .list_books_by_project(&scope.project_id)
        .await
        .unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(books.iter().filter(|b| b.book_number == 1).count(), 1);
    let update=bundle(&svc,&s,PlanningLevel::Contract,&[&root],vec![planned("contract:revised","update_reader_contract",json!({"promise":"A mystery about trust","style_notes":["Close third person"],"boundaries":[]}))]).await;
    let change = accept(&svc, &s, &update).await;
    assert!(
        change.operations[0]
            .prior_state
            .to_string()
            .contains("Find the heir")
    );
}

#[tokio::test]
async fn preflight_error_rolls_back_root_entities_attachments_and_receipts() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "No provisional project".into(),
            scope: None,
        })
        .await
        .unwrap();
    let root = svc
        .stage_planning(artifact(&s, "project:root"))
        .await
        .unwrap();
    let cast = bundle(
        &svc,
        &s,
        PlanningLevel::CastWorld,
        &[&root],
        vec![
            planned(
                "cast:a",
                "create_character",
                json!({"name":"Duplicate","summary":"One","role":"lead"}),
            ),
            planned(
                "cast:b",
                "create_character",
                json!({"name":"Duplicate","summary":"Two","role":"support"}),
            ),
        ],
    )
    .await;
    let mut d = decision(&s, &cast);
    d.decision = PlanningDecision::Accept;
    d.expected_hashes
        .insert(root.id.clone(), root.payload_hash.clone());
    assert!(svc.apply_planning(d).await.is_err());
    assert!(svc.list_projects().await.unwrap().projects.is_empty());
    assert!(
        svc.get_planning_session(&s.id)
            .await
            .unwrap()
            .scope
            .is_none()
    );
    assert!(
        svc.repository
            .planning_receipts(&s.id)
            .await
            .unwrap()
            .is_empty()
    );
    let queued: i64 = svc
        .repository
        .pool()
        .read(|c| {
            c.query_row("SELECT COUNT(*) FROM planning_embedding_refresh", [], |r| {
                r.get(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(queued, 0);
}

#[tokio::test]
async fn receipt_failure_keeps_committed_prefix_and_retries_without_duplicate_entities() {
    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    let p = bundle(
        &svc,
        &s,
        PlanningLevel::CastWorld,
        &[&root],
        vec![
            planned(
                "cast:a",
                "create_character",
                json!({"name":"Ada","summary":"Archivist","role":"lead"}),
            ),
            planned(
                "cast:b",
                "create_character",
                json!({"name":"Ben","summary":"Heir","role":"support"}),
            ),
            planned(
                "place:port",
                "create_location",
                json!({"name":"Port","kind":"city","summary":"Tidal harbor"}),
            ),
        ],
    )
    .await;
    let counter = Arc::new(AtomicUsize::new(0));
    svc.repository.pool().write(move |c| {
  c.create_scalar_function("planning_fail_receipt",0,rusqlite::functions::FunctionFlags::SQLITE_UTF8,move |_|Ok(counter.fetch_add(1,Ordering::SeqCst)+1==5))?;
  c.execute_batch("CREATE TEMP TRIGGER fail_planning_receipt BEFORE INSERT ON planning_operation_receipt WHEN planning_fail_receipt() BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END")?;Ok(())
 }).await.unwrap();
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    let failed = svc.apply_planning(d.clone()).await.unwrap();
    assert!(failed.error.is_some());
    assert!(!failed.complete);
    assert_eq!(
        failed
            .operations
            .iter()
            .map(|r| r.outcome)
            .collect::<Vec<_>>(),
        vec![
            PlanningOperationOutcome::Applied,
            PlanningOperationOutcome::Failed,
            PlanningOperationOutcome::NotReached
        ]
    );
    assert_eq!(
        serde_json::to_value(&failed.operations[0]).unwrap()["decision_note"],
        serde_json::Value::String(d.note.clone()),
        "committed prefix must retain its original approval note"
    );
    let first_id = failed.operations[0].returned_ids["cast:a"].clone();
    let count: i64 = svc
        .repository
        .pool()
        .read(|c| c.query_row("SELECT COUNT(*) FROM character", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(count, 1, "failed receipt must roll back its character too");
    assert!(
        svc.decide_planning(decision(&s, &p)).await.is_err(),
        "partially applied intent cannot be rejected as unwritten history"
    );
    let retry = svc.apply_planning(d).await.unwrap();
    assert!(retry.complete);
    assert!(retry.error.is_none());
    assert_eq!(retry.operations[0].returned_ids["cast:a"], first_id);
    let count: i64 = svc
        .repository
        .pool()
        .read(|c| c.query_row("SELECT COUNT(*) FROM character", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn cancelled_application_rolls_back_and_releases_writer_gate() {
    let (_tmp, svc) = fresh_service_local().await;
    let worker = svc.clone();
    let pool = svc.repository.pool().clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        pool.application_transaction(true, || async move {
            worker
                .create_project(CreateProjectInput {
                    name: "Cancelled".into(),
                    project_type: "novel".into(),
                    genre: "mystery".into(),
                    reader_contract: ReaderContract {
                        promise: "Never commit".into(),
                        style_notes: vec![],
                        boundaries: vec![],
                    },
                })
                .await?;
            started.send(()).unwrap();
            std::future::pending::<()>().await;
            Ok(())
        })
        .await
    });
    ready.await.unwrap();
    assert!(svc.list_projects().await.unwrap().projects.is_empty());
    task.abort();
    let _ = task.await;
    let create = svc.create_project(CreateProjectInput {
        name: "Survives".into(),
        project_type: "novel".into(),
        genre: "mystery".into(),
        reader_contract: ReaderContract {
            promise: "New start".into(),
            style_notes: vec![],
            boundaries: vec![],
        },
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), create)
        .await
        .unwrap()
        .unwrap();
    let projects = svc.list_projects().await.unwrap();
    assert_eq!(projects.projects.len(), 1);
    assert_eq!(projects.projects[0].name, "Survives");
}

#[tokio::test]
async fn root_receipt_failure_rolls_back_project_and_attachment_before_retry() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Root recovery".into(),
            scope: None,
        })
        .await
        .unwrap();
    let p = svc
        .stage_planning(artifact(&s, "project:root"))
        .await
        .unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    svc.repository.pool().write(move |c| {
  c.create_scalar_function("planning_fail_root_receipt",0,rusqlite::functions::FunctionFlags::SQLITE_UTF8,move |_|Ok(counter.fetch_add(1,Ordering::SeqCst)+1==2))?;
  c.execute_batch("CREATE TEMP TRIGGER fail_root_receipt BEFORE INSERT ON planning_operation_receipt WHEN planning_fail_root_receipt() BEGIN SELECT RAISE(ABORT,'injected root receipt failure'); END")?;Ok(())
 }).await.unwrap();
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    let failed = svc.apply_planning(d.clone()).await.unwrap();
    assert!(failed.error.is_some());
    assert!(svc.list_projects().await.unwrap().projects.is_empty());
    assert!(
        svc.get_planning_session(&s.id)
            .await
            .unwrap()
            .scope
            .is_none()
    );
    let retried = svc.apply_planning(d).await.unwrap();
    assert!(retried.complete);
    assert_eq!(svc.list_projects().await.unwrap().projects.len(), 1);
    assert_eq!(
        failed.operations[0].input_hash, retried.operations[0].input_hash,
        "failure receipt must bind the same resolved input"
    );
}

#[tokio::test]
async fn reopened_process_resumes_receipt_before_proposal_finalization() {
    let (tmp, svc) = fresh_service_local().await;
    let (s, mut root) = accepted_root(&svc).await;
    let ids = svc.repository.planning_receipts(&s.id).await.unwrap()[0]
        .returned_ids
        .clone();
    // This is the durable boundary after operation commit and before the separate
    // proposal finalizer: ledger + attachment exist, status is still staged.
    root.status = PlanningProposalStatus::Staged;
    root.decided_by = None;
    root.decided_at = None;
    root.decision_note = None;
    let id = root.id.clone();
    let data = serde_json::to_string(&root).unwrap();
    svc.repository
        .pool()
        .write(move |c| {
            c.execute(
                "UPDATE planning_proposal SET status='staged',data=?2 WHERE id=?1",
                rusqlite::params![id, data],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    drop(svc);
    let pool = crate::sqlite::SqlitePool::open(&tmp.path().join("planning.db"))
        .await
        .unwrap();
    let svc = SqliteSpindleService::new(Repository::with_model_router(
        pool,
        tmp.path().join("data"),
        crate::ai::ModelRouter::local_only(),
    ));
    let report = accept(&svc, &s, &root).await;
    assert_eq!(report.operations[0].returned_ids, ids);
    assert_eq!(report.operations[0].actor, "author");
    assert_eq!(svc.list_projects().await.unwrap().projects.len(), 1);
    let scope = s.scope.unwrap();
    let books = svc
        .repository
        .list_books_by_project(&scope.project_id)
        .await
        .unwrap();
    assert_eq!(books.len(), 1);
    assert_eq!(
        svc.repository
            .list_chapters_by_book(&books[0].id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn changed_plan_is_rejected_without_overwriting_the_newer_plan() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    let p = bundle(
        &svc,
        &s,
        PlanningLevel::Chapter,
        &[&root],
        vec![planned(
            "plan:first",
            "plan_chapter",
            json!({"book_number":1,"chapter_number":1,"synopsis":"Proposed intent","scenes":[]}),
        )],
    )
    .await;
    let scope = s.scope.clone().unwrap();
    let newer = svc
        .plan_chapter(PlanChapterInput {
            project_id: scope.project_id,
            book_number: 1,
            chapter_number: 1,
            pov_character_id: None,
            synopsis: "Newer author intent".into(),
            target_theme_ids: vec![],
            target_conflict_ids: vec![],
            target_plot_line_ids: vec![],
            scenes: vec![],
        })
        .await
        .unwrap();
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    assert!(
        svc.apply_planning(d)
            .await
            .unwrap_err()
            .to_string()
            .contains("stale planning base")
    );
    assert_eq!(
        svc.repository
            .get_chapter_plan(&newer.chapter_plan_id)
            .await
            .unwrap()
            .synopsis,
        "Newer author intent"
    );
}

#[tokio::test]
async fn drafted_chapter_is_immutable_at_acceptance() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    let p = bundle(
        &svc,
        &s,
        PlanningLevel::Chapter,
        &[&root],
        vec![planned(
            "plan:first",
            "plan_chapter",
            json!({"book_number":1,"chapter_number":1,"synopsis":"Unwritten intent","scenes":[]}),
        )],
    )
    .await;
    let scope = s.scope.clone().unwrap();
    let input:SaveSceneDraftInput=serde_json::from_value(json!({"project_id":scope.project_id,"book_number":1,"chapter_number":1,"scene_order":1,"full_text":"Ada found the seal under the harbor ledger and carried it to the gate before the last tide withdrew beyond the channel.","summary":"A seal is found","content_rating":"general"})).unwrap();
    svc.save_scene_draft(input).await.unwrap();
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    assert!(
        svc.apply_planning(d)
            .await
            .unwrap_err()
            .to_string()
            .contains("persisted prose")
    );
}

#[tokio::test]
async fn concurrent_root_acceptance_never_overwrites_the_committed_receipt() {
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Concurrent receipt recovery".into(),
            scope: None,
        })
        .await
        .unwrap();
    let p = svc
        .stage_planning(artifact(&s, "project:root"))
        .await
        .unwrap();
    let mut a = decision(&s, &p);
    a.decision = PlanningDecision::Accept;
    a.actor = "author:a".into();
    let mut b = a.clone();
    b.actor = "author:b".into();
    let (a, b) = tokio::join!(svc.apply_planning(a), svc.apply_planning(b));
    assert!(
        a.as_ref().is_ok_and(|r| r.error.is_none()) || b.as_ref().is_ok_and(|r| r.error.is_none())
    );
    let receipts = svc.repository.planning_receipts(&s.id).await.unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].outcome, PlanningOperationOutcome::Applied);
    assert_eq!(receipts[0].returned_ids.len(), 4);
    let s = svc.get_planning_session(&s.id).await.unwrap();
    accept(&svc, &s, &p).await;
    assert_eq!(svc.list_projects().await.unwrap().projects.len(), 1);
}

#[tokio::test]
async fn active_run_freezes_roots_and_existing_plans_but_allows_missing_future_chapters() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    let scope = s.scope.clone().unwrap();
    svc.plan_chapter(PlanChapterInput {
        project_id: scope.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: None,
        synopsis: "Existing run intent".into(),
        target_theme_ids: vec![],
        target_conflict_ids: vec![],
        target_plot_line_ids: vec![],
        scenes: vec![],
    })
    .await
    .unwrap();
    let existing = bundle(
        &svc,
        &s,
        PlanningLevel::Chapter,
        &[&root],
        vec![planned(
            "plan:replace",
            "plan_chapter",
            json!({"book_number":1,"chapter_number":1,"synopsis":"Changed run intent","scenes":[]}),
        )],
    )
    .await;
    let project = scope.project_id.clone();
    let branch = scope.branch_id.clone();
    svc.repository.pool().write(move |c|{c.execute("INSERT INTO authoring_run(id,project_id,active_branch_id,book_number,start_chapter,end_chapter,checkpoint_interval,last_checkpoint_end_chapter,artifacts_dir,editorial_directives,status,created_at,updated_at) VALUES('authoring_run:planning-test',?1,?2,1,1,3,3,0,'test-artifacts','[]','active',1,1)",rusqlite::params![project,branch])?;Ok(())}).await.unwrap();
    let mut d = decision(&s, &existing);
    d.decision = PlanningDecision::Accept;
    assert!(
        svc.apply_planning(d)
            .await
            .unwrap_err()
            .to_string()
            .contains("plan amendments")
    );
    let project = scope.project_id.clone();
    svc.repository.pool().write(move |c| {
        c.execute("INSERT INTO authoring_run_chapter(authoring_run_id,chapter_number,planned,synopsis,pov_character_id,status,summary_saved) VALUES('authoring_run:planning-test',1,1,'Cached intent',NULL,'pending',0)",[])?;
        c.execute("DELETE FROM chapter_plan WHERE project_id=?1",[project])?;Ok(())
    }).await.unwrap();
    let cached=bundle(&svc,&s,PlanningLevel::Chapter,&[&root],vec![planned("plan:cached","plan_chapter",json!({"book_number":1,"chapter_number":1,"synopsis":"Do not overwrite the cached run plan","scenes":[]}))]).await;
    let mut d = decision(&s, &cached);
    d.decision = PlanningDecision::Accept;
    assert!(
        svc.apply_planning(d)
            .await
            .unwrap_err()
            .to_string()
            .contains("plan amendments")
    );
    let missing=bundle(&svc,&s,PlanningLevel::Chapter,&[&root],vec![planned("plan:future","plan_chapter",json!({"book_number":1,"chapter_number":2,"synopsis":"A missing future chapter","scenes":[]}))]).await;
    accept(&svc, &s, &missing).await;
    let cast = bundle(
        &svc,
        &s,
        PlanningLevel::CastWorld,
        &[&root],
        vec![planned(
            "cast:new",
            "create_character",
            json!({"name":"New lead","summary":"Late root","role":"lead"}),
        )],
    )
    .await;
    let mut d = decision(&s, &cast);
    d.decision = PlanningDecision::Accept;
    assert!(
        svc.apply_planning(d)
            .await
            .unwrap_err()
            .to_string()
            .contains("roots are frozen")
    );
}

#[tokio::test]
async fn multiple_book_allocations_advance_only_their_own_verified_snapshot() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    let books = bundle(
        &svc,
        &s,
        PlanningLevel::Book,
        &[&root],
        vec![
            planned("book:second", "create_book", json!({"title":"Second"})),
            planned("book:third", "create_book", json!({"title":"Third"})),
        ],
    )
    .await;
    let report = accept(&svc, &s, &books).await;
    let numbers = report
        .operations
        .iter()
        .map(|r| match r.result.as_ref().unwrap() {
            PlanningAppliedOutput::CreateBook(out) => out.book_number,
            _ => panic!("unexpected operation result"),
        })
        .collect::<Vec<_>>();
    assert_eq!(numbers, vec![2, 3]);
    let retry = accept(&svc, &s, &books).await;
    assert_eq!(
        report.operations[0].returned_ids,
        retry.operations[0].returned_ids
    );
    assert_eq!(
        svc.repository
            .list_books_by_project(&s.scope.unwrap().project_id)
            .await
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn deferred_search_failure_stays_queued_without_unapplying_canon() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    svc.repository.pool().write(|c|{c.execute_batch("CREATE TEMP TRIGGER fail_planning_search BEFORE INSERT ON search_embedding BEGIN SELECT RAISE(ABORT,'injected search failure'); END")?;Ok(())}).await.unwrap();
    let p = bundle(
        &svc,
        &s,
        PlanningLevel::CastWorld,
        &[&root],
        vec![planned(
            "cast:ada",
            "create_character",
            json!({"name":"Ada","summary":"Archivist","role":"lead"}),
        )],
    )
    .await;
    let report = accept(&svc, &s, &p).await;
    assert_eq!(report.pending_search_refreshes, 1);
    assert_eq!(
        report.operations[0].outcome,
        PlanningOperationOutcome::Applied
    );
    let id = report.operations[0].returned_ids["cast:ada"].clone();
    assert!(svc.repository.get_character(&id).await.is_ok());
    svc.repository
        .pool()
        .write(|c| {
            c.execute_batch("DROP TRIGGER fail_planning_search")?;
            Ok(())
        })
        .await
        .unwrap();
    let retry = accept(&svc, &s, &p).await;
    assert_eq!(retry.pending_search_refreshes, 0);
    assert_eq!(retry.operations[0].returned_ids["cast:ada"], id);
}

#[tokio::test]
async fn phase_a_database_upgrades_and_resumes_an_unaccepted_root() {
    let (tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Legacy staged root".into(),
            scope: None,
        })
        .await
        .unwrap();
    let root = svc
        .stage_planning(artifact(&s, "project:root"))
        .await
        .unwrap();
    let path = tmp.path().join("phase-a.db");
    {
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        crate::sqlite::migrations::runner()
            .set_target(refinery::Target::Version(49))
            .run(&mut conn)
            .unwrap();
        conn.execute("INSERT INTO planning_session(id,project_id,branch_id,data,created_at) VALUES(?1,NULL,NULL,?2,?3)",rusqlite::params![s.id,serde_json::to_string(&s).unwrap(),s.created_at]).unwrap();
        let mut legacy = serde_json::to_value(&root).unwrap();
        legacy.as_object_mut().unwrap().remove("base_state");
        conn.execute("INSERT INTO planning_proposal(id,session_id,payload_hash,status,data,created_at) VALUES(?1,?2,?3,'staged',?4,?5)",rusqlite::params![root.id,s.id,root.payload_hash,serde_json::to_string(&legacy).unwrap(),root.created_at]).unwrap();
    }
    let pool = crate::sqlite::SqlitePool::open(&path).await.unwrap();
    let upgraded = SqliteSpindleService::new(Repository::with_model_router(
        pool,
        tmp.path().join("legacy-data"),
        crate::ai::ModelRouter::local_only(),
    ));
    let report = accept(&upgraded, &s, &root).await;
    assert_eq!(report.operations[0].actor, "author");
    assert!(!report.operations[0].decision_note.is_empty());
    assert_eq!(upgraded.list_projects().await.unwrap().projects.len(), 1);
}

#[tokio::test]
async fn reader_contract_changes_invalidate_previously_staged_detail() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    let p=bundle(&svc,&s,PlanningLevel::Chapter,&[&root],vec![planned("plan:first","plan_chapter",json!({"book_number":1,"chapter_number":1,"synopsis":"Old contract intent","scenes":[]}))]).await;
    svc.update_entity(UpdateEntityInput {
        entity_type: "project".into(),
        entity_id: s.scope.as_ref().unwrap().project_id.clone(),
        changes: json!({"promise":"A different reader promise"}),
        allow_rename: None,
    })
    .await
    .unwrap();
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    assert!(
        svc.apply_planning(d)
            .await
            .unwrap_err()
            .to_string()
            .contains("reader_contract")
    );
}

#[tokio::test]
async fn conflicting_book_id_and_number_fail_before_creating_a_chapter() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    let scope = s.scope.as_ref().unwrap();
    let book = svc
        .create_book(CreateBookInput {
            project_id: scope.project_id.clone(),
            title: Some("Second".into()),
        })
        .await
        .unwrap();
    let p=bundle(&svc,&s,PlanningLevel::Chapter,&[&root],vec![planned("chapter:wrong-book","create_chapter",json!({"book_id":{"kind":"existing_record_id","value":book.book_id},"book_number":1,"chapter_number":2,"title":"Wrong target"}))]).await;
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    assert!(svc.apply_planning(d).await.is_err());
    assert!(
        svc.repository
            .list_chapters_by_book(&book.book_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn lost_application_transaction_cannot_fall_through_to_autocommit_writes() {
    let (_tmp, svc) = fresh_service_local().await;
    let pool = svc.repository.pool().clone();
    let result = pool
        .application_transaction(true, || async {
            pool.write(|c| {
                c.execute_batch("ROLLBACK")?;
                Ok(())
            })
            .await?;
            svc.create_project(CreateProjectInput {
                name: "Must not leak".into(),
                project_type: "novel".into(),
                genre: "mystery".into(),
                reader_contract: ReaderContract {
                    promise: "Atomic boundary".into(),
                    style_notes: vec![],
                    boundaries: vec![],
                },
            })
            .await?;
            Ok(())
        })
        .await;
    assert!(result.is_err());
    assert!(
        svc.list_projects().await.unwrap().projects.is_empty(),
        "a lost transaction must not allow a nested service to commit outside the boundary"
    );
}

#[tokio::test]
async fn project_wide_contract_is_frozen_by_a_run_on_another_branch() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, _root) = accepted_root(&svc).await;
    let scope = s.scope.unwrap();
    let project = scope.project_id.clone();
    let branch = scope.branch_id.clone();
    svc.repository.pool().write(move |c|{c.execute("INSERT INTO authoring_run(id,project_id,active_branch_id,book_number,start_chapter,end_chapter,checkpoint_interval,last_checkpoint_end_chapter,artifacts_dir,editorial_directives,status,created_at,updated_at) VALUES('authoring_run:other-branch',?1,?2,1,1,3,3,0,'test-artifacts','[]','active',1,1)",rusqlite::params![project,branch])?;Ok(())}).await.unwrap();
    let child = svc
        .create_branch(CreateBranchInput {
            project_id: scope.project_id.clone(),
            name: "Alternative".into(),
            branch_type: "experiment".into(),
            description: None,
            parent_branch_id: Some(scope.branch_id),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: scope.project_id.clone(),
        branch_id: child.branch_id.clone(),
    })
    .await
    .unwrap();
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "An alternate branch".into(),
            scope: Some(PlanningScope {
                project_id: scope.project_id,
                branch_id: child.branch_id,
            }),
        })
        .await
        .unwrap();
    let p = bundle(
        &svc,
        &s,
        PlanningLevel::Contract,
        &[],
        vec![planned(
            "contract:changed",
            "update_reader_contract",
            json!({"promise":"A different contract"}),
        )],
    )
    .await;
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    assert!(svc.apply_planning(d).await.is_err());
    let book = bundle(
        &svc,
        &s,
        PlanningLevel::Book,
        &[],
        vec![planned(
            "book:blocked",
            "create_book",
            json!({"title":"No global allocation during another run"}),
        )],
    )
    .await;
    let mut d = decision(&s, &book);
    d.decision = PlanningDecision::Accept;
    assert!(svc.apply_planning(d).await.is_err());
}

#[tokio::test]
async fn explicitly_selected_contract_parent_advances_dependent_context_snapshot() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    let contract = bundle(
        &svc,
        &s,
        PlanningLevel::Contract,
        &[&root],
        vec![planned(
            "contract:updated",
            "update_reader_contract",
            json!({"promise":"An explicitly approved new contract"}),
        )],
    )
    .await;
    let chapter=bundle(&svc,&s,PlanningLevel::Chapter,&[&contract],vec![planned("plan:updated-contract","plan_chapter",json!({"book_number":1,"chapter_number":1,"synopsis":"New contract intent","scenes":[]}))]).await;
    let mut d = decision(&s, &chapter);
    d.decision = PlanningDecision::Accept;
    d.expected_hashes
        .insert(contract.id.clone(), contract.payload_hash.clone());
    let report = svc.apply_planning(d).await.unwrap();
    assert!(report.complete);
    assert!(report.error.is_none());
}

#[tokio::test]
async fn active_run_inherited_plan_requires_an_amendment_instead_of_initial_override() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, _root) = accepted_root(&svc).await;
    let scope = s.scope.unwrap();
    svc.plan_chapter(PlanChapterInput {
        project_id: scope.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: None,
        synopsis: "Ancestor's future plan".into(),
        target_theme_ids: vec![],
        target_conflict_ids: vec![],
        target_plot_line_ids: vec![],
        scenes: vec![],
    })
    .await
    .unwrap();
    let child = svc
        .create_branch(CreateBranchInput {
            project_id: scope.project_id.clone(),
            name: "Inherited".into(),
            branch_type: "experiment".into(),
            description: None,
            parent_branch_id: Some(scope.branch_id),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: scope.project_id.clone(),
        branch_id: child.branch_id.clone(),
    })
    .await
    .unwrap();
    let project = scope.project_id.clone();
    let branch = child.branch_id.clone();
    svc.repository.pool().write(move |c|{c.execute("INSERT INTO authoring_run(id,project_id,active_branch_id,book_number,start_chapter,end_chapter,checkpoint_interval,last_checkpoint_end_chapter,artifacts_dir,editorial_directives,status,created_at,updated_at) VALUES('authoring_run:inherited',?1,?2,1,1,3,3,0,'test-artifacts','[]','active',1,1)",rusqlite::params![project,branch])?;Ok(())}).await.unwrap();
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Respect inherited intent".into(),
            scope: Some(PlanningScope {
                project_id: scope.project_id,
                branch_id: child.branch_id,
            }),
        })
        .await
        .unwrap();
    let p=bundle(&svc,&s,PlanningLevel::Chapter,&[],vec![planned("plan:inherited","plan_chapter",json!({"book_number":1,"chapter_number":1,"synopsis":"An initial-planner override","scenes":[]}))]).await;
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    assert!(
        svc.apply_planning(d)
            .await
            .unwrap_err()
            .to_string()
            .contains("plan amendments")
    );
}

#[tokio::test]
async fn composed_outline_and_plan_advance_only_verified_approved_snapshots() {
    use serde_json::json;
    for separate_proposals in [false, true] {
        let (_tmp, svc) = fresh_service_local().await;
        let (s, root) = accepted_root(&svc).await;
        let outline = planned(
            "outline:composed",
            "set_chapter_outline",
            json!({
                "book_number":1,"chapter_number":1,"content":"Approved outline"
            }),
        );
        let plan = planned(
            "plan:composed",
            "plan_chapter",
            json!({
                "book_number":1,"chapter_number":1,"synopsis":"Approved plan","scenes":[]
            }),
        );
        let first = bundle(
            &svc,
            &s,
            PlanningLevel::Chapter,
            &[&root],
            if separate_proposals {
                vec![outline]
            } else {
                vec![outline, plan.clone()]
            },
        )
        .await;
        let last = if separate_proposals {
            bundle(&svc, &s, PlanningLevel::Chapter, &[&first], vec![plan]).await
        } else {
            first.clone()
        };
        let mut d = decision(&s, &last);
        d.decision = PlanningDecision::Accept;
        d.expected_hashes
            .insert(first.id.clone(), first.payload_hash.clone());
        let report = svc.apply_planning(d.clone()).await.unwrap();
        assert!(report.complete, "{:?}", report.error);
        assert_eq!(report.operations.len(), 2);
        assert!(svc.apply_planning(d).await.unwrap().complete);
    }
}

#[tokio::test]
async fn approved_outline_does_not_refresh_an_unrelated_externally_changed_plan() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let (s, root) = accepted_root(&svc).await;
    let p = bundle(
        &svc,
        &s,
        PlanningLevel::Chapter,
        &[&root],
        vec![
            planned(
                "outline:unrelated",
                "set_chapter_outline",
                json!({
                    "book_number":1,"chapter_number":1,"content":"Approved outline"
                }),
            ),
            planned(
                "plan:external",
                "plan_chapter",
                json!({
                    "book_number":1,"chapter_number":2,"synopsis":"Stale proposed plan","scenes":[]
                }),
            ),
        ],
    )
    .await;
    let newer: PlanChapterInput = serde_json::from_value(json!({
        "project_id":s.scope.as_ref().unwrap().project_id,"book_number":1,"chapter_number":2,
        "synopsis":"External author edit","scenes":[]
    }))
    .unwrap();
    let newer = svc.plan_chapter(newer).await.unwrap();
    let mut d = decision(&s, &p);
    d.decision = PlanningDecision::Accept;
    assert!(
        svc.apply_planning(d)
            .await
            .unwrap_err()
            .to_string()
            .contains("stale planning base")
    );
    assert!(
        svc.repository
            .planning_receipts(&s.id)
            .await
            .unwrap()
            .iter()
            .all(|r| r.proposal_id != p.id)
    );
    assert_eq!(
        svc.repository
            .get_chapter_plan(&newer.chapter_plan_id)
            .await
            .unwrap()
            .synopsis,
        "External author edit"
    );
}
