//! Phase 4 scanner regression eval + corpus precision/recall gate:
//! load `evals/anti_slop/cases.json` and `evals/anti_slop/corpus.json`,
//! assert expected shelf hits, print per-shelf precision/recall, and enforce
//! that every `hard` shelf achieves precision >= 0.9 on the corpus.
//!
//! Run via `python3 evals/anti_slop.py regress`, `python3 evals/anti_slop.py report`,
//! or `cargo test -p spindle-core --test antislop_eval -- --nocapture`.

use serde::{Deserialize, Serialize};
use spindle_core::style::antislop::{
    DEFAULT_CATALOG_MARKDOWN, NON_PORTS, SCANNER_SHELF_IDS, ScanInput, Severity, ShelfPack,
    catalog_shelf_ids, scan,
};
use std::collections::{BTreeMap, BTreeSet};
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
    preference_dims: BTreeMap<String, String>,
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

#[derive(Debug, Deserialize)]
struct CorpusBundle {
    pack_id: String,
    domain: String,
    hard_precision_gate: f64,
    min_negative_passages: usize,
    shelf_ids: Vec<String>,
    passages: Vec<CorpusPassage>,
}

#[derive(Debug, Deserialize)]
struct CorpusPassage {
    id: String,
    kind: String,
    source: CorpusSource,
    prose: String,
    expected_shelves: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CorpusSource {
    author: String,
    title: String,
    year: u32,
    repository: String,
    license: String,
}

#[derive(Debug, Clone, Serialize)]
struct ShelfPrecisionRecall {
    shelf_id: String,
    severity: String,
    tp: usize,
    fp: usize,
    r#fn: usize,
    tn: usize,
    precision: f64,
    recall: f64,
    hard_gate_ok: bool,
}

#[derive(Debug, Clone, Serialize)]
struct CorpusPrecisionReport {
    total_passages: usize,
    negative_passages: usize,
    positive_passages: usize,
    hard_precision_gate: f64,
    gate_passed: bool,
    shelves: Vec<ShelfPrecisionRecall>,
}

fn cases_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/anti_slop/cases.json")
}

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/anti_slop/corpus.json")
}

fn load_bundle() -> EvalBundle {
    let raw = std::fs::read_to_string(cases_path()).expect("evals/anti_slop/cases.json");
    serde_json::from_str(&raw).expect("cases.json schema")
}

fn load_corpus() -> CorpusBundle {
    let raw = std::fs::read_to_string(corpus_path()).expect("evals/anti_slop/corpus.json");
    serde_json::from_str(&raw).expect("corpus.json schema")
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

#[test]
fn corpus_precision_recall_report_and_hard_shelf_gate() {
    let pack = ShelfPack::load_default().expect("v0 pack");
    let corpus = load_corpus();
    assert_eq!(corpus.pack_id, "fiction.default");
    assert_eq!(corpus.domain, "fiction");
    assert!(
        (corpus.hard_precision_gate - 0.9).abs() < f64::EPSILON,
        "hard precision gate must be 0.9"
    );
    assert_eq!(
        corpus.shelf_ids,
        SCANNER_SHELF_IDS
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>()
    );

    let negative_count = corpus
        .passages
        .iter()
        .filter(|p| p.kind == "negative")
        .count();
    let positive_count = corpus
        .passages
        .iter()
        .filter(|p| p.kind == "positive")
        .count();
    assert!(
        negative_count >= corpus.min_negative_passages && negative_count >= 150,
        "corpus must include at least 150 public-domain negative passages, got {negative_count}"
    );
    assert!(
        positive_count >= 12,
        "corpus must include positive examples across shelves, got {positive_count}"
    );

    for passage in &corpus.passages {
        assert!(
            !passage.prose.trim().is_empty(),
            "{}: empty prose",
            passage.id
        );
        assert!(
            !passage.source.author.trim().is_empty()
                && !passage.source.title.trim().is_empty()
                && !passage.source.repository.trim().is_empty()
                && !passage.source.license.trim().is_empty(),
            "{}: missing source/license provenance",
            passage.id
        );
        if passage.kind == "negative" {
            assert!(
                passage.source.year <= 1928,
                "{}: negative passage must be public-domain pre-1929 (got {})",
                passage.id,
                passage.source.year
            );
            assert!(
                passage.expected_shelves.is_empty(),
                "{}: negative passage must not expect shelf hits",
                passage.id
            );
        } else {
            assert!(
                !passage.expected_shelves.is_empty(),
                "{}: positive passage must declare expected_shelves",
                passage.id
            );
        }
    }

    // Scan every passage once and collect hits per passage.
    let mut passage_hits: Vec<(&CorpusPassage, BTreeSet<String>)> =
        Vec::with_capacity(corpus.passages.len());
    for passage in &corpus.passages {
        let report = scan(&pack, &ScanInput::fiction(&passage.prose));
        let hit_ids: BTreeSet<String> = report.hits.iter().map(|h| h.shelf_id.clone()).collect();
        passage_hits.push((passage, hit_ids));
    }

    let mut shelf_reports = Vec::new();
    let mut gate_passed = true;

    println!("\n=== Anti-Slop Corpus Precision / Recall Report ===");
    println!(
        "passages={} (negative={}, positive={}) hard_precision_gate={:.2}",
        corpus.passages.len(),
        negative_count,
        positive_count,
        corpus.hard_precision_gate
    );
    println!(
        "{:<24} {:<6} {:>4} {:>4} {:>4} {:>4} {:>9} {:>9} {:>8}",
        "shelf_id", "sev", "tp", "fp", "fn", "tn", "precision", "recall", "gate_ok"
    );

    for shelf in &pack.shelves {
        let effective_sev = pack.effective_severity(shelf, None);
        let sev_label = match effective_sev {
            Severity::Hard => "hard",
            Severity::Soft => "soft",
        };
        let mut tp = 0usize;
        let mut fp = 0usize;
        let mut fn_count = 0usize;
        let mut tn = 0usize;

        for (passage, hit_ids) in &passage_hits {
            let expected = passage.expected_shelves.iter().any(|s| s == &shelf.id);
            let actual = hit_ids.contains(&shelf.id);
            match (expected, actual) {
                (true, true) => tp += 1,
                (false, true) => {
                    fp += 1;
                    println!(
                        "  [FP] shelf={} passage={} ({} - {})",
                        shelf.id, passage.id, passage.source.author, passage.source.title
                    );
                }
                (true, false) => {
                    fn_count += 1;
                    println!("  [FN] shelf={} passage={}", shelf.id, passage.id);
                }
                (false, false) => tn += 1,
            }
        }

        let precision = if tp + fp > 0 {
            tp as f64 / (tp + fp) as f64
        } else {
            0.0
        };
        let recall = if tp + fn_count > 0 {
            tp as f64 / (tp + fn_count) as f64
        } else {
            0.0
        };
        let hard_gate_ok =
            effective_sev != Severity::Hard || precision >= corpus.hard_precision_gate;
        if !hard_gate_ok {
            gate_passed = false;
        }

        println!(
            "{:<24} {:<6} {:>4} {:>4} {:>4} {:>4} {:>9.3} {:>9.3} {:>8}",
            shelf.id, sev_label, tp, fp, fn_count, tn, precision, recall, hard_gate_ok
        );

        assert!(
            tp + fn_count > 0,
            "corpus must include positive examples for shelf {}",
            shelf.id
        );
        if effective_sev == Severity::Hard {
            assert!(
                precision >= corpus.hard_precision_gate,
                "hard shelf `{}` failed precision gate: precision={:.3} < {:.2} (tp={tp}, fp={fp})",
                shelf.id,
                precision,
                corpus.hard_precision_gate
            );
        }

        shelf_reports.push(ShelfPrecisionRecall {
            shelf_id: shelf.id.clone(),
            severity: sev_label.to_string(),
            tp,
            fp,
            r#fn: fn_count,
            tn,
            precision: (precision * 10000.0).round() / 10000.0,
            recall: (recall * 10000.0).round() / 10000.0,
            hard_gate_ok,
        });
    }

    let summary = CorpusPrecisionReport {
        total_passages: corpus.passages.len(),
        negative_passages: negative_count,
        positive_passages: positive_count,
        hard_precision_gate: corpus.hard_precision_gate,
        gate_passed,
        shelves: shelf_reports,
    };
    println!(
        "ANTISLOP_CORPUS_REPORT_JSON={}",
        serde_json::to_string(&summary).expect("serialize corpus report")
    );
    assert!(gate_passed, "all hard shelves must have precision >= 0.9");
}
