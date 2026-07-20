//! Authoring-run event journal emitter (ADR 0002, evolution §3.4).
//!
//! [`RunJournal`] wraps the append-only repository journal with the ADR D3.3
//! **never-fails** discipline: emission happens AFTER the state change commits,
//! and a journal write error logs at `warn` and returns unit — it never fails
//! the run step. Consumers therefore treat the journal as an honest-but-not-
//! guaranteed-complete timeline; `authoring_status` (DB state) stays the source
//! of truth (ADR D3.3).
//!
//! Payloads carry ids / paths / counts / enums ONLY — never prose, fact/secret
//! text, evidence, or model output (ADR D3.1). The payload builders in this
//! module construct that shape by hand from typed inputs, so the no-prose
//! contract is enforced at the emission boundary rather than trusted to
//! callers.

use serde_json::{Map, Value, json};
use spindle_adapters::sqlite::Repository;

/// The ADR 0002 D2 v1 kind vocabulary. Reserved P3/P4 kinds
/// (`checkpoint_auto_approved`, `replan_proposed`, `deltas_decided`) are listed
/// so consumers can pre-register handlers and the payload-discipline pin can
/// assert every emitted kind is a member — they are NOT emitted until those
/// phases land (ADR D3.4).
pub const RUN_EVENT_KINDS: &[&str] = &[
    "run_started",
    "scene_drafted",
    "scene_verify_completed",
    "scene_revised",
    "scene_committed",
    "scene_mined",
    "deltas_decided",
    "beats_annotated",
    "chapter_summarized",
    "checkpoint_created",
    "checkpoint_auto_approved",
    "checkpoint_blocked",
    "checkpoint_reviewed",
    "replan_proposed",
    "pass_skipped",
    "run_blocked",
    "run_resumed",
    "run_paused",
    "run_completed",
];

/// Is `kind` a member of the ADR D2 vocabulary? Used by the payload-discipline
/// pin (J4) and available to any consumer validating a stream.
pub fn is_run_event_kind(kind: &str) -> bool {
    RUN_EVENT_KINDS.contains(&kind)
}

/// Never-fails wrapper over the append-only run-event journal (ADR D3.3).
///
/// Cheap to construct (borrows the repository); create one per handler where
/// events are emitted.
#[derive(Clone, Copy)]
pub struct RunJournal<'a> {
    repo: &'a Repository,
}

impl<'a> RunJournal<'a> {
    pub fn new(repo: &'a Repository) -> Self {
        Self { repo }
    }

    /// Append one event, honoring the ADR D3.3 discipline: on any error, log at
    /// `warn` and return unit. A journaling outage must never halt drafting.
    pub async fn emit(&self, run_id: &str, kind: &str, payload: Value) {
        // Dev-only guard: every emitted kind must be in the ADR D2 vocabulary
        // (the kind set is a one-way door — a typo'd kind is a bug, not a new
        // kind). No cost in release; catches drift in tests/dev.
        debug_assert!(
            is_run_event_kind(kind),
            "emitting unknown journal kind '{kind}' (not in ADR 0002 D2 vocabulary)"
        );
        if let Err(error) = self.repo.append_run_event(run_id, kind, payload).await {
            tracing::warn!(
                run_id,
                kind,
                error = format!("{error:#}"),
                "run-event journal append failed; run proceeds (ADR D3.3)"
            );
        }
    }
}

/// Insert `key: value` into `map` only when `value` is `Some` — ADR D2 marks
/// several payload keys optional (`mode?`, `mining_policy?`, …); a `None` is
/// omitted rather than serialized as JSON null.
fn insert_opt<T: Into<Value>>(map: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(value) = value {
        map.insert(key.to_string(), value.into());
    }
}

/// `run_started` payload (ADR D2). Optional keys are omitted when absent.
pub fn run_started_payload(
    book_number: i32,
    start_chapter: i32,
    end_chapter: i32,
    mode: Option<&str>,
    mining_policy: Option<&str>,
    max_revise_attempts: Option<i32>,
) -> Value {
    let mut map = Map::new();
    map.insert("book_number".into(), json!(book_number));
    map.insert("start_chapter".into(), json!(start_chapter));
    map.insert("end_chapter".into(), json!(end_chapter));
    insert_opt(&mut map, "mode", mode.map(str::to_string));
    insert_opt(&mut map, "mining_policy", mining_policy.map(str::to_string));
    // ADR D2 names the key `revise_policy?`; we carry the resolved bounded
    // budget under it (ids/counts, never prose).
    insert_opt(&mut map, "revise_policy", max_revise_attempts);
    Value::Object(map)
}

/// `scene_drafted` payload (ADR D2). `origin` is `"host"` or `"agent"`.
pub fn scene_drafted_payload(
    chapter: i32,
    scene_order: i32,
    scene_id: &str,
    origin: &str,
) -> Value {
    json!({
        "chapter": chapter,
        "scene_order": scene_order,
        "scene_id": scene_id,
        "origin": origin,
    })
}

/// Parse a `verify_detail` string into a `finding_counts` map and the ADR D2
/// `verdict`. The detail strings the harness records are already prose-free
/// (counts + status words — evolution I8); this maps them to the structured
/// payload shape without ever carrying prose.
///
/// - `verify_status == "clean"` → `verdict = "clean"`, empty counts.
/// - otherwise (`findings` / `parked_findings`) → `verdict = "findings"`, with
///   `{"actionable": n}` parsed from the leading integer in the detail (the
///   harness renders `"{count} finding(s) …"`). `error` status yields
///   `verdict = "findings"` with empty counts (the count is unknown).
pub fn verify_completed_payload(
    chapter: i32,
    scene_order: i32,
    scene_id: &str,
    verify_status: &str,
    verify_detail: Option<&str>,
) -> Value {
    let verdict = if verify_status == "clean" {
        "clean"
    } else {
        "findings"
    };
    let mut finding_counts = Map::new();
    if verdict == "findings"
        && let Some(count) = leading_count(verify_detail.unwrap_or(""))
    {
        finding_counts.insert("actionable".into(), json!(count));
    }
    json!({
        "chapter": chapter,
        "scene_order": scene_order,
        "scene_id": scene_id,
        "finding_counts": Value::Object(finding_counts),
        "verdict": verdict,
    })
}

/// `scene_revised` payload (ADR D2).
pub fn scene_revised_payload(
    chapter: i32,
    scene_order: i32,
    scene_id: &str,
    attempt: i32,
    directive_finding_count: usize,
) -> Value {
    json!({
        "chapter": chapter,
        "scene_order": scene_order,
        "scene_id": scene_id,
        "attempt": attempt,
        "directive_finding_count": directive_finding_count,
    })
}

/// `scene_committed` / `beats_annotated` share this `(chapter, scene_order,
/// scene_id)` shape (ADR D2).
pub fn scene_ref_payload(chapter: i32, scene_order: i32, scene_id: &str) -> Value {
    json!({
        "chapter": chapter,
        "scene_order": scene_order,
        "scene_id": scene_id,
    })
}

/// `scene_mined` payload (ADR D2). `staged_count` is present on a `staged`
/// outcome (parsed from the harness `"staged N delta(s)"` detail); otherwise
/// `skip_reason` carries the prose-free status/skip word.
pub fn scene_mined_payload(
    chapter: i32,
    scene_order: i32,
    scene_id: &str,
    mine_status: &str,
    mine_detail: Option<&str>,
) -> Value {
    let mut map = Map::new();
    map.insert("chapter".into(), json!(chapter));
    map.insert("scene_order".into(), json!(scene_order));
    map.insert("scene_id".into(), json!(scene_id));
    map.insert("mine_status".into(), json!(mine_status));
    if mine_status == "staged" {
        insert_opt(
            &mut map,
            "staged_count",
            leading_count(mine_detail.unwrap_or("")),
        );
    } else {
        insert_opt(&mut map, "skip_reason", mine_detail.map(str::to_string));
    }
    Value::Object(map)
}

/// `chapter_summarized` payload (ADR D2). `summary_artifact_path` is an artifact
/// PATH — it points at content, it never carries content (ADR D3.1).
pub fn chapter_summarized_payload(chapter: i32, summary_artifact_path: Option<&str>) -> Value {
    let mut map = Map::new();
    map.insert("chapter".into(), json!(chapter));
    insert_opt(
        &mut map,
        "summary_artifact_path",
        summary_artifact_path.map(str::to_string),
    );
    Value::Object(map)
}

/// `checkpoint_created` payload (ADR D2). `sampled_scene_ids` are ids only.
pub fn checkpoint_created_payload(
    start_chapter: i32,
    end_chapter: i32,
    save_point_id: &str,
    sampled_scene_ids: &[String],
) -> Value {
    json!({
        "start_chapter": start_chapter,
        "end_chapter": end_chapter,
        "save_point_id": save_point_id,
        "sampled_scene_ids": sampled_scene_ids,
    })
}

/// `checkpoint_auto_approved` payload (ADR D2, reserved kind activated in P3).
/// Keys: `start_chapter, end_chapter, policy, finding_counts` — the policy under
/// which the automation self-cleared, and the deep-consistency severity counts
/// that satisfied the threshold (all zero at or above the policy's floor, by
/// construction — auto_advisory tolerates `info`, auto_strict tolerates none).
/// `finding_counts` is a `{severity: n}` map, ids/counts only (ADR D3.1).
pub fn checkpoint_auto_approved_payload(
    start_chapter: i32,
    end_chapter: i32,
    policy: &str,
    finding_counts: &std::collections::BTreeMap<String, i64>,
) -> Value {
    let mut counts = Map::new();
    for (severity, count) in finding_counts {
        counts.insert(severity.clone(), json!(count));
    }
    json!({
        "start_chapter": start_chapter,
        "end_chapter": end_chapter,
        "policy": policy,
        "finding_counts": Value::Object(counts),
    })
}

/// `checkpoint_blocked` payload (ADR D2). `reason` reuses a prose-free
/// enum/status word (e.g. `"await_checkpoint_review"`).
pub fn checkpoint_blocked_payload(start_chapter: i32, end_chapter: i32, reason: &str) -> Value {
    json!({
        "start_chapter": start_chapter,
        "end_chapter": end_chapter,
        "reason": reason,
    })
}

/// `checkpoint_reviewed` payload (ADR D2).
pub fn checkpoint_reviewed_payload(
    start_chapter: i32,
    end_chapter: i32,
    directive_count: usize,
) -> Value {
    json!({
        "start_chapter": start_chapter,
        "end_chapter": end_chapter,
        "directive_count": directive_count,
    })
}

/// `pass_skipped` payload (ADR D2). `reason` reuses the harness's recorded
/// detail string, which is already prose-free (counts / status words —
/// evolution I8).
pub fn pass_skipped_payload(
    pass: &str,
    chapter: Option<i32>,
    scene_order: Option<i32>,
    reason: &str,
) -> Value {
    let mut map = Map::new();
    map.insert("pass".into(), json!(pass));
    insert_opt(&mut map, "chapter", chapter);
    insert_opt(&mut map, "scene_order", scene_order);
    map.insert("reason".into(), json!(reason));
    Value::Object(map)
}

/// `run_blocked` / `run_resumed` / `run_paused` / `run_completed` payload
/// (ADR D2): an optional prose-free `reason`.
pub fn run_status_payload(reason: Option<&str>) -> Value {
    let mut map = Map::new();
    insert_opt(&mut map, "reason", reason.map(str::to_string));
    Value::Object(map)
}

/// Parse a leading unsigned integer out of a detail string (`"3 finding(s) …"`,
/// `"staged 2 delta(s)"`). Returns `None` when no integer is present. This is
/// how the structured `finding_counts` / `staged_count` are recovered from the
/// harness's already-prose-free detail lines without re-plumbing the executor.
fn leading_count(detail: &str) -> Option<i64> {
    detail
        .split_whitespace()
        .find_map(|token| token.parse::<i64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_membership_matches_adr_d2() {
        assert!(is_run_event_kind("run_started"));
        assert!(is_run_event_kind("scene_verify_completed"));
        assert!(is_run_event_kind("run_completed"));
        // Reserved-but-not-yet-emitted kinds are still valid vocabulary.
        assert!(is_run_event_kind("checkpoint_auto_approved"));
        assert!(is_run_event_kind("replan_proposed"));
        assert!(!is_run_event_kind("totally_made_up"));
    }

    #[test]
    fn run_started_omits_absent_optional_keys() {
        let payload = run_started_payload(1, 1, 3, None, None, None);
        let obj = payload.as_object().unwrap();
        assert_eq!(obj["book_number"], json!(1));
        assert_eq!(obj["end_chapter"], json!(3));
        assert!(!obj.contains_key("mode"));
        assert!(!obj.contains_key("mining_policy"));
        assert!(!obj.contains_key("revise_policy"));

        let full = run_started_payload(1, 1, 3, Some("agent"), Some("propose_all"), Some(2));
        let obj = full.as_object().unwrap();
        assert_eq!(obj["mode"], json!("agent"));
        assert_eq!(obj["mining_policy"], json!("propose_all"));
        assert_eq!(obj["revise_policy"], json!(2));
    }

    #[test]
    fn verify_completed_maps_status_to_verdict_and_counts() {
        let clean = verify_completed_payload(1, 1, "scene:a", "clean", Some("0 finding(s)"));
        assert_eq!(clean["verdict"], json!("clean"));
        assert_eq!(clean["finding_counts"], json!({}));

        let findings = verify_completed_payload(
            1,
            1,
            "scene:a",
            "findings",
            Some("3 finding(s) at or above warning"),
        );
        assert_eq!(findings["verdict"], json!("findings"));
        assert_eq!(findings["finding_counts"]["actionable"], json!(3));

        let parked = verify_completed_payload(
            1,
            1,
            "scene:a",
            "parked_findings",
            Some("2 finding(s) parked after 1 revision(s)"),
        );
        assert_eq!(parked["verdict"], json!("findings"));
        assert_eq!(parked["finding_counts"]["actionable"], json!(2));
    }

    #[test]
    fn checkpoint_auto_approved_carries_policy_and_severity_counts() {
        let mut counts = std::collections::BTreeMap::new();
        counts.insert("error".to_string(), 0);
        counts.insert("warning".to_string(), 0);
        counts.insert("info".to_string(), 2);
        let payload = checkpoint_auto_approved_payload(1, 3, "auto_advisory", &counts);
        let obj = payload.as_object().unwrap();
        assert_eq!(obj["start_chapter"], json!(1));
        assert_eq!(obj["end_chapter"], json!(3));
        assert_eq!(obj["policy"], json!("auto_advisory"));
        assert_eq!(obj["finding_counts"]["error"], json!(0));
        assert_eq!(obj["finding_counts"]["warning"], json!(0));
        assert_eq!(obj["finding_counts"]["info"], json!(2));
        // The kind is a member of the ADR D2 vocabulary (activated reserved kind).
        assert!(is_run_event_kind("checkpoint_auto_approved"));
    }

    #[test]
    fn scene_mined_carries_staged_count_or_skip_reason() {
        let staged = scene_mined_payload(1, 1, "scene:a", "staged", Some("staged 2 delta(s)"));
        assert_eq!(staged["mine_status"], json!("staged"));
        assert_eq!(staged["staged_count"], json!(2));
        assert!(staged.as_object().unwrap().get("skip_reason").is_none());

        let skipped = scene_mined_payload(1, 1, "scene:a", "skipped", Some("rating_gated"));
        assert_eq!(skipped["mine_status"], json!("skipped"));
        assert_eq!(skipped["skip_reason"], json!("rating_gated"));
        assert!(skipped.as_object().unwrap().get("staged_count").is_none());
    }
}
