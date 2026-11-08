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
        target_range: None,
        schedule: PlanningSchedule::default(),
        assumptions: Vec::new(),
        author_questions: Vec::new(),
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

fn read_full_http_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let mut data = Vec::new();
    let mut buf = [0_u8; 8192];
    let mut expected_total = None;
    loop {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if expected_total.is_none()
            && let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(&data[..pos]);
            let content_len = headers
                .lines()
                .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            expected_total = Some(pos + 4 + content_len);
        }
        if let Some(total) = expected_total
            && data.len() >= total
        {
            break;
        }
    }
    String::from_utf8_lossy(&data).to_string()
}

fn spawn_planning_http_server(
    responses: Vec<(u16, serde_json::Value)>,
) -> (
    String,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    std::thread::JoinHandle<usize>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind planning test listener");
    let addr = listener.local_addr().unwrap();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_clone = std::sync::Arc::clone(&captured);
    let handle = std::thread::spawn(move || {
        let mut count = 0usize;
        for (status, body_json) in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let req = read_full_http_request(&mut stream);
            let req_body = req
                .split_once("\r\n\r\n")
                .map(|(_, b)| b.to_string())
                .unwrap_or_default();
            captured_clone.lock().unwrap().push(req_body);
            let body = body_json.to_string();
            let status_text = if status == 200 { "OK" } else { "Error" };
            let resp = format!(
                "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
            count += 1;
        }
        count
    });
    (format!("http://{addr}/v1"), captured, handle)
}

async fn service_with_toml(config_toml: &str) -> (tempfile::TempDir, SqliteSpindleService) {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = crate::sqlite::SqlitePool::open(&tmp.path().join("planning.db"))
        .await
        .unwrap();
    let config_path = tmp.path().join("spindle.toml");
    std::fs::write(&config_path, config_toml).unwrap();
    let router = crate::ai::ModelRouter::local_only();
    router
        .configure(Some(&config_path.display().to_string()))
        .unwrap();
    let repo = Repository::with_model_router(pool, tmp.path().join("data"), router);
    (tmp, SqliteSpindleService::new(repo))
}

#[tokio::test]
async fn route_generation_real_agent_stages_all_levels_and_applies_only_on_human_accept() {
    use serde_json::json;
    let contract_payload = json!({
        "schema_version": 1,
        "level": "contract",
        "rationale": "Establish the harbor mystery contract.",
        "assumptions": ["Single-POV close third mystery"],
        "operations": [{
            "output_ref": "contract:project",
            "operation": {
                "operation": "create_project",
                "input": {
                    "name": "Salt Harbor",
                    "project_type": "novel",
                    "genre": "mystery",
                    "reader_contract": {
                        "promise": "Uncover the vanished heir",
                        "style_notes": ["Close atmospheric tension"],
                        "boundaries": ["No omniscient reveals"]
                    }
                }
            }
        }]
    });
    let seed_payload = json!({
        "schema_version": 1,
        "level": "cast_world",
        "rationale": "Introduce protagonist, rival, harbor location, and tide rule.",
        "operations": [
            {
                "output_ref": "cast:ada",
                "operation": {
                    "operation": "create_character",
                    "input": {
                        "name": "Ada Venn",
                        "summary": "Harbor chart-keeper",
                        "role": "protagonist",
                        "realm": null
                    }
                }
            },
            {
                "output_ref": "cast:corin",
                "operation": {
                    "operation": "create_character",
                    "input": {
                        "name": "Corin Vale",
                        "summary": "Harbor magistrate",
                        "role": "antagonist",
                        "realm": null
                    }
                }
            },
            {
                "output_ref": "place:pier",
                "operation": {
                    "operation": "create_location",
                    "input": {
                        "name": "North Pier",
                        "kind": "harbor",
                        "realm": null,
                        "summary": "Fog-bound stone pier"
                    }
                }
            },
            {
                "output_ref": "rule:tide",
                "operation": {
                    "operation": "create_world_rule",
                    "input": {
                        "rule_name": "Floodgate Lock",
                        "rule_type": "physical",
                        "description": "The inner basin cannot open at low tide",
                        "established_in": {"book_number": 1, "chapter_number": 1, "note": null}
                    }
                }
            },
            {
                "output_ref": "rel:ada-corin",
                "operation": {
                    "operation": "create_relationship",
                    "input": {
                        "character_a_id": {"kind": "proposal_ref", "value": "cast:ada"},
                        "character_b_id": {"kind": "proposal_ref", "value": "cast:corin"},
                        "relationship_type": "rivalry",
                        "initial_trust": 20,
                        "initial_tension": 75,
                        "dynamics": ["Guarded suspicion"]
                    }
                }
            }
        ]
    });
    let series_arc_payload = json!({
        "schema_version": 1,
        "level": "series_arc",
        "rationale": "Bind theme, conflict, plot line, promise, arc, and pacing.",
        "operations": [
            {
                "output_ref": "theme:truth",
                "operation": {
                    "operation": "create_theme",
                    "input": {
                        "theme_statement": "Truth costs safety",
                        "thesis_antithesis": "Silence preserves order vs exposure frees the harbor",
                        "introduction_point": {"book_number": 1, "chapter_number": 1, "scene_order": 1, "note": null},
                        "resolution_point": {"book_number": 1, "chapter_number": 3, "scene_order": 2, "note": null}
                    }
                }
            },
            {
                "output_ref": "conflict:ledger",
                "operation": {
                    "operation": "create_conflict",
                    "input": {
                        "name": "The Altered Ledger",
                        "conflict_type": "investigation",
                        "stakes": "Ada's livelihood and the heir's survival",
                        "escalation_stages": ["Missing page", "Forged seal"],
                        "expected_total_cycles": 2,
                        "try_fail_cycles": [{"attempt_order": 1, "label": "Inspect manifest", "outcome": "Locked by magistrate", "cost": "Ada flagged", "revelation": "Seal was forged"}],
                        "stated_consequences": [{"description": "Harbor charter revoked", "stated_at": {"book_number": 1, "chapter_number": 1, "scene_order": 1, "note": null}, "must_demonstrate_by": "chapter 3", "delivered": false}]
                    }
                }
            },
            {
                "output_ref": "plot:main",
                "operation": {
                    "operation": "create_plot_line",
                    "input": {
                        "name": "Vanished Heir",
                        "plot_type": "main",
                        "summary": "Trace the forged manifest to the hidden heir",
                        "status": "active",
                        "convergence_points": [{"book_number": 1, "chapter_number": 3, "scene_order": 2, "note": "Basin confrontation"}],
                        "connected_conflict_ids": [{"kind": "proposal_ref", "value": "conflict:ledger"}],
                        "connected_theme_ids": [{"kind": "proposal_ref", "value": "theme:truth"}]
                    }
                }
            },
            {
                "output_ref": "promise:seal",
                "operation": {
                    "operation": "create_narrative_promise",
                    "input": {
                        "promise_type": "clue",
                        "description": "The wax seal impression matches the magistrate's ring",
                        "planted_at": {"book_number": 1, "chapter_number": 1, "scene_order": 1, "note": null},
                        "planned_payoff": {"book_number": 1, "chapter_number": 3, "scene_order": 2, "note": null},
                        "notes": ["Planted in Ch1"]
                    }
                }
            },
            {
                "output_ref": "arc:ada",
                "operation": {
                    "operation": "create_character_arc",
                    "input": {
                        "character_id": {"kind": "proposal_ref", "value": "cast:ada"},
                        "arc_type": "positive_change",
                        "starting_state": "Complicit silence",
                        "ending_state": "Public witness",
                        "milestones": [{"label": "Break seal", "placement": {"book_number": 1, "chapter_number": 2, "scene_order": 1, "note": null}, "description": "Copies the hidden ledger", "unlocks": ["Confront Corin"], "reached_at": null}],
                        "thematic_purpose": "Chooses truth over safety",
                        "connected_theme_ids": [{"kind": "proposal_ref", "value": "theme:truth"}]
                    }
                }
            }
        ],
        "schedule": {
            "promise_events": [
                {
                    "promise_ref": {"kind": "proposal_ref", "value": "promise:seal"},
                    "action": "plant",
                    "placement": {"book_number": 1, "chapter_number": 1, "scene_order": 1, "note": null}
                },
                {
                    "promise_ref": {"kind": "proposal_ref", "value": "promise:seal"},
                    "action": "payoff",
                    "placement": {"book_number": 1, "chapter_number": 3, "scene_order": 2, "note": null}
                }
            ]
        }
    });
    let book_payload = json!({
        "schema_version": 1,
        "level": "book",
        "rationale": "Set Book 1 outline.",
        "operations": [{
            "output_ref": "outline:book1",
            "operation": {
                "operation": "set_book_outline",
                "input": {
                    "book_number": 1,
                    "book_id": {"kind": "proposal_ref", "value": "contract:project:book"},
                    "format": "markdown",
                    "content": "# Book 1\n- Act I: Missing manifest\n- Act II: Basin lock\n- Act III: Public reckoning"
                }
            }
        }]
    });
    let chapter_payload = json!({
        "schema_version": 1,
        "level": "chapter",
        "rationale": "Outline Chapter 1 beats and POV/intensity schedule.",
        "operations": [{
            "output_ref": "outline:ch1",
            "operation": {
                "operation": "set_chapter_outline",
                "input": {
                    "book_number": 1,
                    "chapter_number": 1,
                    "entity_id": {"kind": "proposal_ref", "value": "contract:project:chapter"},
                    "content": "Ada inspects the night manifest at North Pier.",
                    "beats": [
                        {"order": 1, "summary": "Discover missing page", "scene_id": null, "status": "planned"},
                        {"order": 2, "summary": "Spot Corin's wax seal", "scene_id": null, "status": "planned"}
                    ]
                }
            }
        }],
        "schedule": {
            "pov_plan": [{
                "book_number": 1,
                "chapter_number": 1,
                "scene_order": 1,
                "pov_character_id": {"kind": "proposal_ref", "value": "cast:ada"},
                "rationale": "Ada discovers the discrepancy firsthand"
            }],
            "intensity_targets": [{
                "book_number": 1,
                "chapter_number": 1,
                "target_intensity": 0.65,
                "recovery_beat": false,
                "note": "Rising opening tension"
            }]
        }
    });
    let scene_payload = json!({
        "schema_version": 1,
        "level": "scene",
        "rationale": "Concrete scene manifests for Chapter 1.",
        "operations": [{
            "output_ref": "plan:ch1-scenes",
            "operation": {
                "operation": "plan_chapter",
                "input": {
                    "book_number": 1,
                    "chapter_number": 1,
                    "pov_character_id": {"kind": "proposal_ref", "value": "cast:ada"},
                    "synopsis": "Ada finds the altered manifest and forged wax seal at North Pier.",
                    "target_theme_ids": [{"kind": "proposal_ref", "value": "theme:truth"}],
                    "target_conflict_ids": [{"kind": "proposal_ref", "value": "conflict:ledger"}],
                    "target_plot_line_ids": [{"kind": "proposal_ref", "value": "plot:main"}],
                    "scenes": [
                        {
                            "scene_order": 1,
                            "summary": "Ada checks the tide ledger at North Pier and finds a torn leaf.",
                            "beat_structure": ["Open ledger", "Find missing page"],
                            "character_ids": [{"kind": "proposal_ref", "value": "cast:ada"}],
                            "location_id": {"kind": "proposal_ref", "value": "place:pier"},
                            "content_rating": "teen",
                            "purpose": "Plant the altered ledger mystery"
                        },
                        {
                            "scene_order": 2,
                            "summary": "Corin interrupts Ada and locks the registry chest with his ring seal.",
                            "beat_structure": ["Corin arrives", "Seal impression"],
                            "character_ids": [
                                {"kind": "proposal_ref", "value": "cast:ada"},
                                {"kind": "proposal_ref", "value": "cast:corin"}
                            ],
                            "location_id": {"kind": "proposal_ref", "value": "place:pier"},
                            "content_rating": "teen",
                            "purpose": "Establish rivalry and plant the wax seal clue"
                        }
                    ]
                }
            }
        }]
    });

    let wrap = |content: serde_json::Value| {
        (
            200,
            json!({
                "choices": [{"message": {"content": content.to_string()}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 120, "completion_tokens": 80}
            }),
        )
    };
    let (plan_endpoint, plan_reqs, plan_handle) = spawn_planning_http_server(vec![
        wrap(contract_payload),
        wrap(seed_payload),
        wrap(series_arc_payload),
    ]);
    let (outline_endpoint, outline_reqs, outline_handle) = spawn_planning_http_server(vec![
        wrap(book_payload),
        wrap(chapter_payload),
        wrap(scene_payload),
    ]);

    let (_tmp, svc) = service_with_toml(&format!(
        r#"
[health_check]
enabled = false

[[agents]]
id = "planner-agent"
name = "Planner Agent"
provider = "openai-compatible"
endpoint = "{plan_endpoint}"
model = "planner-v1"
ratings = ["general", "teen", "mature"]

[[agents]]
id = "outliner-agent"
name = "Outliner Agent"
provider = "openai-compatible"
endpoint = "{outline_endpoint}"
model = "outliner-v1"
ratings = ["general", "teen", "mature"]

[[routing]]
route = "plan"
agent = "planner-agent"

[[routing]]
route = "outline"
agent = "outliner-agent"
"#
    ))
    .await;

    let session = svc
        .begin_planning(BeginPlanningInput {
            premise: "A harbor chart-keeper discovers a forged manifest hiding a missing heir."
                .into(),
            scope: None,
        })
        .await
        .unwrap();

    for expected_level in PlanningLevel::all() {
        let report = svc
            .advance_planning(AdvancePlanningInput {
                session_id: session.id.clone(),
                scope: None,
                level: None,
                parents: None,
                target_range: None,
                allow_review_backend: None,
                rating_ceiling: Some(spindle_core::models::ContentRating::Mature),
                budget: None,
                host_submission: None,
                retry_key: None,
            })
            .await
            .unwrap();
        assert_eq!(report.level, expected_level);
        assert_eq!(report.outcome, PlanningSessionOutcome::Staged);
        let prop = report.proposal.as_ref().expect("staged proposal");
        assert_eq!(prop.status, PlanningProposalStatus::Staged);
        let receipt = prop.generation_receipt.as_ref().expect("receipt");
        assert_eq!(receipt.source, PlanningOutputSource::Model);
        assert_eq!(receipt.prompt_id, expected_level.prompt_id());
        assert_eq!(receipt.attempts_used, 1);
        assert!(!receipt.repaired);
        assert_eq!(receipt.calls.len(), 1);
    }

    assert_eq!(plan_handle.join().unwrap(), 3);
    assert_eq!(outline_handle.join().unwrap(), 3);
    assert_eq!(plan_reqs.lock().unwrap().len(), 3);
    assert_eq!(outline_reqs.lock().unwrap().len(), 3);

    // Zero direct bible writes while proposals remain staged.
    assert!(
        svc.list_projects().await.unwrap().projects.is_empty(),
        "model output must land only as staged proposals, never directly in bible tables"
    );

    let staged = svc
        .list_planning_proposals(&session.id, None)
        .await
        .unwrap();
    assert_eq!(staged.len(), 6);

    // Accept the contract root first, binding the session to the created project/branch.
    let root = &staged[0];
    let root_apply = svc
        .apply_planning(DecidePlanningInput {
            session_id: session.id.clone(),
            scope: None,
            expected_hashes: BTreeMap::from([(root.id.clone(), root.payload_hash.clone())]),
            decision: PlanningDecision::Accept,
            actor: "author".into(),
            note: "Accept contract".into(),
        })
        .await
        .unwrap();
    assert!(root_apply.complete, "{:?}", root_apply.error);

    let bound_session = svc.get_planning_session(&session.id).await.unwrap();
    let scope = bound_session.scope.clone().expect("bound scope");
    assert_eq!(bound_session.usage.model_calls, 6);
    assert_eq!(bound_session.usage.input_tokens, 720);
    assert_eq!(bound_session.usage.output_tokens, 480);

    // Now accept all remaining 5 levels in dependency order.
    let remaining_hashes: BTreeMap<String, String> = staged[1..]
        .iter()
        .map(|p| (p.id.clone(), p.payload_hash.clone()))
        .collect();
    let rest_apply = svc
        .apply_planning(DecidePlanningInput {
            session_id: session.id.clone(),
            scope: Some(scope.clone()),
            expected_hashes: remaining_hashes,
            decision: PlanningDecision::Accept,
            actor: "author".into(),
            note: "Accept seed, series arc, book, chapter, and scene manifests".into(),
        })
        .await
        .unwrap();
    assert!(rest_apply.complete, "{:?}", rest_apply.error);
    assert_eq!(
        svc.list_projects().await.unwrap().projects.len(),
        1,
        "human acceptance commits planned project and downstream bible entities"
    );
}

#[tokio::test]
async fn route_generation_missing_routes_and_local_stubs_report_honestly_with_host_fallback() {
    // 1. Default local_only router: missing `plan` route, allow_review_backend=false
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "A lighthouse keeper hears a bell underwater.".into(),
            scope: None,
        })
        .await
        .unwrap();
    let r1 = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: Some(false),
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(r1.outcome, PlanningSessionOutcome::NeedsRoute);
    assert!(r1.proposal.is_none());
    assert!(matches!(
        r1.route_problem,
        Some(PlanningRouteProblem::MissingRoute {
            ref route_task,
            allow_review_backend: false
        }) if route_task == "plan"
    ));
    let host_req = r1.host_request.expect("host fallback request");
    assert_eq!(host_req.prompt_id, PlanningPromptId::PlanningContractV1);
    assert!(!host_req.prompt_hash.is_empty());
    assert!(!host_req.schema_hash.is_empty());

    // 2. allow_review_backend=true when `review` is still the built-in local stub -> LocalStubRejected
    let r2 = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: Some(true),
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(r2.outcome, PlanningSessionOutcome::NeedsRoute);
    assert!(matches!(
        r2.route_problem,
        Some(PlanningRouteProblem::LocalStubRejected {
            ref route_task,
            ref resolved_route,
            builtin_default: true,
            ..
        }) if route_task == "plan" && resolved_route == "review"
    ));

    // 3. Operator-configured local stub bound to `plan` -> LocalStubRejected
    let (_tmp_local, svc_local_stub) = service_with_toml(
        r#"
[health_check]
enabled = false

[[agents]]
id = "local-stub-planner"
name = "Local Stub Planner"
provider = "local"
endpoint = "local://stub"
model = "stub-v1"
ratings = ["general", "teen", "mature", "explicit"]

[[routing]]
route = "plan"
agent = "local-stub-planner"
"#,
    )
    .await;
    let s_stub = svc_local_stub
        .begin_planning(BeginPlanningInput {
            premise: "Stub test premise".into(),
            scope: None,
        })
        .await
        .unwrap();
    let r3 = svc_local_stub
        .advance_planning(AdvancePlanningInput {
            session_id: s_stub.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(r3.outcome, PlanningSessionOutcome::NeedsRoute);
    assert!(matches!(
        r3.route_problem,
        Some(PlanningRouteProblem::LocalStubRejected {
            ref agent_name,
            ref adapter_kind,
            builtin_default: false,
            ..
        }) if agent_name == "local-stub-planner" && adapter_kind == "local"
    ));

    // 4. Missing API key env and RatingNotCovered on configured HTTP agent
    let (_tmp_key, svc_key) = service_with_toml(
        r#"
[health_check]
enabled = false

[[agents]]
id = "missing-key-planner"
name = "Missing Key Planner"
provider = "openai-compatible"
endpoint = "http://127.0.0.1:9/v1"
model = "planner-v1"
api_key_env = "SPINDLE_MISSING_PLANNING_KEY_ENV_TEST_ONLY"
ratings = ["general"]

[[routing]]
route = "plan"
agent = "missing-key-planner"
"#,
    )
    .await;
    let s_key = svc_key
        .begin_planning(BeginPlanningInput {
            premise: "Missing key premise".into(),
            scope: None,
        })
        .await
        .unwrap();
    let r4 = svc_key
        .advance_planning(AdvancePlanningInput {
            session_id: s_key.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(r4.outcome, PlanningSessionOutcome::NeedsRoute);
    assert!(matches!(
        r4.route_problem,
        Some(PlanningRouteProblem::MissingApiKey {
            ref env_var,
            ..
        }) if env_var == "SPINDLE_MISSING_PLANNING_KEY_ENV_TEST_ONLY"
    ));

    let (_tmp_rating, svc_rating) = service_with_toml(
        r#"
[health_check]
enabled = false

[[agents]]
id = "tame-planner"
name = "Tame Planner"
provider = "openai-compatible"
endpoint = "http://127.0.0.1:9/v1"
model = "planner-v1"
ratings = ["general", "teen"]

[[routing]]
route = "plan"
agent = "tame-planner"
"#,
    )
    .await;
    let s_rating = svc_rating
        .begin_planning(BeginPlanningInput {
            premise: "Explicit ceiling premise".into(),
            scope: None,
        })
        .await
        .unwrap();
    let r5 = svc_rating
        .advance_planning(AdvancePlanningInput {
            session_id: s_rating.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: Some(spindle_core::models::ContentRating::Explicit),
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(r5.outcome, PlanningSessionOutcome::NeedsRoute);
    assert!(matches!(
        r5.route_problem,
        Some(PlanningRouteProblem::RatingNotCovered {
            rating_ceiling: spindle_core::models::ContentRating::Explicit,
            ..
        })
    ));
}

#[tokio::test]
async fn route_generation_explicit_review_backend_opt_in_and_no_implicit_fallback() {
    use serde_json::json;
    let valid_contract = json!({
        "schema_version": 1,
        "level": "contract",
        "rationale": "Contract via explicit review backend.",
        "operations": [{
            "output_ref": "contract:project",
            "operation": {
                "operation": "create_project",
                "input": {
                    "name": "Review Backend Novel",
                    "project_type": "novel",
                    "genre": "drama",
                    "reader_contract": {
                        "promise": "Character study",
                        "style_notes": [],
                        "boundaries": []
                    }
                }
            }
        }]
    });
    let (endpoint, _reqs, handle) = spawn_planning_http_server(vec![(
        200,
        json!({
            "choices": [{"message": {"content": valid_contract.to_string()}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 50, "completion_tokens": 40}
        }),
    )]);

    let (_tmp, svc) = service_with_toml(&format!(
        r#"
[health_check]
enabled = false

[[agents]]
id = "real-review-agent"
name = "Real Review Agent"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "reviewer-pro"
ratings = ["general", "teen", "mature"]

[[routing]]
route = "review"
agent = "real-review-agent"
"#
    ))
    .await;

    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "An archivist reconstructs a lost treaty.".into(),
            scope: None,
        })
        .await
        .unwrap();

    // Without explicit opt-in, `review` is NOT used for `plan`.
    let denied = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: Some(false),
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(denied.outcome, PlanningSessionOutcome::NeedsRoute);
    assert!(denied.proposal.is_none());

    // With explicit `allow_review_backend: Some(true)`, the configured real `review` agent serves.
    let allowed = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: Some(true),
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(allowed.outcome, PlanningSessionOutcome::Staged);
    let receipt = allowed.generation_receipt.expect("receipt");
    assert_eq!(receipt.route_task.as_deref(), Some("plan"));
    assert_eq!(receipt.resolved_route.as_deref(), Some("review"));
    assert_eq!(receipt.agent_name.as_deref(), Some("real-review-agent"));
    assert_eq!(handle.join().unwrap(), 1);
}

#[tokio::test]
async fn route_generation_repairs_once_with_json_pointer_findings_and_rejects_second_invalid_output()
 {
    use serde_json::json;
    // 1. First response has an unresolved proposal_ref; repair response fixes it.
    let bad_seed = json!({
        "schema_version": 1,
        "level": "cast_world",
        "rationale": "Introduce cast with broken relationship ref.",
        "operations": [
            {
                "output_ref": "cast:ada",
                "operation": {
                    "operation": "create_character",
                    "input": {"name": "Ada", "summary": "Seeker", "role": "lead", "realm": null}
                }
            },
            {
                "output_ref": "rel:broken",
                "operation": {
                    "operation": "create_relationship",
                    "input": {
                        "character_a_id": {"kind": "proposal_ref", "value": "cast:ada"},
                        "character_b_id": {"kind": "proposal_ref", "value": "cast:missing_partner"},
                        "relationship_type": "ally",
                        "initial_trust": 50,
                        "initial_tension": 10
                    }
                }
            }
        ]
    });
    let repaired_seed = json!({
        "schema_version": 1,
        "level": "cast_world",
        "rationale": "Introduce both characters before linking relationship.",
        "operations": [
            {
                "output_ref": "cast:ada",
                "operation": {
                    "operation": "create_character",
                    "input": {"name": "Ada", "summary": "Seeker", "role": "lead", "realm": null}
                }
            },
            {
                "output_ref": "cast:bram",
                "operation": {
                    "operation": "create_character",
                    "input": {"name": "Bram", "summary": "Guide", "role": "support", "realm": null}
                }
            },
            {
                "output_ref": "rel:fixed",
                "operation": {
                    "operation": "create_relationship",
                    "input": {
                        "character_a_id": {"kind": "proposal_ref", "value": "cast:ada"},
                        "character_b_id": {"kind": "proposal_ref", "value": "cast:bram"},
                        "relationship_type": "ally",
                        "initial_trust": 50,
                        "initial_tension": 10
                    }
                }
            }
        ]
    });
    // 2. For SeriesArc: both initial and repair responses are invalid (3rd response must NEVER be requested).
    let invalid_arc_1 = "not valid json at all";
    let invalid_arc_2 = json!({
        "schema_version": 1,
        "level": "series_arc",
        "rationale": "Asserts a realized milestone in initial planning",
        "operations": [{
            "output_ref": "arc:illegal",
            "operation": {
                "operation": "create_character_arc",
                "input": {
                    "character_id": {"kind": "proposal_ref", "value": "cast:ada"},
                    "arc_type": "growth",
                    "starting_state": "Doubt",
                    "ending_state": "Conviction",
                    "milestones": [{
                        "label": "Already happened",
                        "placement": {"book_number": 1, "chapter_number": 1, "scene_order": 1, "note": null},
                        "description": "Illegally marked reached",
                        "unlocks": [],
                        "reached_at": {"book_number": 1, "chapter_number": 1, "scene_order": 1, "note": null}
                    }],
                    "thematic_purpose": "Trust"
                }
            }
        }]
    });

    let (endpoint, captured_reqs, handle) = spawn_planning_http_server(vec![
        (
            200,
            json!({"choices": [{"message": {"content": bad_seed.to_string()}}], "usage": {"prompt_tokens": 100, "completion_tokens": 60}}),
        ),
        (
            200,
            json!({"choices": [{"message": {"content": repaired_seed.to_string()}}], "usage": {"prompt_tokens": 140, "completion_tokens": 70}}),
        ),
        (
            200,
            json!({"choices": [{"message": {"content": invalid_arc_1}}], "usage": {"prompt_tokens": 100, "completion_tokens": 20}}),
        ),
        (
            200,
            json!({"choices": [{"message": {"content": invalid_arc_2.to_string()}}], "usage": {"prompt_tokens": 140, "completion_tokens": 60}}),
        ),
    ]);

    let (_tmp, svc) = service_with_toml(&format!(
        r#"
[health_check]
enabled = false

[[agents]]
id = "repair-test-agent"
name = "Repair Test Agent"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "planner-v1"
ratings = ["general", "teen", "mature"]

[[routing]]
route = "plan"
agent = "repair-test-agent"
"#
    ))
    .await;

    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Repair loop test".into(),
            scope: None,
        })
        .await
        .unwrap();

    // Advance CastWorld: Call 1 fails -> Call 2 repairs and stages.
    let seed_report = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::CastWorld),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(seed_report.outcome, PlanningSessionOutcome::Staged);
    let seed_receipt = seed_report.generation_receipt.expect("seed receipt");
    assert_eq!(seed_receipt.attempts_used, 2);
    assert!(seed_receipt.repaired);
    assert_eq!(seed_receipt.prompt_id, PlanningPromptId::PlanningRepairV1);
    assert_eq!(seed_receipt.calls.len(), 2);
    assert_eq!(
        seed_receipt.calls[0].prompt_id,
        PlanningPromptId::PlanningSeedV1
    );
    assert_eq!(
        seed_receipt.calls[1].prompt_id,
        PlanningPromptId::PlanningRepairV1
    );

    // Verify the repair prompt sent to the model carried the exact JSON pointer.
    {
        let reqs = captured_reqs.lock().unwrap();
        assert_eq!(reqs.len(), 2);
        assert!(
            reqs[1].contains("planning_repair_v1")
                && reqs[1].contains("/operations/1/operation/input/character_b_id")
                && reqs[1].contains("unresolved_proposal_ref"),
            "repair prompt must include exact RFC 6901 JSON-pointer finding: {}",
            reqs[1]
        );
    }

    // Advance SeriesArc: Call 1 fails (malformed JSON) -> Call 2 fails (realized_event_in_initial_plan) -> stops at 2 attempts!
    let arc_report = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::SeriesArc),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(
        arc_report.outcome,
        PlanningSessionOutcome::ModelOutputRejected
    );
    assert!(
        arc_report.proposal.is_none(),
        "rejected model output must never be staged"
    );
    let arc_receipt = arc_report.generation_receipt.expect("arc receipt");
    assert_eq!(arc_receipt.attempts_used, 2);
    assert!(!arc_receipt.repaired);
    assert_eq!(arc_receipt.calls.len(), 2);
    assert!(
        arc_report
            .findings
            .iter()
            .any(|f| f.code == "realized_event_in_initial_plan"
                && f.path == "/operations/0/operation")
    );
    let parked_host = arc_report
        .host_request
        .expect("level parked for host after 2nd invalid output");
    assert_eq!(parked_host.prior_findings.len(), 1);
    assert_eq!(handle.join().unwrap(), 4);

    // Only the repaired CastWorld proposal was staged (total 1 proposal in session).
    let proposals = svc.list_planning_proposals(&s.id, None).await.unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].artifact.level, PlanningLevel::CastWorld);
}

#[tokio::test]
async fn route_generation_host_and_fixture_submissions_use_same_validator_and_label_source() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Host fallback & fixture labeling test.".into(),
            scope: None,
        })
        .await
        .unwrap();

    // Claiming source="model" on a host submission is rejected.
    let spoofed = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: Some(PlanningHostSubmission {
                source: PlanningOutputSource::Model,
                payload: json!({}),
            }),
            retry_key: None,
        })
        .await
        .unwrap_err();
    assert!(spoofed.to_string().contains("never 'model'"));

    // Invalid host submission fails closed through the same validator.
    let invalid_host = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: Some(PlanningHostSubmission {
                source: PlanningOutputSource::Host,
                payload: json!({
                    "schema_version": 1,
                    "level": "contract",
                    "rationale": "Wrong operation for level",
                    "operations": [{
                        "output_ref": "book:wrong",
                        "operation": {"operation": "create_book", "input": {"title": "Wrong"}}
                    }]
                }),
            }),
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(
        invalid_host.outcome,
        PlanningSessionOutcome::ModelOutputRejected
    );
    assert!(invalid_host.proposal.is_none());
    assert!(
        invalid_host
            .findings
            .iter()
            .any(|f| f.code == "operation_not_allowed_for_level")
    );

    // Valid host submission stages with source="host".
    let valid_host = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: Some(PlanningHostSubmission {
                source: PlanningOutputSource::Host,
                payload: json!({
                    "schema_version": 1,
                    "level": "contract",
                    "rationale": "Host-supplied contract",
                    "operations": [{
                        "output_ref": "contract:project",
                        "operation": {
                            "operation": "create_project",
                            "input": {
                                "name": "Host Project",
                                "project_type": "novel",
                                "genre": "sci-fi",
                                "reader_contract": {"promise": "First contact", "style_notes": [], "boundaries": []}
                            }
                        }
                    }]
                }),
            }),
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(valid_host.outcome, PlanningSessionOutcome::Staged);
    assert_eq!(
        valid_host.generation_receipt.as_ref().unwrap().source,
        PlanningOutputSource::Host
    );

    // Valid fixture submission with author_questions stages with source="fixture" and outcome="needs_author".
    let valid_fixture = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::CastWorld),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: Some(PlanningHostSubmission {
                source: PlanningOutputSource::Fixture,
                payload: json!({
                    "schema_version": 1,
                    "level": "cast_world",
                    "rationale": "Deterministic fixture cast",
                    "author_questions": [{
                        "id": "q-antagonist",
                        "question": "Should the envoy be sympathetic or implacable?",
                        "choices": ["Sympathetic", "Implacable"],
                        "recommended": "Sympathetic"
                    }],
                    "operations": [{
                        "output_ref": "cast:envoy",
                        "operation": {
                            "operation": "create_character",
                            "input": {"name": "Vael", "summary": "Envoy", "role": "protagonist", "realm": null}
                        }
                    }]
                }),
            }),
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(valid_fixture.outcome, PlanningSessionOutcome::NeedsAuthor);
    assert_eq!(valid_fixture.author_questions.len(), 1);
    let fixture_prop = valid_fixture.proposal.expect("staged fixture proposal");
    assert_eq!(
        fixture_prop.generation_receipt.as_ref().unwrap().source,
        PlanningOutputSource::Fixture
    );
}

#[tokio::test]
async fn route_generation_enforces_token_call_and_cost_ceilings_before_dispatch_and_never_persists_secrets()
 {
    use serde_json::json;
    let secret_value = "sk-super-secret-planning-token-never-persist";
    let env_name = "SPINDLE_PLANNING_SECRET_TEST_TOKEN_V1";
    // SAFETY: test-scoped unique env var name not read concurrently elsewhere.
    unsafe {
        std::env::set_var(env_name, secret_value);
    }

    let valid_contract = json!({
        "schema_version": 1,
        "level": "contract",
        "rationale": "Contract within budget.",
        "operations": [{
            "output_ref": "contract:project",
            "operation": {
                "operation": "create_project",
                "input": {
                    "name": "Budgeted Novel",
                    "project_type": "novel",
                    "genre": "mystery",
                    "reader_contract": {"promise": "Tightly budgeted", "style_notes": [], "boundaries": []}
                }
            }
        }]
    });
    // Response 1: valid contract WITH provider usage.
    // Response 2: valid cast_world WITHOUT provider usage (`usage` omitted).
    // Response 3: invalid series_arc when max_model_calls is reached before repair.
    let valid_seed = json!({
        "schema_version": 1,
        "level": "cast_world",
        "rationale": "Seed without provider usage.",
        "operations": [{
            "output_ref": "cast:hero",
            "operation": {
                "operation": "create_character",
                "input": {"name": "Iris", "summary": "Investigator", "role": "protagonist", "realm": null}
            }
        }]
    });

    let (endpoint, _reqs, handle) = spawn_planning_http_server(vec![
        (
            200,
            json!({
                "choices": [{"message": {"content": valid_contract.to_string()}}],
                "usage": {"prompt_tokens": 200, "completion_tokens": 100}
            }),
        ),
        (
            200,
            json!({
                "choices": [{"message": {"content": valid_seed.to_string()}}]
            }),
        ),
        (
            200,
            json!({
                "choices": [{"message": {"content": "invalid json requiring repair"}}],
                "usage": {"prompt_tokens": 100, "completion_tokens": 50}
            }),
        ),
    ]);

    let (_tmp, svc) = service_with_toml(&format!(
        r#"
[health_check]
enabled = false

[[agents]]
id = "budget-agent"
name = "Budget Agent"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "budget-v1"
api_key_env = "{env_name}"
ratings = ["general", "teen", "mature"]

[[routing]]
route = "plan"
agent = "budget-agent"
"#
    ))
    .await;

    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Budget and secret-hygiene verification".into(),
            scope: None,
        })
        .await
        .unwrap();

    // 1. Pre-dispatch output token ceiling check: max_output_tokens (500) < max_output_tokens_per_call (2000)
    let out_exhausted = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: Some(PlanningBudget {
                max_output_tokens: 500,
                max_output_tokens_per_call: 2000,
                ..PlanningBudget::default()
            }),
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(
        out_exhausted.outcome,
        PlanningSessionOutcome::BudgetExhausted
    );
    assert_eq!(out_exhausted.usage.model_calls, 0);

    // 2. Pre-dispatch USD cost ceiling check: max_cost_usd too small to cover call reserve
    let cost_exhausted = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: Some(PlanningBudget {
                max_cost_usd: Some(0.0001),
                input_cost_per_million_usd: Some(500.0),
                output_cost_per_million_usd: Some(1500.0),
                ..PlanningBudget::default()
            }),
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(
        cost_exhausted.outcome,
        PlanningSessionOutcome::BudgetExhausted
    );
    assert_eq!(cost_exhausted.usage.model_calls, 0);

    // 3. Call 1 with external prices configured -> EstimatedFromConfiguredPrices
    let priced_budget = PlanningBudget {
        max_model_calls: 3,
        max_input_tokens: 48_000,
        max_output_tokens: 24_000,
        max_total_tokens: 72_000,
        max_output_tokens_per_call: 2_000,
        max_cost_usd: Some(10.0),
        input_cost_per_million_usd: Some(10.0),
        output_cost_per_million_usd: Some(30.0),
    };
    let c1 = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: Some(priced_budget.clone()),
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(c1.outcome, PlanningSessionOutcome::Staged);
    let call_1_receipt = &c1.generation_receipt.as_ref().unwrap().calls[0];
    assert!(call_1_receipt.provider_reported_usage);
    assert_eq!(
        call_1_receipt.cost_status,
        PlanningCostStatus::EstimatedFromConfiguredPrices
    );
    assert!(call_1_receipt.estimated_cost_usd.unwrap() > 0.0);

    // 4. Call 2 omits provider usage -> UnknownMissingProviderUsage and None cost
    let unpriced_budget = PlanningBudget {
        max_model_calls: 3,
        ..PlanningBudget::default()
    };
    let c2 = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::CastWorld),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: Some(unpriced_budget),
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(c2.outcome, PlanningSessionOutcome::Staged);
    let call_2_receipt = &c2.generation_receipt.as_ref().unwrap().calls[0];
    assert!(!call_2_receipt.provider_reported_usage);
    assert_eq!(call_2_receipt.estimated_cost_usd, None);
    assert_eq!(
        call_2_receipt.cost_status,
        PlanningCostStatus::UnknownMissingProviderUsage
    );

    // 5. Call 3 returns invalid JSON, and max_model_calls=3 prevents repair Call 4 from dispatching!
    let c3 = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::SeriesArc),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(c3.outcome, PlanningSessionOutcome::BudgetExhausted);
    assert_eq!(c3.usage.model_calls, 3);
    assert_eq!(c3.generation_receipt.as_ref().unwrap().attempts_used, 1);
    assert_eq!(handle.join().unwrap(), 3);

    // Verify secret value never appears anywhere in persisted SQLite rows.
    let dump: Vec<String> = svc
        .repository
        .pool()
        .read(move |conn| {
            let mut out = Vec::new();
            for table in [
                "planning_session",
                "planning_proposal",
                "planning_generation_attempt",
            ] {
                let mut stmt = conn.prepare(&format!("SELECT data FROM {table}"))?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                out.extend(rows);
            }
            let mut stmt = conn.prepare("SELECT payload FROM model_call")?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.extend(rows);
            Ok(Ok::<_, anyhow::Error>(out))
        })
        .await
        .unwrap()
        .unwrap();
    assert!(!dump.is_empty());
    for row in &dump {
        assert!(
            !row.contains(secret_value),
            "persisted planning/telemetry row leaked secret credential"
        );
    }
}

#[tokio::test]
async fn route_generation_retry_key_is_idempotent_and_v0050_database_upgrades_cleanly() {
    use serde_json::json;
    // Part 1: V0051 database upgrades cleanly to V0052 and preserves Phase A/B proposal hashes.
    let tmp_upg = tempfile::TempDir::new().unwrap();
    let db_path = tmp_upg.path().join("upgrade_v0050.db");
    {
        let mut conn = rusqlite::Connection::open(&db_path).unwrap();
        crate::sqlite::migrations::runner()
            .set_target(refinery::Target::Version(51))
            .run(&mut conn)
            .unwrap();
    }
    let pool = crate::sqlite::SqlitePool::open(&db_path).await.unwrap();
    let valid_contract = json!({
        "schema_version": 1,
        "level": "contract",
        "rationale": "Idempotent contract generation.",
        "operations": [{
            "output_ref": "contract:project",
            "operation": {
                "operation": "create_project",
                "input": {
                    "name": "Idempotent Harbor",
                    "project_type": "novel",
                    "genre": "mystery",
                    "reader_contract": {"promise": "Idempotent", "style_notes": [], "boundaries": []}
                }
            }
        }]
    });
    let (endpoint, _reqs, handle) = spawn_planning_http_server(vec![(
        200,
        json!({
            "choices": [{"message": {"content": valid_contract.to_string()}}],
            "usage": {"prompt_tokens": 90, "completion_tokens": 50}
        }),
    )]);
    let config_path = tmp_upg.path().join("spindle.toml");
    std::fs::write(
        &config_path,
        format!(
            r#"
[health_check]
enabled = false

[[agents]]
id = "idem-agent"
name = "Idem Agent"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "idem-v1"
ratings = ["general", "teen", "mature"]

[[routing]]
route = "plan"
agent = "idem-agent"
"#
        ),
    )
    .unwrap();
    let router = crate::ai::ModelRouter::local_only();
    router
        .configure(Some(&config_path.display().to_string()))
        .unwrap();
    let repo = Repository::with_model_router(pool, tmp_upg.path().join("data"), router);
    let svc = SqliteSpindleService::new(repo);

    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Retry key idempotency".into(),
            scope: None,
        })
        .await
        .unwrap();

    let first = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: Some("retry-contract-001".into()),
        })
        .await
        .unwrap();
    let second = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: Some("retry-contract-001".into()),
        })
        .await
        .unwrap();
    assert_eq!(first.attempt_id, second.attempt_id);
    assert_eq!(
        first.proposal.as_ref().unwrap().id,
        second.proposal.as_ref().unwrap().id
    );
    assert_eq!(handle.join().unwrap(), 1);
    assert_eq!(
        svc.list_planning_proposals(&s.id, None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        svc.list_planning_generation_attempts(&s.id, None)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn dependent_prompts_include_parent_operation_inputs_and_accepted_constraints() {
    use serde_json::json;
    let (_tmp, svc) = fresh_service_local().await;
    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Harbor mystery".into(),
            scope: None,
        })
        .await
        .unwrap();

    // Stage a Contract proposal where the fair-play alibi rule is ONLY inside the operation input,
    // not repeated in `rationale`.
    let contract_only_decision = "The lighthouse keeper's alibi is verified by the brass tide-clock and never broken by supernatural means.";
    let contract_prop = svc
        .stage_planning(StagePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            intent_hash: s.intent_hash.clone(),
            schema_version: 1,
            level: PlanningLevel::Contract,
            parents: BTreeMap::new(),
            operations: vec![PlanningOperationItem {
                output_ref: "contract:project".into(),
                operation: PlanningOperation::CreateProject(PlanningCreateProjectInput {
                    name: "Binnacle Watch".into(),
                    project_type: "novel".into(),
                    genre: "mystery".into(),
                    reader_contract: PlanningReaderContract {
                        promise: contract_only_decision.into(),
                        style_notes: vec!["maritime precision".into()],
                        boundaries: vec!["no supernatural deus ex machina".into()],
                    },
                }),
            }],
            rationale: "Initial contract.".into(),
            target_range: None,
            schedule: PlanningSchedule::default(),
            assumptions: Vec::new(),
            author_questions: Vec::new(),
        })
        .await
        .unwrap();

    // Stage a Chapter proposal depending on Contract where the chapter beat detail is ONLY inside
    // `SetChapterOutline` operation input, not in `rationale`.
    let chapter_only_decision =
        "Mara discovers the salt-stained ledger hidden inside the hollow binnacle.";
    let chapter_beat_only =
        "Comparing the harbourmaster signatures proves the manifest was forged at low tide.";
    let chapter_prop = svc
        .stage_planning(StagePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            intent_hash: s.intent_hash.clone(),
            schema_version: 1,
            level: PlanningLevel::Chapter,
            parents: BTreeMap::from([(
                contract_prop.id.clone(),
                contract_prop.payload_hash.clone(),
            )]),
            operations: vec![PlanningOperationItem {
                output_ref: "chapter:1:outline".into(),
                operation: PlanningOperation::SetChapterOutline(PlanningSetChapterOutlineInput {
                    book_number: 1,
                    chapter_number: 1,
                    entity_id: Some(PlanningRef::ProposalRef("contract:project:chapter".into())),
                    entity_type: Some("chapter".into()),
                    format: Some("markdown".into()),
                    content: Some(chapter_only_decision.into()),
                    beats: vec![PlanningChapterOutlineBeat {
                        order: 1,
                        summary: chapter_beat_only.into(),
                        scene_id: None,
                        status: "planned".into(),
                    }],
                }),
            }],
            rationale: "Chapter 1 outline.".into(),
            target_range: None,
            schedule: PlanningSchedule::default(),
            assumptions: Vec::new(),
            author_questions: Vec::new(),
        })
        .await
        .unwrap();

    // Request Scene generation passing only the immediate Chapter parent in `parents`.
    // Both the Chapter operation inputs AND the transitive ancestor Contract operation inputs
    // must be serialized into the prompt.
    let report = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Scene),
            parents: Some(BTreeMap::from([(
                chapter_prop.id.clone(),
                chapter_prop.payload_hash.clone(),
            )])),
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(report.outcome, PlanningSessionOutcome::NeedsRoute);
    let host_prompt = &report.host_request.as_ref().unwrap().user_prompt;
    assert!(
        host_prompt.contains(chapter_only_decision),
        "dependent prompt must include parent SetChapterOutline content even when absent from rationale"
    );
    assert!(
        host_prompt.contains(chapter_beat_only),
        "dependent prompt must include parent SetChapterOutline beats even when absent from rationale"
    );
    assert!(
        host_prompt.contains(contract_only_decision),
        "dependent prompt must include transitive parent reader_contract promise even when absent from rationale"
    );
    assert!(
        host_prompt.contains("no supernatural deus ex machina"),
        "dependent prompt must include transitive parent reader_contract boundaries"
    );

    // Apply the root Contract so the session attaches to the real project/branch, and verify
    // `scoped_existing_record_digest` also serializes the accepted project reader_contract.
    let applied = svc
        .decide_planning(DecidePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            expected_hashes: BTreeMap::from([(
                contract_prop.id.clone(),
                contract_prop.payload_hash.clone(),
            )]),
            decision: PlanningDecision::Accept,
            actor: "author:test".into(),
            note: "Accept root contract".into(),
        })
        .await
        .unwrap();
    let attached_scope = svc.get_planning_session(&s.id).await.unwrap().scope;
    assert!(attached_scope.is_some());
    let digest = svc
        .repository
        .describe_planning_scope_digest(attached_scope)
        .await
        .unwrap()
        .join("\n");
    assert!(
        digest.contains(contract_only_decision),
        "scoped existing-record digest must include accepted reader_contract from project table; got: {digest}"
    );
    let _ = (applied, json!({}));
}

#[tokio::test]
async fn missing_provider_usage_preserves_conservative_usd_spend_across_repair_and_subsequent_calls()
 {
    use serde_json::json;
    let valid_contract = json!({
        "schema_version": 1,
        "level": "contract",
        "rationale": "Establish mystery contract.",
        "operations": [{
            "output_ref": "contract:project",
            "operation": {
                "operation": "create_project",
                "input": {
                    "name": "Bound Harbor",
                    "project_type": "novel",
                    "genre": "mystery",
                    "reader_contract": {"promise": "Fair clues", "style_notes": [], "boundaries": []}
                }
            }
        }]
    });
    let valid_cast = json!({
        "schema_version": 1,
        "level": "cast_world",
        "rationale": "Introduce protagonist.",
        "operations": [{
            "output_ref": "cast:mara",
            "operation": {
                "operation": "create_character",
                "input": {
                    "name": "Mara",
                    "summary": "Investigator",
                    "role": "protagonist",
                    "realm": null
                }
            }
        }]
    });

    // Response 1 (for session A): invalid JSON AND omits provider `usage`!
    // Response 2 (for session B call 1): valid Contract WITH provider `usage`.
    // Response 3 (for session B call 2): valid CastWorld WITHOUT provider `usage`.
    let (endpoint, _reqs, handle) = spawn_planning_http_server(vec![
        (
            200,
            json!({
                "choices": [{"message": {"content": "not valid json"}}],
            }),
        ),
        (
            200,
            json!({
                "choices": [{"message": {"content": valid_contract.to_string()}}],
                "usage": {"prompt_tokens": 1_000, "completion_tokens": 500}
            }),
        ),
        (
            200,
            json!({
                "choices": [{"message": {"content": valid_cast.to_string()}}],
            }),
        ),
    ]);
    let (_tmp, svc) = service_with_toml(&format!(
        r#"
[health_check]
enabled = false

[[agents]]
id = "tight-usd-agent"
name = "Tight USD Agent"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "tight-usd-v1"
ratings = ["general", "teen", "mature"]

[[routing]]
route = "plan"
agent = "tight-usd-agent"
"#
    ))
    .await;

    // Part A: Repair path under a tight USD ceiling when Call 1 omits provider usage.
    // With input_cost = $10/1M, output_cost = $1000/1M, max_output_tokens_per_call = 1000:
    // Each call's output reserve alone is $1.00 (plus ~$0.015 input = ~$1.015).
    // Setting max_cost_usd = $1.50 allows Call 1 (~$1.015 <= $1.50),
    // and even though Call 1 omits provider usage, its conservative reserved spend (~$1.015)
    // is preserved so the repair Call 2 (~$1.015 + ~$1.015 > $1.50) is blocked before dispatch!
    let s_repair = svc
        .begin_planning(BeginPlanningInput {
            premise: "Tight USD repair ceiling with missing usage".into(),
            scope: None,
        })
        .await
        .unwrap();
    let tight_repair_budget = PlanningBudget {
        max_model_calls: 6,
        max_input_tokens: 48_000,
        max_output_tokens: 24_000,
        max_total_tokens: 72_000,
        max_output_tokens_per_call: 1_000,
        max_cost_usd: Some(1.50),
        input_cost_per_million_usd: Some(10.0),
        output_cost_per_million_usd: Some(1_000.0),
    };
    let repair_blocked = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s_repair.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: Some(tight_repair_budget),
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(
        repair_blocked.outcome,
        PlanningSessionOutcome::BudgetExhausted,
        "repair call must be blocked by conservative reserved spend when Call 1 omits usage"
    );
    assert_eq!(repair_blocked.usage.model_calls, 1);
    assert_eq!(repair_blocked.usage.estimated_cost_usd, None);
    assert_eq!(
        repair_blocked.usage.cost_status,
        PlanningCostStatus::UnknownMissingProviderUsage
    );
    assert!(
        repair_blocked.usage.reserved_cost_usd.unwrap() >= 1.0,
        "conservative reserved spend must be preserved even when estimated_cost_usd is None"
    );
    assert_eq!(
        repair_blocked
            .generation_receipt
            .as_ref()
            .unwrap()
            .attempts_used,
        1
    );

    // Part B: Multi-call across advance_planning calls where Call 1 reports usage ($0.51)
    // and Call 2 omits usage (conservative reserve >= $1.00), so Call 3 is blocked under max_cost_usd = $2.20!
    let s_multi = svc
        .begin_planning(BeginPlanningInput {
            premise: "Multi-call tight USD ceiling with mixed usage reporting".into(),
            scope: None,
        })
        .await
        .unwrap();
    let multi_budget = PlanningBudget {
        max_model_calls: 6,
        max_input_tokens: 48_000,
        max_output_tokens: 24_000,
        max_total_tokens: 72_000,
        max_output_tokens_per_call: 1_000,
        max_cost_usd: Some(2.20),
        input_cost_per_million_usd: Some(10.0),
        output_cost_per_million_usd: Some(1_000.0),
    };
    let m1 = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s_multi.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: Some(multi_budget.clone()),
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(m1.outcome, PlanningSessionOutcome::Staged);
    let known_spend_1 = m1.usage.estimated_cost_usd.unwrap();
    assert!(known_spend_1 > 0.50);

    let m2 = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s_multi.id.clone(),
            scope: None,
            level: Some(PlanningLevel::CastWorld),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: Some(multi_budget.clone()),
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(m2.outcome, PlanningSessionOutcome::Staged);
    assert_eq!(m2.usage.estimated_cost_usd, None);
    assert_eq!(
        m2.usage.cost_status,
        PlanningCostStatus::UnknownMissingProviderUsage
    );
    assert!(
        m2.usage.reserved_cost_usd.unwrap() >= known_spend_1 + 1.0,
        "Call 2 omitting usage must add conservative spend on top of Call 1's known spend"
    );

    // Call 3 must be rejected before network I/O because reserved_cost_usd (~$1.52) + Call 3 reserve (~$1.02) > $2.20!
    let m3 = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s_multi.id.clone(),
            scope: None,
            level: Some(PlanningLevel::SeriesArc),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: Some(multi_budget),
            host_submission: None,
            retry_key: None,
        })
        .await
        .unwrap();
    assert_eq!(m3.outcome, PlanningSessionOutcome::BudgetExhausted);
    assert_eq!(m3.usage.model_calls, 2);
    assert_eq!(handle.join().unwrap(), 3);
}

#[tokio::test]
async fn simultaneous_advances_reserve_budget_atomically_and_simultaneous_retry_keys_deduplicate() {
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let valid_contract = json!({
        "schema_version": 1,
        "level": "contract",
        "rationale": "Concurrent contract generation.",
        "operations": [{
            "output_ref": "contract:project",
            "operation": {
                "operation": "create_project",
                "input": {
                    "name": "Concurrent Harbor",
                    "project_type": "novel",
                    "genre": "mystery",
                    "reader_contract": {"promise": "Atomic budget", "style_notes": [], "boundaries": []}
                }
            }
        }]
    });
    let valid_cast = json!({
        "schema_version": 1,
        "level": "cast_world",
        "rationale": "Concurrent cast generation.",
        "operations": [{
            "output_ref": "cast:ivo",
            "operation": {
                "operation": "create_character",
                "input": {
                    "name": "Ivo",
                    "summary": "Harbourmaster",
                    "role": "supporting",
                    "realm": null
                }
            }
        }]
    });

    // Spawn a multi-connection HTTP server that adds a small delay on each request so concurrent
    // advances overlap in flight while talking to the model backend.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let call_counter = Arc::new(AtomicUsize::new(0));
    let call_counter_srv = call_counter.clone();
    let body_contract = json!({
        "choices": [{"message": {"content": valid_contract.to_string()}}],
        "usage": {"prompt_tokens": 110, "completion_tokens": 40}
    })
    .to_string();
    let body_cast = json!({
        "choices": [{"message": {"content": valid_cast.to_string()}}],
        "usage": {"prompt_tokens": 130, "completion_tokens": 60}
    })
    .to_string();
    let srv_handle = std::thread::spawn(move || {
        let mut worker_threads = Vec::new();
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            let counter = call_counter_srv.clone();
            let contract_reply = body_contract.clone();
            let cast_reply = body_cast.clone();
            worker_threads.push(std::thread::spawn(move || {
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .ok();
                let mut buf = [0u8; 65536];
                let mut total = 0usize;
                loop {
                    match stream.read(&mut buf[total..]) {
                        Ok(0) => break,
                        Ok(n) => {
                            total += n;
                            let s = String::from_utf8_lossy(&buf[..total]);
                            if let Some(hdr_end) = s.find("\r\n\r\n") {
                                let hdrs = &s[..hdr_end];
                                let content_len = hdrs
                                    .lines()
                                    .find_map(|l| {
                                        let lower = l.to_ascii_lowercase();
                                        lower
                                            .strip_prefix("content-length:")
                                            .and_then(|v| v.trim().parse::<usize>().ok())
                                    })
                                    .unwrap_or(0);
                                if total >= hdr_end + 4 + content_len {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                let req_str = String::from_utf8_lossy(&buf[..total]);
                let body = if req_str.contains("planning_seed_v1") {
                    cast_reply
                } else {
                    contract_reply
                };
                counter.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(60));
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.flush();
            }));
        }
        for w in worker_threads {
            w.join().unwrap();
        }
    });

    let endpoint = format!("http://{addr}/v1");
    let (_tmp, svc) = service_with_toml(&format!(
        r#"
[health_check]
enabled = false

[[agents]]
id = "concurrent-agent"
name = "Concurrent Agent"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "concurrent-v1"
ratings = ["general", "teen", "mature"]

[[routing]]
route = "plan"
agent = "concurrent-agent"
"#
    ))
    .await;

    // 1. Simultaneous identical retry_keys: only 1 model call is dispatched, both return the same attempt.
    let s_retry = svc
        .begin_planning(BeginPlanningInput {
            premise: "Simultaneous retry key deduplication".into(),
            scope: None,
        })
        .await
        .unwrap();
    let req_a = AdvancePlanningInput {
        session_id: s_retry.id.clone(),
        scope: None,
        level: Some(PlanningLevel::Contract),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: None,
        host_submission: None,
        retry_key: Some("sim-retry-001".into()),
    };
    let req_b = req_a.clone();
    let (res_a, res_b) = tokio::join!(svc.advance_planning(req_a), svc.advance_planning(req_b));
    let (res_a, res_b) = (res_a.unwrap(), res_b.unwrap());
    assert_eq!(res_a.attempt_id, res_b.attempt_id);
    assert_eq!(
        res_a.proposal.as_ref().unwrap().id,
        res_b.proposal.as_ref().unwrap().id
    );
    assert_eq!(call_counter.load(Ordering::SeqCst), 1);
    assert_eq!(
        svc.get_planning_session(&s_retry.id)
            .await
            .unwrap()
            .usage
            .model_calls,
        1
    );

    // 2. Simultaneous advances with max_model_calls = 1: one succeeds, one hits BudgetExhausted before network I/O.
    let s_cap1 = svc
        .begin_planning(BeginPlanningInput {
            premise: "Simultaneous advances under max_model_calls=1".into(),
            scope: None,
        })
        .await
        .unwrap();
    let cap1_budget = PlanningBudget {
        max_model_calls: 1,
        ..PlanningBudget::default()
    };
    let c1_a = AdvancePlanningInput {
        session_id: s_cap1.id.clone(),
        scope: None,
        level: Some(PlanningLevel::Contract),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: Some(cap1_budget.clone()),
        host_submission: None,
        retry_key: None,
    };
    let c1_b = AdvancePlanningInput {
        session_id: s_cap1.id.clone(),
        scope: None,
        level: Some(PlanningLevel::CastWorld),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: Some(cap1_budget),
        host_submission: None,
        retry_key: None,
    };
    let (out_1a, out_1b) = tokio::join!(svc.advance_planning(c1_a), svc.advance_planning(c1_b));
    let (out_1a, out_1b) = (out_1a.unwrap(), out_1b.unwrap());
    let outcomes = [out_1a.outcome, out_1b.outcome];
    assert!(outcomes.contains(&PlanningSessionOutcome::Staged));
    assert!(outcomes.contains(&PlanningSessionOutcome::BudgetExhausted));
    assert_eq!(call_counter.load(Ordering::SeqCst), 2);
    assert_eq!(
        svc.get_planning_session(&s_cap1.id)
            .await
            .unwrap()
            .usage
            .model_calls,
        1
    );

    // 3. Simultaneous advances with max_model_calls = 2: both overlap in flight and reconcile usage without losing either call's tokens!
    let s_cap2 = svc
        .begin_planning(BeginPlanningInput {
            premise: "Simultaneous overlapping advances reconcile usage".into(),
            scope: None,
        })
        .await
        .unwrap();
    let cap2_budget = PlanningBudget {
        max_model_calls: 2,
        ..PlanningBudget::default()
    };
    let c2_a = AdvancePlanningInput {
        session_id: s_cap2.id.clone(),
        scope: None,
        level: Some(PlanningLevel::Contract),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: Some(cap2_budget.clone()),
        host_submission: None,
        retry_key: None,
    };
    let c2_b = AdvancePlanningInput {
        session_id: s_cap2.id.clone(),
        scope: None,
        level: Some(PlanningLevel::CastWorld),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: Some(cap2_budget),
        host_submission: None,
        retry_key: None,
    };
    let (out_2a, out_2b) = tokio::join!(svc.advance_planning(c2_a), svc.advance_planning(c2_b));
    let (out_2a, out_2b) = (out_2a.unwrap(), out_2b.unwrap());
    assert_eq!(out_2a.outcome, PlanningSessionOutcome::Staged);
    assert_eq!(out_2b.outcome, PlanningSessionOutcome::Staged);
    assert_eq!(call_counter.load(Ordering::SeqCst), 4);
    let final_session = svc.get_planning_session(&s_cap2.id).await.unwrap();
    assert_eq!(final_session.usage.model_calls, 2);
    assert_eq!(final_session.usage.input_tokens, 110 + 130);
    assert_eq!(final_session.usage.output_tokens, 40 + 60);
    assert_eq!(final_session.usage.total_tokens, 150 + 190);
    srv_handle.join().unwrap();
}

#[tokio::test]
async fn cancelled_and_reopened_in_progress_attempt_retains_reservation_and_recovers_on_same_retry_key()
 {
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let valid_contract = json!({
        "schema_version": 1,
        "level": "contract",
        "rationale": "Recovered contract after cancellation.",
        "operations": [{
            "output_ref": "contract:project",
            "operation": {
                "operation": "create_project",
                "input": {
                    "name": "Recovered Harbor",
                    "project_type": "novel",
                    "genre": "mystery",
                    "reader_contract": {"promise": "Durable recovery", "style_notes": [], "boundaries": []}
                }
            }
        }]
    });
    let valid_cast = json!({
        "schema_version": 1,
        "level": "cast_world",
        "rationale": "Recovered cast after process reopen.",
        "operations": [{
            "output_ref": "cast:tomas",
            "operation": {
                "operation": "create_character",
                "input": {
                    "name": "Tomas",
                    "summary": "Signalman",
                    "role": "supporting",
                    "realm": null
                }
            }
        }]
    });

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body_contract = json!({
        "choices": [{"message": {"content": valid_contract.to_string()}}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 45}
    })
    .to_string();
    let body_cast = json!({
        "choices": [{"message": {"content": valid_cast.to_string()}}],
        "usage": {"prompt_tokens": 120, "completion_tokens": 55}
    })
    .to_string();

    let srv_handle = std::thread::spawn(move || {
        // Request 1: slow (300ms) so the client future times out and is dropped mid-await.
        // Request 2: fast response for the in-process retry of "cancel-key-001".
        // Request 3: fast response for the post-reopen retry of "reopen-key-001".
        for (delay_ms, body) in [
            (300u64, body_contract.clone()),
            (0u64, body_contract),
            (0u64, body_cast),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .ok();
            let mut buf = [0u8; 65536];
            let mut total = 0usize;
            loop {
                match stream.read(&mut buf[total..]) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        let s = String::from_utf8_lossy(&buf[..total]);
                        if let Some(hdr_end) = s.find("\r\n\r\n") {
                            let hdrs = &s[..hdr_end];
                            let content_len = hdrs
                                .lines()
                                .find_map(|l| {
                                    let lower = l.to_ascii_lowercase();
                                    lower
                                        .strip_prefix("content-length:")
                                        .and_then(|v| v.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            if total >= hdr_end + 4 + content_len {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            if delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        }
    });

    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("planning_cancel_reopen.db");
    let config_path = tmp.path().join("spindle.toml");
    let endpoint = format!("http://{addr}/v1");
    std::fs::write(
        &config_path,
        format!(
            r#"
[health_check]
enabled = false

[[agents]]
id = "recover-agent"
name = "Recover Agent"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "recover-v1"
ratings = ["general", "teen", "mature"]

[[routing]]
route = "plan"
agent = "recover-agent"
"#
        ),
    )
    .unwrap();

    let pool = crate::sqlite::SqlitePool::open(&db_path).await.unwrap();
    let router = crate::ai::ModelRouter::local_only();
    router
        .configure(Some(&config_path.display().to_string()))
        .unwrap();
    let repo = Repository::with_model_router(pool, tmp.path().join("data"), router);
    let svc = SqliteSpindleService::new(repo);

    let s = svc
        .begin_planning(BeginPlanningInput {
            premise: "Cancel and reopen retry recovery".into(),
            scope: None,
        })
        .await
        .unwrap();

    // 1. Start advance_planning with "cancel-key-001" and cancel/drop the future mid-await!
    let req_cancel = AdvancePlanningInput {
        session_id: s.id.clone(),
        scope: None,
        level: Some(PlanningLevel::Contract),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: None,
        host_submission: None,
        retry_key: Some("cancel-key-001".into()),
    };
    let timed_out = tokio::time::timeout(
        std::time::Duration::from_millis(75),
        svc.advance_planning(req_cancel.clone()),
    )
    .await;
    assert!(
        timed_out.is_err(),
        "first call should time out while HTTP response is delayed"
    );

    // Uncertain paid-call reservation (model_calls == 1) must be retained on the session,
    // and the interrupted attempt must be visible via list/find APIs.
    let session_after_cancel = svc.get_planning_session(&s.id).await.unwrap();
    assert_eq!(session_after_cancel.usage.model_calls, 1);
    let attempts_after_cancel = svc
        .list_planning_generation_attempts(&s.id, None)
        .await
        .unwrap();
    assert_eq!(attempts_after_cancel.len(), 1);
    assert_eq!(attempts_after_cancel[0].usage.model_calls, 1);

    // 2. Retry the exact same retry_key ("cancel-key-001"): must recover and succeed (`Staged`)
    // while retaining the uncertain call reservation (`model_calls == 2`).
    let recovered = svc.advance_planning(req_cancel.clone()).await.unwrap();
    assert_eq!(recovered.outcome, PlanningSessionOutcome::Staged);
    assert_eq!(recovered.attempt_id, attempts_after_cancel[0].attempt_id);
    assert_eq!(recovered.usage.model_calls, 2);

    // Idempotent replay after terminal completion does not dispatch another call.
    let replayed = svc.advance_planning(req_cancel).await.unwrap();
    assert_eq!(replayed.attempt_id, recovered.attempt_id);
    assert_eq!(replayed.usage.model_calls, 2);

    // 3. Simulate process crash/reopen with an abandoned `'in_progress'` attempt for "reopen-key-001"
    // and an uncertain reserved call (`model_calls == 3`).
    drop(svc);
    let pool2 = crate::sqlite::SqlitePool::open(&db_path).await.unwrap();
    let router2 = crate::ai::ModelRouter::local_only();
    router2
        .configure(Some(&config_path.display().to_string()))
        .unwrap();
    let repo2 = Repository::with_model_router(pool2, tmp.path().join("data"), router2);
    let svc2 = SqliteSpindleService::new(repo2);

    let crashed_claim = svc2
        .repository
        .claim_planning_generation_attempt(&s.id, None, Some("reopen-key-001".into()))
        .await
        .unwrap();
    let (crashed_attempt_id, crashed_claim_token, crashed_guard) = match crashed_claim {
        crate::sqlite::repository::PlanningAttemptClaim::Claimed {
            attempt_id,
            claim_token,
            active_guard,
            ..
        } => (attempt_id, claim_token, active_guard),
        _ => panic!("expected fresh claim"),
    };
    svc2.repository
        .reserve_planning_model_call(
            &crashed_attempt_id,
            &crashed_claim_token,
            PlanningLevel::CastWorld,
            &s.id,
            None,
            PlanningBudget::default(),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    drop(crashed_guard);
    drop(svc2);

    // Reopen in a fresh service instance and retry "reopen-key-001": must recover and reach `Staged`
    // while retaining the crashed run's reserved call (`model_calls == 4`).
    let pool3 = crate::sqlite::SqlitePool::open(&db_path).await.unwrap();
    let router3 = crate::ai::ModelRouter::local_only();
    router3
        .configure(Some(&config_path.display().to_string()))
        .unwrap();
    let repo3 = Repository::with_model_router(pool3, tmp.path().join("data"), router3);
    let svc3 = SqliteSpindleService::new(repo3);

    let reopened_res = svc3
        .advance_planning(AdvancePlanningInput {
            session_id: s.id.clone(),
            scope: None,
            level: Some(PlanningLevel::CastWorld),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: None,
            retry_key: Some("reopen-key-001".into()),
        })
        .await
        .unwrap();
    assert_eq!(reopened_res.outcome, PlanningSessionOutcome::Staged);
    assert_eq!(reopened_res.attempt_id, crashed_attempt_id);
    assert_eq!(reopened_res.usage.model_calls, 4);

    srv_handle.join().unwrap();
}

#[tokio::test]
async fn transactional_commit_revalidates_duplicate_output_refs_and_parent_rejection_during_generation()
 {
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let dup_cast_payload = json!({
        "schema_version": 1,
        "level": "cast_world",
        "rationale": "Overlapping cast proposal with shared output_ref.",
        "operations": [{
            "output_ref": "cast:shared_hero",
            "operation": {
                "operation": "create_character",
                "input": {
                    "name": "Sera",
                    "summary": "Navigator",
                    "role": "protagonist",
                    "realm": null
                }
            }
        }]
    });
    let child_cast_payload = json!({
        "schema_version": 1,
        "level": "cast_world",
        "rationale": "Cast proposal whose parent contract is rejected while model call is in flight.",
        "operations": [{
            "output_ref": "cast:after_parent_reject",
            "operation": {
                "operation": "create_character",
                "input": {
                    "name": "Bram",
                    "summary": "Quartermaster",
                    "role": "supporting",
                    "realm": null
                }
            }
        }]
    });

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let call_counter = Arc::new(AtomicUsize::new(0));
    let call_counter_srv = call_counter.clone();
    let (req1_entered_tx, req1_entered_rx) = tokio::sync::oneshot::channel::<()>();
    let (req1_release_tx, req1_release_rx) = std::sync::mpsc::channel::<()>();
    let (req2_entered_tx, req2_entered_rx) = tokio::sync::oneshot::channel::<()>();
    let (req2_release_tx, req2_release_rx) = std::sync::mpsc::channel::<()>();

    let dup_body = json!({
        "choices": [{"message": {"content": dup_cast_payload.to_string()}}],
        "usage": {"prompt_tokens": 90, "completion_tokens": 40}
    })
    .to_string();
    let child_body = json!({
        "choices": [{"message": {"content": child_cast_payload.to_string()}}],
        "usage": {"prompt_tokens": 95, "completion_tokens": 45}
    })
    .to_string();

    let srv_handle = std::thread::spawn(move || {
        // Call 1: overlapping model vs host submission with same `output_ref` ("cast:shared_hero").
        // Call 2: model generation of CastWorld while parent Contract is rejected mid-flight.
        let mut entered_signals = [Some(req1_entered_tx), Some(req2_entered_tx)];
        let release_waits = [req1_release_rx, req2_release_rx];
        let bodies = [dup_body, child_body];
        for idx in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .ok();
            let mut buf = [0u8; 65536];
            let mut total = 0usize;
            loop {
                match stream.read(&mut buf[total..]) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        let s = String::from_utf8_lossy(&buf[..total]);
                        if let Some(hdr_end) = s.find("\r\n\r\n") {
                            let hdrs = &s[..hdr_end];
                            let content_len = hdrs
                                .lines()
                                .find_map(|l| {
                                    let lower = l.to_ascii_lowercase();
                                    lower
                                        .strip_prefix("content-length:")
                                        .and_then(|v| v.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            if total >= hdr_end + 4 + content_len {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            call_counter_srv.fetch_add(1, Ordering::SeqCst);
            if let Some(tx) = entered_signals[idx].take() {
                let _ = tx.send(());
            }
            let _ = release_waits[idx].recv_timeout(std::time::Duration::from_secs(5));
            let body = &bodies[idx];
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        }
    });

    let endpoint = format!("http://{addr}/v1");
    let (_tmp, svc) = service_with_toml(&format!(
        r#"
[health_check]
enabled = false

[[agents]]
id = "tx-guard-agent"
name = "Tx Guard Agent"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "tx-guard-v1"
ratings = ["general", "teen", "mature"]

[[routing]]
route = "plan"
agent = "tx-guard-agent"
"#
    ))
    .await;

    // 1. Overlapping host submissions AND overlapping model+host submissions with the same `output_ref`.
    let s_dup = svc
        .begin_planning(BeginPlanningInput {
            premise: "Prevent duplicate output_ref across overlapping advances".into(),
            scope: None,
        })
        .await
        .unwrap();

    // 1a. Two simultaneous host submissions with different retry_keys and the same `output_ref`:
    // both may validate on the read pool, but transactional commit must allow only ONE Staged proposal.
    let host_dup_payload = json!({
        "schema_version": 1,
        "level": "cast_world",
        "rationale": "Simultaneous host submission with identical output_ref.",
        "operations": [{
            "output_ref": "cast:simultaneous_host",
            "operation": {
                "operation": "create_character",
                "input": {
                    "name": "Lira",
                    "summary": "Cartographer",
                    "role": "supporting",
                    "realm": null
                }
            }
        }]
    });
    let host_a = AdvancePlanningInput {
        session_id: s_dup.id.clone(),
        scope: None,
        level: Some(PlanningLevel::CastWorld),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: None,
        host_submission: Some(PlanningHostSubmission {
            source: PlanningOutputSource::Host,
            payload: host_dup_payload.clone(),
        }),
        retry_key: Some("host-dup-key-a".into()),
    };
    let host_b = AdvancePlanningInput {
        retry_key: Some("host-dup-key-b".into()),
        ..host_a.clone()
    };
    let (res_ha, res_hb) = tokio::join!(svc.advance_planning(host_a), svc.advance_planning(host_b));
    let (res_ha, res_hb) = (res_ha.unwrap(), res_hb.unwrap());
    let host_outcomes = [res_ha.outcome, res_hb.outcome];
    assert!(host_outcomes.contains(&PlanningSessionOutcome::Staged));
    assert!(host_outcomes.contains(&PlanningSessionOutcome::ModelOutputRejected));
    let rejected_host = if res_ha.outcome == PlanningSessionOutcome::ModelOutputRejected {
        &res_ha
    } else {
        &res_hb
    };
    assert!(rejected_host.proposal.is_none());
    assert!(
        rejected_host
            .findings
            .iter()
            .any(|f| f.code == "duplicate_output_ref")
    );

    // 1b. Overlapping model advance and host submission with the same `output_ref` ("cast:shared_hero").
    let model_dup_req = AdvancePlanningInput {
        session_id: s_dup.id.clone(),
        scope: None,
        level: Some(PlanningLevel::CastWorld),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: Some(PlanningBudget {
            max_model_calls: 1,
            ..PlanningBudget::default()
        }),
        host_submission: None,
        retry_key: Some("model-dup-key".into()),
    };
    let host_shared_req = AdvancePlanningInput {
        session_id: s_dup.id.clone(),
        scope: None,
        level: Some(PlanningLevel::CastWorld),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: None,
        host_submission: Some(PlanningHostSubmission {
            source: PlanningOutputSource::Host,
            payload: dup_cast_payload.clone(),
        }),
        retry_key: Some("host-shared-key".into()),
    };
    let model_fut = svc.advance_planning(model_dup_req);
    let overlap_fut = async {
        req1_entered_rx.await.unwrap();
        let host_out = svc.advance_planning(host_shared_req).await.unwrap();
        assert_eq!(host_out.outcome, PlanningSessionOutcome::Staged);
        let _ = req1_release_tx.send(());
        host_out
    };
    let (model_res, _host_res) = tokio::join!(model_fut, overlap_fut);
    let model_res = model_res.unwrap();
    // With max_model_calls=2 and only 1 call budget used before repair (wait: we only scripted 1 HTTP response for dup_body, so set max_model_calls=1 on s_dup!)
    assert!(matches!(
        model_res.outcome,
        PlanningSessionOutcome::BudgetExhausted | PlanningSessionOutcome::ModelOutputRejected
    ));
    assert!(model_res.proposal.is_none());
    assert!(
        model_res
            .findings
            .iter()
            .any(|f| f.code == "duplicate_output_ref")
    );
    let dup_proposals = svc.list_planning_proposals(&s_dup.id, None).await.unwrap();
    let shared_hero_count = dup_proposals
        .iter()
        .filter(|p| {
            p.status == PlanningProposalStatus::Staged
                && p.artifact
                    .operations
                    .iter()
                    .any(|op| op.output_ref == "cast:shared_hero")
        })
        .count();
    assert_eq!(shared_hero_count, 1);

    // 2. Parent proposal rejected while dependent model generation is in flight:
    // must NOT stage a child proposal under the rejected parent, and must NOT waste a repair call.
    let s_parent = svc
        .begin_planning(BeginPlanningInput {
            premise: "Reject parent during child generation".into(),
            scope: None,
        })
        .await
        .unwrap();
    let contract_staged = svc
        .advance_planning(AdvancePlanningInput {
            session_id: s_parent.id.clone(),
            scope: None,
            level: Some(PlanningLevel::Contract),
            parents: None,
            target_range: None,
            allow_review_backend: None,
            rating_ceiling: None,
            budget: None,
            host_submission: Some(PlanningHostSubmission {
                source: PlanningOutputSource::Host,
                payload: json!({
                    "schema_version": 1,
                    "level": "contract",
                    "rationale": "Parent contract to be rejected during child generation.",
                    "operations": [{
                        "output_ref": "contract:project",
                        "operation": {
                            "operation": "create_project",
                            "input": {
                                "name": "Ephemeral Harbor",
                                "project_type": "novel",
                                "genre": "mystery",
                                "reader_contract": {"promise": "No orphan children", "style_notes": [], "boundaries": []}
                            }
                        }
                    }]
                }),
            }),
            retry_key: Some("parent-contract-stage".into()),
        })
        .await
        .unwrap()
        .proposal
        .unwrap();

    let child_gen_fut = svc.advance_planning(AdvancePlanningInput {
        session_id: s_parent.id.clone(),
        scope: None,
        level: Some(PlanningLevel::CastWorld),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: None,
        host_submission: None,
        retry_key: Some("child-cast-gen".into()),
    });
    let reject_parent_fut = async {
        req2_entered_rx.await.unwrap();
        let decided = svc
            .decide_planning(DecidePlanningInput {
                session_id: s_parent.id.clone(),
                scope: None,
                decision: PlanningDecision::Reject,
                actor: "human:author".into(),
                note: "Rejecting parent while child generation is in flight.".into(),
                expected_hashes: [(
                    contract_staged.id.clone(),
                    contract_staged.payload_hash.clone(),
                )]
                .into_iter()
                .collect(),
            })
            .await
            .unwrap();
        assert_eq!(decided.len(), 1);
        assert_eq!(decided[0].status, PlanningProposalStatus::Rejected);
        let _ = req2_release_tx.send(());
    };
    let (child_out, ()) = tokio::join!(child_gen_fut, reject_parent_fut);
    let child_out = child_out.unwrap();
    assert_eq!(
        child_out.outcome,
        PlanningSessionOutcome::ModelOutputRejected
    );
    assert!(child_out.proposal.is_none());
    assert!(
        child_out
            .findings
            .iter()
            .any(|f| f.code == "stale_or_inactive_planning_parent")
    );
    assert_eq!(
        child_out.generation_receipt.as_ref().unwrap().attempts_used,
        1,
        "unrepairable parent rejection must not dispatch a second model call"
    );
    assert_eq!(call_counter.load(Ordering::SeqCst), 2);

    let final_proposals = svc
        .list_planning_proposals(&s_parent.id, None)
        .await
        .unwrap();
    assert!(
        final_proposals
            .iter()
            .all(|p| p.status != PlanningProposalStatus::Staged),
        "no staged proposals may remain after parent rejection"
    );

    // 3. Direct race between read-pool validation and commit_planning_generation write transaction:
    // Both candidates pass `validate_planning_stage_input` before either commits; the second commit
    // is rejected inside the write transaction (`duplicate_output_ref`).
    let s_tx_race = svc
        .begin_planning(BeginPlanningInput {
            premise: "Direct read-to-commit race window".into(),
            scope: None,
        })
        .await
        .unwrap();
    let tx_race_input = StagePlanningInput {
        session_id: s_tx_race.id.clone(),
        scope: None,
        intent_hash: s_tx_race.intent_hash.clone(),
        schema_version: 1,
        level: PlanningLevel::CastWorld,
        parents: Default::default(),
        operations: vec![PlanningOperationItem {
            output_ref: "cast:race_twin".into(),
            operation: serde_json::from_value(json!({
                "operation": "create_character",
                "input": {
                    "name": "Vesper",
                    "summary": "Lighthouse keeper",
                    "role": "protagonist",
                    "realm": null
                }
            }))
            .unwrap(),
        }],
        rationale: "Validated before concurrent commit.".into(),
        target_range: None,
        schedule: Default::default(),
        assumptions: Vec::new(),
        author_questions: Vec::new(),
    };
    let findings_pre_1 = svc
        .repository
        .validate_planning_stage_input(tx_race_input.clone(), None)
        .await
        .unwrap();
    let findings_pre_2 = svc
        .repository
        .validate_planning_stage_input(tx_race_input.clone(), None)
        .await
        .unwrap();
    assert!(findings_pre_1.is_empty());
    assert!(findings_pre_2.is_empty());

    let dummy_receipt = PlanningGenerationReceipt {
        source: PlanningOutputSource::Host,
        prompt_id: PlanningPromptId::PlanningSeedV1,
        prompt_hash: "h".into(),
        schema_hash: "s".into(),
        route_task: None,
        resolved_route: None,
        agent_name: None,
        adapter_kind: None,
        model: None,
        attempts_used: 1,
        repaired: false,
        calls: Vec::new(),
        recorded_at: 1,
    };
    let dummy_host_req = PlanningHostRequest {
        session_id: s_tx_race.id.clone(),
        level: PlanningLevel::CastWorld,
        route_task: "plan".into(),
        prompt_id: PlanningPromptId::PlanningSeedV1,
        prompt_hash: "h".into(),
        schema_hash: "s".into(),
        system_prompt: "sys".into(),
        user_prompt: "usr".into(),
        artifact_schema: json!({}),
        parent_proposals: Default::default(),
        target_range: s_tx_race.horizon.clone(),
        route_problem: None,
        prior_findings: Vec::new(),
    };
    let meta = crate::sqlite::repository::PlanningSessionMetadataUpdate {
        session_id: s_tx_race.id.clone(),
        scope: None,
        horizon: s_tx_race.horizon.clone(),
        budget: s_tx_race.budget.clone(),
        rating_ceiling: None,
        allow_review_backend: false,
        last_outcome: PlanningSessionOutcome::Staged,
    };
    let (race_attempt_1, race_token_1, _guard_1) = match svc
        .repository
        .claim_planning_generation_attempt(&s_tx_race.id, None, Some("race-1".into()))
        .await
        .unwrap()
    {
        crate::sqlite::repository::PlanningAttemptClaim::Claimed {
            attempt_id,
            claim_token,
            active_guard,
            ..
        } => (attempt_id, claim_token, active_guard),
        _ => panic!("expected claim 1"),
    };
    let (race_attempt_2, race_token_2, _guard_2) = match svc
        .repository
        .claim_planning_generation_attempt(&s_tx_race.id, None, Some("race-2".into()))
        .await
        .unwrap()
    {
        crate::sqlite::repository::PlanningAttemptClaim::Claimed {
            attempt_id,
            claim_token,
            active_guard,
            ..
        } => (attempt_id, claim_token, active_guard),
        _ => panic!("expected claim 2"),
    };

    let rep_1 = svc
        .repository
        .commit_planning_generation(
            &race_token_1,
            meta.clone(),
            Some(crate::sqlite::repository::PlanningStagedCommitCandidate {
                input: tx_race_input.clone(),
                receipt: dummy_receipt.clone(),
                findings: Vec::new(),
                fallback_host_request: dummy_host_req.clone(),
            }),
            PlanningAdvanceReport {
                attempt_id: race_attempt_1,
                session_id: s_tx_race.id.clone(),
                level: PlanningLevel::CastWorld,
                retry_key: Some("race-1".into()),
                outcome: PlanningSessionOutcome::Staged,
                proposal: None,
                generation_receipt: Some(dummy_receipt.clone()),
                findings: Vec::new(),
                author_questions: Vec::new(),
                host_request: None,
                route_problem: None,
                usage: s_tx_race.usage.clone(),
                budget: s_tx_race.budget.clone(),
                message: None,
                created_at: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(rep_1.outcome, PlanningSessionOutcome::Staged);
    assert!(rep_1.proposal.is_some());

    let rep_2 = svc
        .repository
        .commit_planning_generation(
            &race_token_2,
            meta,
            Some(crate::sqlite::repository::PlanningStagedCommitCandidate {
                input: tx_race_input,
                receipt: dummy_receipt.clone(),
                findings: Vec::new(),
                fallback_host_request: dummy_host_req,
            }),
            PlanningAdvanceReport {
                attempt_id: race_attempt_2,
                session_id: s_tx_race.id.clone(),
                level: PlanningLevel::CastWorld,
                retry_key: Some("race-2".into()),
                outcome: PlanningSessionOutcome::Staged,
                proposal: None,
                generation_receipt: Some(dummy_receipt),
                findings: Vec::new(),
                author_questions: Vec::new(),
                host_request: None,
                route_problem: None,
                usage: s_tx_race.usage.clone(),
                budget: s_tx_race.budget.clone(),
                message: None,
                created_at: 2,
            },
        )
        .await
        .unwrap();
    assert_eq!(rep_2.outcome, PlanningSessionOutcome::ModelOutputRejected);
    assert!(rep_2.proposal.is_none());
    assert!(
        rep_2
            .findings
            .iter()
            .any(|f| f.code == "duplicate_output_ref")
    );
    assert_eq!(
        svc.list_planning_proposals(&s_tx_race.id, None)
            .await
            .unwrap()
            .len(),
        1
    );

    srv_handle.join().unwrap();
}

#[tokio::test]
async fn two_repositories_on_same_database_coordinate_retry_key_and_fence_stale_claims() {
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let valid_contract = json!({
        "schema_version": 1,
        "level": "contract",
        "rationale": "Cross-repository coordinated contract.",
        "operations": [{
            "output_ref": "contract:project",
            "operation": {
                "operation": "create_project",
                "input": {
                    "name": "Shared DB Harbor",
                    "project_type": "novel",
                    "genre": "mystery",
                    "reader_contract": {"promise": "Single dispatch across repos", "style_notes": [], "boundaries": []}
                }
            }
        }]
    });

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let call_counter = Arc::new(AtomicUsize::new(0));
    let call_counter_srv = call_counter.clone();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let body = json!({
        "choices": [{"message": {"content": valid_contract.to_string()}}],
        "usage": {"prompt_tokens": 105, "completion_tokens": 45}
    })
    .to_string();

    let srv_handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .ok();
        let mut buf = [0u8; 65536];
        let mut total = 0usize;
        loop {
            match stream.read(&mut buf[total..]) {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    let s = String::from_utf8_lossy(&buf[..total]);
                    if let Some(hdr_end) = s.find("\r\n\r\n") {
                        let hdrs = &s[..hdr_end];
                        let content_len = hdrs
                            .lines()
                            .find_map(|l| {
                                let lower = l.to_ascii_lowercase();
                                lower
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if total >= hdr_end + 4 + content_len {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        call_counter_srv.fetch_add(1, Ordering::SeqCst);
        let _ = entered_tx.send(());
        let _ = release_rx.recv_timeout(std::time::Duration::from_secs(5));
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(reply.as_bytes());
        let _ = stream.flush();
    });

    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("shared_planning.db");
    let config_path = tmp.path().join("spindle.toml");
    let endpoint = format!("http://{addr}/v1");
    std::fs::write(
        &config_path,
        format!(
            r#"
[health_check]
enabled = false

[[agents]]
id = "shared-db-agent"
name = "Shared DB Agent"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "shared-db-v1"
ratings = ["general", "teen", "mature"]

[[routing]]
route = "plan"
agent = "shared-db-agent"
"#
        ),
    )
    .unwrap();

    // Open two separate SqlitePool + Repository + SqliteSpindleService instances on the same database.
    let pool_a = crate::sqlite::SqlitePool::open(&db_path).await.unwrap();
    let router_a = crate::ai::ModelRouter::local_only();
    router_a
        .configure(Some(&config_path.display().to_string()))
        .unwrap();
    let svc_a = SqliteSpindleService::new(Repository::with_model_router(
        pool_a,
        tmp.path().join("data_a"),
        router_a,
    ));

    let pool_b = crate::sqlite::SqlitePool::open(&db_path).await.unwrap();
    let router_b = crate::ai::ModelRouter::local_only();
    router_b
        .configure(Some(&config_path.display().to_string()))
        .unwrap();
    let svc_b = SqliteSpindleService::new(Repository::with_model_router(
        pool_b,
        tmp.path().join("data_b"),
        router_b,
    ));

    let s = svc_a
        .begin_planning(BeginPlanningInput {
            premise: "Coordinate retry key across two live repositories".into(),
            scope: None,
        })
        .await
        .unwrap();

    let req = AdvancePlanningInput {
        session_id: s.id.clone(),
        scope: None,
        level: Some(PlanningLevel::Contract),
        parents: None,
        target_range: None,
        allow_review_backend: None,
        rating_ceiling: None,
        budget: None,
        host_submission: None,
        retry_key: Some("shared-db-retry-001".into()),
    };

    let fut_a = svc_a.advance_planning(req.clone());
    let fut_b = async {
        entered_rx.await.unwrap();
        // While svc_a's call is paused in flight, retrying the same key through the second
        // repository instance must detect the live claim and refuse to reclaim or dispatch a second call,
        // and lookup/list APIs on svc_b must hide the live in-progress row.
        let in_flight_retry = svc_b.advance_planning(req.clone()).await;
        assert!(
            in_flight_retry.is_err(),
            "second repository instance must not reclaim a live in-progress attempt while first is in flight"
        );
        assert!(
            in_flight_retry
                .unwrap_err()
                .to_string()
                .contains("already in progress")
        );
        assert_eq!(call_counter.load(Ordering::SeqCst), 1);
        assert!(
            svc_b
                .repository
                .find_planning_generation_attempt(&s.id, "shared-db-retry-001")
                .await
                .unwrap()
                .is_none()
        );

        // Release svc_a's HTTP response.
        let _ = release_tx.send(());
    };

    let (res_a, ()) = tokio::join!(fut_a, fut_b);
    let res_a = res_a.unwrap();
    let res_b = svc_b.advance_planning(req).await.unwrap();
    assert_eq!(res_a.outcome, PlanningSessionOutcome::Staged);
    assert_eq!(res_b.outcome, PlanningSessionOutcome::Staged);
    assert_eq!(res_a.attempt_id, res_b.attempt_id);
    assert_eq!(
        res_a.proposal.as_ref().unwrap().id,
        res_b.proposal.as_ref().unwrap().id
    );
    assert_eq!(call_counter.load(Ordering::SeqCst), 1);
    assert_eq!(
        svc_b
            .get_planning_session(&s.id)
            .await
            .unwrap()
            .usage
            .model_calls,
        1
    );
    assert_eq!(
        svc_b
            .list_planning_proposals(&s.id, None)
            .await
            .unwrap()
            .len(),
        1
    );

    // Fencing check: if an attempt is claimed (token 1) and its lease expires (or its guard is dropped)
    // and is reclaimed by a second owner (token 2), a stale reserve or commit from token 1 is rejected.
    let (fenced_id, stale_token, guard_1) = match svc_a
        .repository
        .claim_planning_generation_attempt(&s.id, None, Some("fenced-retry-key".into()))
        .await
        .unwrap()
    {
        crate::sqlite::repository::PlanningAttemptClaim::Claimed {
            attempt_id,
            claim_token,
            active_guard,
            ..
        } => (attempt_id, claim_token, active_guard),
        _ => panic!("expected initial claim"),
    };
    // Expire the lease in SQLite even while guard_1 is still held (simulating a stalled owner past TTL).
    let fenced_id_exp = fenced_id.clone();
    svc_a
        .repository
        .pool()
        .write(move |conn| {
            conn.execute(
                "UPDATE planning_generation_attempt SET lease_expires_at = 0 WHERE id = ?1",
                [&fenced_id_exp],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let (reclaimed_id, fresh_token, guard_2) = match svc_b
        .repository
        .claim_planning_generation_attempt(&s.id, None, Some("fenced-retry-key".into()))
        .await
        .unwrap()
    {
        crate::sqlite::repository::PlanningAttemptClaim::Claimed {
            attempt_id,
            claim_token,
            active_guard,
            ..
        } => (attempt_id, claim_token, active_guard),
        _ => panic!("expected reclaimed claim"),
    };
    drop(guard_1);
    assert_eq!(fenced_id, reclaimed_id);
    assert_ne!(stale_token, fresh_token);

    // A third repository instance on the same database must see guard_2's live ownership
    // even after guard_1 was dropped, and must not reclaim the fresh owner.
    let pool_c = crate::sqlite::SqlitePool::open(&db_path).await.unwrap();
    let router_c = crate::ai::ModelRouter::local_only();
    router_c
        .configure(Some(&config_path.display().to_string()))
        .unwrap();
    let svc_c = SqliteSpindleService::new(Repository::with_model_router(
        pool_c,
        tmp.path().join("data_c"),
        router_c,
    ));
    let third_reclaim = svc_c
        .repository
        .claim_planning_generation_attempt(&s.id, None, Some("fenced-retry-key".into()))
        .await;
    assert!(
        third_reclaim.is_err(),
        "third repository instance must not reclaim fresh owner after old guard is dropped"
    );
    assert!(
        third_reclaim
            .unwrap_err()
            .to_string()
            .contains("already in progress")
    );

    let stale_reserve = svc_a
        .repository
        .reserve_planning_model_call(
            &fenced_id,
            &stale_token,
            PlanningLevel::CastWorld,
            &s.id,
            None,
            PlanningBudget::default(),
            100,
        )
        .await;
    assert!(
        stale_reserve.is_err(),
        "stale claim_token must not reserve budget"
    );
    drop(guard_2);

    srv_handle.join().unwrap();
}
