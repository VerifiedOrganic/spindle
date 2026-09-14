//! Phase 4 scanner regression eval: load `evals/anti_slop/cases.json` and
//! assert expected shelf hits. Run via `python3 evals/anti_slop.py regress`
//! or `cargo test -p spindle-core --test antislop_eval`.

use serde::Deserialize;
use spindle_core::style::antislop::{
    DEFAULT_CATALOG_MARKDOWN, NON_PORTS, SCANNER_SHELF_IDS, ScanInput, Severity, ShelfPack,
    catalog_shelf_ids, scan,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct EvalBundle {
    pack_id: String,
    domain: String,
    shelf_ids: Vec<String>,
    preference_dims: Vec<String>,
    interpretation: String,
    cases: Vec<EvalCase>,
}

#[derive(Debug, Deserialize)]
struct EvalCase {
    id: String,
    name: String,
    prose: String,
    expect: EvalExpect,
    #[serde(default)]
    preference_dims: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct EvalExpect {
    #[serde(default)]
    skipped: bool,
    #[serde(default)]
    hard_count: Option<u32>,
    #[serde(default)]
    must_include: Vec<ExpectedHit>,
    #[serde(default)]
    must_exclude: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExpectedHit {
    shelf_id: String,
    severity: String,
}

fn cases_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/anti_slop/cases.json")
}

fn load_bundle() -> EvalBundle {
    let raw = std::fs::read_to_string(cases_path()).expect("evals/anti_slop/cases.json");
    serde_json::from_str(&raw).expect("cases.json schema")
}

#[test]
fn eval_cases_cover_twelve_fiction_shelves_without_quality_claim() {
    let bundle = load_bundle();
    assert_eq!(bundle.pack_id, "fiction.default");
    assert_eq!(bundle.domain, "fiction");
    assert_eq!(
        bundle.shelf_ids,
        SCANNER_SHELF_IDS
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        bundle
            .interpretation
            .to_ascii_lowercase()
            .contains("not a quality")
    );
    assert!(
        bundle
            .interpretation
            .to_ascii_lowercase()
            .contains("superiority")
    );
    let allowed_dims: BTreeSet<_> = bundle.preference_dims.iter().cloned().collect();
    let mut covered = BTreeSet::new();
    for case in &bundle.cases {
        for hit in &case.expect.must_include {
            covered.insert(hit.shelf_id.clone());
        }
        for (dim, value) in &case.preference_dims {
            assert!(
                allowed_dims.contains(dim),
                "{}: unknown preference dim {dim}",
                case.id
            );
            assert!(
                matches!(value.as_str(), "present" | "absent" | "n/a"),
                "{}: preference dim {dim} must be a label, not a quality score",
                case.id
            );
        }
    }
    for id in &bundle.shelf_ids {
        assert!(
            covered.contains(id),
            "eval cases must include a regression for {id}"
        );
    }
}

#[test]
fn eval_cases_match_scanner_regressions() {
    let pack = ShelfPack::load_default().expect("v0 pack");
    let bundle = load_bundle();
    for case in &bundle.cases {
        let report = scan(&pack, &ScanInput::fiction(&case.prose));
        println!(
            "=== {} {} skipped={} hard={} soft={} ===",
            case.id, case.name, report.skipped, report.hard_count, report.soft_count
        );
        for hit in &report.hits {
            println!("  {} {:?}", hit.shelf_id, hit.severity);
        }
        assert_eq!(
            report.skipped, case.expect.skipped,
            "{} skipped mismatch",
            case.id
        );
        if let Some(hard) = case.expect.hard_count {
            assert_eq!(report.hard_count, hard, "{} hard_count", case.id);
        }
        for expected in &case.expect.must_include {
            let severity = match expected.severity.as_str() {
                "hard" => Severity::Hard,
                "soft" => Severity::Soft,
                other => panic!("{}: bad severity {other}", case.id),
            };
            assert!(
                report
                    .hits
                    .iter()
                    .any(|hit| hit.shelf_id == expected.shelf_id && hit.severity == severity),
                "{} missing {} {:?}",
                case.id,
                expected.shelf_id,
                severity
            );
            if severity == Severity::Soft {
                assert!(
                    report
                        .hits
                        .iter()
                        .filter(|hit| hit.shelf_id == expected.shelf_id)
                        .all(|hit| hit.severity == Severity::Soft),
                    "{} {} must stay soft",
                    case.id,
                    expected.shelf_id
                );
            }
        }
        for excluded in &case.expect.must_exclude {
            assert!(
                report.hits.iter().all(|hit| hit.shelf_id != *excluded),
                "{} must not emit {excluded}: {report:?}",
                case.id
            );
            if NON_PORTS.contains(&excluded.as_str()) {
                assert!(
                    pack.shelf(excluded).is_none(),
                    "{excluded} must not be a shelf"
                );
            }
        }
        if case.id == "AS06" {
            assert_eq!(
                report.hard_count, 0,
                "solitary_fade must stay out of hard_count"
            );
        }
        if case.id == "AS05" {
            assert_eq!(report.hard_count, 0, "said_bookism product lock");
        }
    }
}

#[test]
fn eval_manifest_ids_match_catalog() {
    let bundle = load_bundle();
    assert_eq!(
        bundle.shelf_ids,
        catalog_shelf_ids(DEFAULT_CATALOG_MARKDOWN)
    );
}
