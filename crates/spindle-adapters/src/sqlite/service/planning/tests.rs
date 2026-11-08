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
        note: "Change the premise".into(),
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
async fn batch_preflight_and_acceptance_fail_closed() {
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
    assert!(
        svc.decide_planning(d)
            .await
            .unwrap_err()
            .to_string()
            .contains("unavailable")
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
