//! Phase 3 critic / revise loop contracts.
//!
//! Hard verify findings use a rewrite-from-beats prompt (≤1–2 passes), not a
//! paraphrase-humanizer. Soft hits stay advisory and never enter the verify
//! finding set. Dual-persona review injects the scan report, requires Craft
//! Technician shelf-ID citations, and gives the Literary Critic a structure
//! block that is not a BLUF / Voices / Flesch / delve gate.

use super::Severity;
use super::scan::{AntiSlopHit, AntiSlopReport};

/// Scene-scoped verify / consistency check name for fiction anti-slop.
pub const ANTI_SLOP_CHECK: &str = "anti_slop";

/// Hard over-limit hit promoted into the scene-scoped verify set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AntiSlopVerifyFinding {
    pub check_type: String,
    pub severity: String,
    pub message: String,
    pub suggested_action: String,
    pub shelf_id: String,
}

/// Dual-persona prompt injection: report + persona-specific blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DualPersonaAntiSlopInjection {
    pub report_block: String,
    pub craft_technician_block: String,
    pub literary_critic_structure_block: String,
}

/// Shared rewrite-from-beats contract (adapters + harness).
pub fn rewrite_from_beats_contract() -> &'static str {
    "Rewrite from the scene beat (at most 1–2 passes). Do not synonym-swap or \
     paraphrase-humanize. Use voice_samples as the on-voice target when present. \
     Soft shelves stay advisory and must not be treated as a fail-closed gate."
}

/// Full rewrite-from-beats prompt for a hard verify fail: contract + hard hits.
pub fn rewrite_from_beats_prompt(report: &AntiSlopReport) -> String {
    let mut lines = vec![
        "## Rewrite-from-beats".to_string(),
        rewrite_from_beats_contract().to_string(),
        format!(
            "Budget: at most {} from-beats pass(es). Then re-lint and surface leftovers.",
            report.rewrite_max_passes.clamp(1, 2)
        ),
        format!(
            "hard_count={} soft_count={} (soft is advisory; do not fail-close on soft).",
            report.hard_count, report.soft_count
        ),
        String::new(),
        "Hard shelves to rewrite from the beat:".to_string(),
    ];
    let hard: Vec<&AntiSlopHit> = report
        .hits
        .iter()
        .filter(|hit| hit.severity == Severity::Hard)
        .collect();
    if hard.is_empty() {
        lines.push("- NONE".to_string());
    } else {
        for hit in hard {
            lines.push(format!(
                "- `{}`: {} — {}",
                hit.shelf_id,
                excerpt_preview(&hit.excerpt),
                hit.rewrite_hint
            ));
        }
    }
    lines.push(String::new());
    lines.push(
        "Residuals after the budget: keep the leftover shelf IDs visible. Do not drop them."
            .to_string(),
    );
    lines.join("\n")
}

/// Hard over-limit hits as warning-level verify findings. Soft is omitted.
pub fn verify_findings(report: &AntiSlopReport) -> Vec<AntiSlopVerifyFinding> {
    report
        .hits
        .iter()
        .filter(|hit| hit.severity == Severity::Hard)
        .map(|hit| AntiSlopVerifyFinding {
            check_type: ANTI_SLOP_CHECK.to_string(),
            severity: "warning".to_string(),
            message: format!(
                "hard shelf `{}` over limit: {} — {}",
                hit.shelf_id,
                excerpt_preview(&hit.excerpt),
                hit.rewrite_hint
            ),
            suggested_action: format!("{}. {}", hit.rewrite_hint, rewrite_from_beats_contract()),
            shelf_id: hit.shelf_id.clone(),
        })
        .collect()
}

/// Dual-persona injection: report plus persona contracts.
pub fn dual_persona_injection(report: &AntiSlopReport) -> DualPersonaAntiSlopInjection {
    DualPersonaAntiSlopInjection {
        report_block: render_report_block(report),
        craft_technician_block: craft_technician_citation_block(),
        literary_critic_structure_block: literary_critic_structure_block(),
    }
}

/// Product lock: `auto_strict` blocks when `hard_count != 0`. Soft-only does not.
pub fn auto_strict_blocks(report: &AntiSlopReport) -> bool {
    report.hard_count > 0
}

/// Rewrite budget for the in-run loop: clamp to 1–2 when verify/revise is on.
pub fn rewrite_attempt_budget(max_revise_attempts: i32) -> u8 {
    match max_revise_attempts {
        1 => 1,
        n if n >= 2 => 2,
        _ => 0,
    }
}

/// Residual hard shelves after a bounded rewrite (re-lint surface).
pub fn residual_hard_ids(report: &AntiSlopReport) -> Vec<String> {
    report
        .hits
        .iter()
        .filter(|hit| hit.severity == Severity::Hard)
        .map(|hit| hit.shelf_id.clone())
        .collect()
}

/// Human residual line for parked verify detail.
pub fn residual_summary(report: &AntiSlopReport) -> String {
    let hard = residual_hard_ids(report);
    if hard.is_empty() {
        "anti_slop residuals: none".to_string()
    } else {
        format!("anti_slop residuals: {}", hard.join(", "))
    }
}

fn render_report_block(report: &AntiSlopReport) -> String {
    let mut lines = vec![
        "## Anti-slop report".to_string(),
        format!(
            "pack={}@{} hard_count={} soft_count={} rewrite_max_passes={}",
            report.pack_id,
            report.pack_version,
            report.hard_count,
            report.soft_count,
            report.rewrite_max_passes
        ),
        "This is a craft report, not an authorship test. Fiction shelves only.".to_string(),
    ];
    if report.hits.is_empty() {
        lines.push("- no shelf hits".to_string());
    } else {
        for hit in &report.hits {
            let sev = match hit.severity {
                Severity::Hard => "hard",
                Severity::Soft => "soft",
            };
            lines.push(format!(
                "- {sev} `{}`: {} — {}",
                hit.shelf_id,
                excerpt_preview(&hit.excerpt),
                hit.rewrite_hint
            ));
        }
    }
    lines.join("\n")
}

fn craft_technician_citation_block() -> String {
    "## Craft Technician — shelf citations\n\
     When a craft concern matches the injected anti-slop report, cite the shelf \
     ID in backticks (example: `emotion_cocktail`). If no shelf applies, write NONE.\n\
     Rewrite advice must be rewrite-from-beats (at most 1–2 passes), not a \
     paraphrase-humanizer. Soft shelves (including `solitary_fade`) stay advisory.\n\
     Do not invent Voices, BLUF, Flesch, or delve-as-tech-gate findings."
        .to_string()
}

fn literary_critic_structure_block() -> String {
    "## Literary Critic — structure\n\
     Judge scene structure as a reader: opening pressure, turn, close, and \
     lived-in place/body when the POV is alone.\n\
     Do not apply BLUF, FAQ endings, Voices/tech buzzlists, Flesch/grade-level, \
     or delve-as-tech-gate. Do not score fiction shelves; the Craft Technician \
     cites those IDs."
        .to_string()
}

fn excerpt_preview(excerpt: &str) -> String {
    let trimmed = excerpt.trim();
    const MAX: usize = 72;
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= MAX {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::antislop::{ScanInput, ShelfPack, scan};

    fn hard_report() -> AntiSlopReport {
        let pack = ShelfPack::load_default().expect("pack");
        scan(
            &pack,
            &ScanInput::fiction(
                "It wasn't anger. It was disappointment.\n\
                 This wasn't a homecoming. It was a reckoning.\n\
                 She felt a mix of relief and dread when the letter came.",
            ),
        )
    }

    fn soft_only_report() -> AntiSlopReport {
        let pack = ShelfPack::load_default().expect("pack");
        scan(
            &pack,
            &ScanInput::fiction(
                "She spent the afternoon thinking about what he'd said.\n\n\
                 Hours passed.\n\n\
                 Later, at the meeting, she told him everything.",
            ),
        )
    }

    #[test]
    fn rewrite_from_beats_prompt_is_not_a_paraphrase_humanizer() {
        let report = hard_report();
        let prompt = rewrite_from_beats_prompt(&report);
        assert!(prompt.contains("Rewrite-from-beats") || prompt.contains("from the scene beat"));
        assert!(prompt.contains("beat"));
        assert!(prompt.contains("1–2") || prompt.contains("1-2") || prompt.contains("at most"));
        assert!(
            prompt.to_lowercase().contains("paraphrase"),
            "must forbid paraphrase-humanizer: {prompt}"
        );
        assert!(
            prompt.contains("do not synonym-swap") || prompt.contains("Do not synonym-swap"),
            "{prompt}"
        );
        assert!(prompt.contains("emotion_cocktail") || prompt.contains("contrast_not_x_but_y"));
        assert!(report.rewrite_max_passes <= 2);
        assert!(!prompt.contains("humanize the sentence"));
    }

    #[test]
    fn verify_findings_are_hard_warnings_only() {
        let hard = hard_report();
        assert!(hard.hard_count >= 1);
        let findings = verify_findings(&hard);
        assert!(!findings.is_empty());
        assert!(
            findings
                .iter()
                .all(|f| f.check_type == ANTI_SLOP_CHECK && f.severity == "warning")
        );
        assert!(findings.iter().all(|f| f.shelf_id != "solitary_fade"));
        assert!(findings.iter().all(|f| f.suggested_action.contains("beat")));

        let soft = soft_only_report();
        assert!(soft.soft_count >= 1);
        assert_eq!(soft.hard_count, 0);
        assert!(
            soft.hits.iter().any(|hit| hit.shelf_id == "solitary_fade"),
            "solitary_fade ID must stay stable"
        );
        assert!(verify_findings(&soft).is_empty());
        assert!(!auto_strict_blocks(&soft));
        assert!(auto_strict_blocks(&hard));
    }

    #[test]
    fn dual_persona_injects_report_citation_and_structure_block() {
        let report = hard_report();
        let injection = dual_persona_injection(&report);
        assert!(injection.report_block.contains("Anti-slop report"));
        assert!(injection.report_block.contains("hard_count="));
        assert!(
            injection.craft_technician_block.contains("shelf")
                && injection.craft_technician_block.contains("NONE")
        );
        assert!(injection.craft_technician_block.contains("solitary_fade"));
        assert!(
            injection
                .literary_critic_structure_block
                .contains("structure")
        );
        assert!(
            injection
                .literary_critic_structure_block
                .contains("opening")
        );
        assert!(injection.literary_critic_structure_block.contains("turn"));
        assert!(injection.literary_critic_structure_block.contains("close"));
        for block in [
            &injection.report_block,
            &injection.craft_technician_block,
            &injection.literary_critic_structure_block,
        ] {
            let lower = block.to_lowercase();
            if lower.contains("flesch") || lower.contains("bluf") || lower.contains("delve") {
                assert!(
                    lower.contains("do not"),
                    "non-ports may be named only as a forbid: {block}"
                );
            }
        }
        assert!(
            injection
                .literary_critic_structure_block
                .to_lowercase()
                .contains("do not apply bluf")
        );
    }

    #[test]
    fn rewrite_attempt_budget_caps_at_one_or_two() {
        assert_eq!(rewrite_attempt_budget(0), 0);
        assert_eq!(rewrite_attempt_budget(1), 1);
        assert_eq!(rewrite_attempt_budget(2), 2);
        assert_eq!(rewrite_attempt_budget(9), 2);
        assert_eq!(rewrite_attempt_budget(-1), 0);
    }

    #[test]
    fn residuals_surface_hard_ids_after_relint() {
        let hard = hard_report();
        let summary = residual_summary(&hard);
        assert!(summary.contains("anti_slop residuals"));
        assert!(summary.contains("emotion_cocktail") || summary.contains("contrast_not_x_but_y"));
        let clean = residual_summary(&AntiSlopReport {
            pack_id: "fiction.default".into(),
            pack_version: "0.1.1".into(),
            domain: "fiction".into(),
            surface: crate::style::antislop::ScanSurface::Fiction,
            skipped: false,
            skip_reason: None,
            hard_count: 0,
            soft_count: 0,
            hits: Vec::new(),
            rewrite_max_passes: 2,
            structural_observations: Vec::new(),
        });
        assert!(clean.contains("none"));
    }
}
