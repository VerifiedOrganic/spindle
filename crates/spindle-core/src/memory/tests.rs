use super::*;

fn pos(book: i32, chapter: i32, scene: i32) -> MemoryPositionV1 {
    MemoryPositionV1::new(book, chapter, scene)
}

fn coverage(start: (i32, i32, i32), end: (i32, i32, i32)) -> MemoryCoverageV1 {
    MemoryCoverageV1 {
        start: pos(start.0, start.1, start.2),
        end: pos(end.0, end.1, end.2),
    }
}

fn summary(
    id: &str,
    layer: MemoryLayerV1,
    coverage: MemoryCoverageV1,
    text: &str,
) -> MemorySummaryV1 {
    MemorySummaryV1 {
        summary_id: id.into(),
        project_id: "project:p".into(),
        branch_id: "branch:main".into(),
        layer,
        scope_key: id.into(),
        arc_id: None,
        coverage,
        text: text.into(),
        child_summary_ids: Vec::new(),
        entity_ids: vec!["character:veyra".into()],
        thread_ids: vec!["promise:thread-1".into()],
        source_revision_hashes: vec!["rev:1".into()],
        derived_event_ids: vec!["canon_event:1".into()],
        secret_fact_ids: Vec::new(),
        tier: MemoryTierV1::Ratified,
        status: MemorySummaryStatusV1::Fresh,
        reader_visible: true,
        model: "derive-v1".into(),
        prompt_version: "memory-v1".into(),
        reducer_version: 1,
        updated_at_micros: 1,
    }
}

#[test]
fn parent_after_cursor_is_excluded_and_children_are_selected() {
    let book = summary(
        "book:1",
        MemoryLayerV1::Book,
        coverage((1, 1, 1), (1, 20, 1)),
        "whole book",
    );
    let chapters = vec![
        summary(
            "chapter:1:1",
            MemoryLayerV1::Chapter,
            coverage((1, 1, 1), (1, 1, i32::MAX)),
            "chapter one",
        ),
        summary(
            "chapter:1:2",
            MemoryLayerV1::Chapter,
            coverage((1, 2, 1), (1, 2, i32::MAX)),
            "chapter two",
        ),
    ];
    let mut summaries = vec![book];
    summaries.extend(chapters);
    let selection = select_layered_memory(
        &summaries,
        &pos(1, 2, 1),
        &MemoryVisibilityPolicyV1::default(),
    );
    let ids: Vec<&str> = selection
        .items
        .iter()
        .map(|summary| summary.summary_id.as_str())
        .collect();
    assert_eq!(ids, vec!["chapter:1:1"]);
    assert_eq!(selection.after_cursor_count, 2);
    assert!(selection.omissions.iter().any(|omission| {
        omission.layer == MemoryLayerV1::Book
            && omission.reason == MemoryOmissionReasonV1::AtOrAfterCursor
    }));
}

#[test]
fn parent_before_cursor_covers_child_summary() {
    let book = summary(
        "book:1",
        MemoryLayerV1::Book,
        coverage((1, 1, 1), (1, 2, i32::MAX)),
        "book one",
    );
    let chapter = summary(
        "chapter:1:1",
        MemoryLayerV1::Chapter,
        coverage((1, 1, 1), (1, 1, i32::MAX)),
        "chapter one",
    );
    let selection = select_layered_memory(
        &[book, chapter],
        &pos(1, 3, 1),
        &MemoryVisibilityPolicyV1::default(),
    );
    assert_eq!(selection.items.len(), 1);
    assert_eq!(selection.items[0].layer, MemoryLayerV1::Book);
    assert!(
        selection
            .omissions
            .iter()
            .any(|omission| omission.reason == MemoryOmissionReasonV1::CoveredByParent)
    );
}

#[test]
fn stale_and_secret_summaries_are_omitted_with_counts() {
    let mut stale = summary(
        "chapter:1:1",
        MemoryLayerV1::Chapter,
        coverage((1, 1, 1), (1, 1, i32::MAX)),
        "stale",
    );
    stale.status = MemorySummaryStatusV1::Stale {
        reason: "scene edited".into(),
    };
    let mut secret = summary(
        "chapter:1:2",
        MemoryLayerV1::Chapter,
        coverage((1, 2, 1), (1, 2, i32::MAX)),
        "secret",
    );
    secret.secret_fact_ids = vec!["canonical_fact:secret".into()];
    let mut hidden = summary(
        "chapter:1:3",
        MemoryLayerV1::Chapter,
        coverage((1, 3, 1), (1, 3, i32::MAX)),
        "hidden",
    );
    hidden.secret_fact_ids = vec!["canonical_fact:hidden".into()];

    let policy = MemoryVisibilityPolicyV1 {
        reader_visible_only: true,
        allowed_secret_fact_ids: BTreeSet::from(["canonical_fact:secret".into()]),
        hidden_secret_fact_ids: BTreeSet::from(["canonical_fact:hidden".into()]),
    };
    let selection = select_layered_memory(&[stale, secret, hidden], &pos(1, 4, 1), &policy);
    assert_eq!(selection.items.len(), 1);
    assert_eq!(selection.items[0].summary_id, "chapter:1:2");
    assert_eq!(selection.stale_count, 1);
    assert_eq!(selection.secret_filtered_count, 1);
}

#[test]
fn invalidation_walks_transitive_ancestors_and_collects_indexes() {
    let child = summary(
        "scene:1",
        MemoryLayerV1::Scene,
        coverage((1, 1, 1), (1, 1, 1)),
        "scene",
    );
    let chapter = summary(
        "chapter:1:1",
        MemoryLayerV1::Chapter,
        coverage((1, 1, 1), (1, 1, i32::MAX)),
        "chapter",
    );
    let book = summary(
        "book:1",
        MemoryLayerV1::Book,
        coverage((1, 1, 1), (1, 2, i32::MAX)),
        "book",
    );
    let edges = vec![
        MemoryDependencyEdgeV1 {
            summary_id: "chapter:1:1".into(),
            depends_on_summary_id: "scene:1".into(),
            source_kind: "child".into(),
        },
        MemoryDependencyEdgeV1 {
            summary_id: "book:1".into(),
            depends_on_summary_id: "chapter:1:1".into(),
            source_kind: "child".into(),
        },
    ];
    let plan = plan_memory_invalidation(
        &["scene:1".into()],
        &[child, chapter, book],
        &edges,
        "scene_edit",
    );
    assert_eq!(
        plan.stale_summary_ids,
        vec!["book:1", "chapter:1:1", "scene:1"]
    );
    assert_eq!(plan.jobs.len(), 3);
    assert_eq!(plan.affected_entity_ids, vec!["character:veyra"]);
    assert_eq!(plan.affected_thread_ids, vec!["promise:thread-1"]);
}

fn thread(
    id: &str,
    kind: ThreadKindV1,
    urgency: ThreadUrgencyV1,
    scene_relevant: bool,
    age: u32,
) -> ThreadDigestV1 {
    ThreadDigestV1 {
        thread_id: id.into(),
        kind,
        label: id.into(),
        urgency,
        due_position: None,
        planned: false,
        blocked: false,
        scene_relevant,
        age_scene_count: age,
        source_event_ids: Vec::new(),
    }
}

#[test]
fn urgent_high_id_thread_beats_low_id_filler_and_overflow_is_visible() {
    let threads = vec![
        thread(
            "promise:aaa-filler",
            ThreadKindV1::Promise,
            ThreadUrgencyV1::Open,
            false,
            1,
        ),
        thread(
            "promise:zzz-urgent",
            ThreadKindV1::Promise,
            ThreadUrgencyV1::Overdue,
            false,
            9,
        ),
    ];
    let policy = ThreadRetrievalPolicyV1 {
        max_total: 1,
        page_offset: 0,
        class_quotas: BTreeMap::new(),
    };
    let out = retrieve_threads(&threads, &policy);
    assert_eq!(out.selected.len(), 1);
    assert_eq!(out.selected[0].thread_id, "promise:zzz-urgent");
    assert_eq!(out.overflow_total, 1);
    assert_eq!(out.omitted_by_class.get(&ThreadKindV1::Promise), Some(&1));
    assert_eq!(out.next_offset, Some(1));
}

#[test]
fn class_quotas_reserve_capacity_and_page_overflow() {
    let threads = vec![
        thread(
            "promise:1",
            ThreadKindV1::Promise,
            ThreadUrgencyV1::Overdue,
            false,
            2,
        ),
        thread(
            "promise:2",
            ThreadKindV1::Promise,
            ThreadUrgencyV1::DueSoon,
            false,
            2,
        ),
        thread(
            "conflict:1",
            ThreadKindV1::Conflict,
            ThreadUrgencyV1::BlockingContradiction,
            true,
            2,
        ),
    ];
    let policy = ThreadRetrievalPolicyV1 {
        max_total: 2,
        page_offset: 0,
        class_quotas: BTreeMap::from([(ThreadKindV1::Promise, 1), (ThreadKindV1::Conflict, 1)]),
    };
    let out = retrieve_threads(&threads, &policy);
    assert_eq!(out.selected.len(), 2);
    assert!(
        out.selected
            .iter()
            .any(|thread| thread.thread_id == "promise:1")
    );
    assert!(
        out.selected
            .iter()
            .any(|thread| thread.thread_id == "conflict:1")
    );
}

#[test]
fn budget_preserves_hard_constraints_and_reports_truncation() {
    let scene = summary(
        "scene:1",
        MemoryLayerV1::Scene,
        coverage((1, 1, 1), (1, 1, 1)),
        &"scene detail ".repeat(30),
    );
    let chapter = summary(
        "chapter:1:1",
        MemoryLayerV1::Chapter,
        coverage((1, 1, 1), (1, 1, i32::MAX)),
        &"chapter summary ".repeat(30),
    );
    let selection = select_layered_memory(
        &[chapter, scene],
        &pos(1, 1, 2),
        &MemoryVisibilityPolicyV1::default(),
    );
    let policy = MemoryBudgetPolicyV1 {
        total_tokens: 100,
        hard_constraint_tokens: 20,
        layer_caps: BTreeMap::from([(MemoryLayerV1::Scene, 30), (MemoryLayerV1::Chapter, 30)]),
    };
    let budgeted = build_memory_budgeted_context(&selection, &policy);
    assert!(budgeted.hard_constraints_preserved);
    assert!(!budgeted.budget_exceeded);
    assert!(budgeted.estimated_tokens <= 100);
    assert!(budgeted.included_sections.contains(&"scene".to_string()));
    assert!(budgeted.truncated_sections.contains(&"scene".to_string()));
}

#[test]
fn budget_exceeded_by_hard_constraints_is_explicit() {
    let selection = LayeredMemorySelectionV1 {
        cursor: pos(1, 1, 1),
        items: Vec::new(),
        omissions: Vec::new(),
        secret_filtered_count: 0,
        stale_count: 0,
        after_cursor_count: 0,
    };
    let policy = MemoryBudgetPolicyV1 {
        total_tokens: 50,
        hard_constraint_tokens: 60,
        layer_caps: BTreeMap::new(),
    };
    let budgeted = build_memory_budgeted_context(&selection, &policy);
    assert!(budgeted.budget_exceeded);
    assert!(!budgeted.hard_constraints_preserved);
}

#[test]
fn direct_state_entity_retrieval_needs_no_embedding_provider() {
    let entity = EntityRetrievalV1 {
        subject_id: "character:veyra".into(),
        names: vec!["Veyra".into()],
        aliases: vec!["the captain".into()],
        location: Some("location:market".into()),
        possessions: vec!["item:sword".into()],
        injuries: vec!["injury:rib".into()],
        knowledge: vec!["proposition:secret".into()],
        neighbors: vec!["character:kael".into()],
        source_event_ids: vec!["canon_event:1".into()],
        provider: EntityRetrievalProviderV1::DirectState,
    };
    assert!(entity.is_available_without_embedding());
}
