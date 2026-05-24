use crate::models::ContextFormat;
use std::fmt;

/// Kind of section in a [`ContextBundle`].
///
/// [`SectionKind::HardConstraint`] sections are never trimmed. If hard
/// constraints alone exceed the budget, `enforce_budget` returns an error.
///
/// [`SectionKind::Supplementary(priority)`] sections are trimmed in ascending
/// priority order — lower values are trimmed first. Priority 0 is the first to
/// be dropped; higher values are kept longer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionKind {
    HardConstraint,
    Supplementary(u8),
}

/// A budget-trimmable section of context.
///
/// Implementations describe how to render the section in markdown and JSON,
/// estimate token cost under a given format, and clear their content when
/// trimmed. The [`SectionKind`] determines trimming behaviour:
/// hard-constraint sections are never dropped, while supplementary sections
/// are trimmed in ascending priority order.
pub trait Section: Send + Sync {
    fn kind(&self) -> SectionKind;
    fn id(&self) -> &str;
    fn is_hard_constraint(&self) -> bool {
        matches!(self.kind(), SectionKind::HardConstraint)
    }
    fn is_empty(&self) -> bool;
    fn token_estimate(&self, format: ContextFormat) -> usize;
    fn to_markdown(&self) -> String;
    fn to_json_value(&self) -> serde_json::Value;
    fn clear_content(&mut self);
}

#[derive(Debug, Clone)]
pub struct BundleError {
    pub message: String,
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for BundleError {}

#[derive(Debug, Clone)]
pub struct BudgetReport {
    pub estimated_tokens: usize,
    pub budget_tokens: usize,
    pub truncated_section_ids: Vec<String>,
}

impl BudgetReport {
    pub fn is_over_budget(&self) -> bool {
        self.estimated_tokens > self.budget_tokens
    }
}

pub struct ContextBundle {
    sections: Vec<Box<dyn Section>>,
    format: ContextFormat,
    budget_tokens: Option<usize>,
    budget_enforced: bool,
}

impl fmt::Debug for ContextBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextBundle")
            .field("format", &self.format)
            .field("budget_tokens", &self.budget_tokens)
            .field("section_count", &self.sections.len())
            .field("budget_enforced", &self.budget_enforced)
            .finish()
    }
}

impl ContextBundle {
    pub fn new(format: ContextFormat) -> Self {
        Self {
            sections: Vec::new(),
            format,
            budget_tokens: None,
            budget_enforced: false,
        }
    }

    pub fn with_budget(mut self, budget: usize) -> Self {
        self.budget_tokens = Some(budget);
        self.budget_enforced = false;
        self
    }

    pub fn add_section(mut self, section: Box<dyn Section>) -> Self {
        self.sections.push(section);
        self.budget_enforced = false;
        self
    }

    pub fn push_section(&mut self, section: Box<dyn Section>) {
        self.sections.push(section);
        self.budget_enforced = false;
    }

    pub fn sections(&self) -> &[Box<dyn Section>] {
        &self.sections
    }

    fn hard_constraints(&self) -> Vec<&dyn Section> {
        self.sections
            .iter()
            .map(Box::as_ref)
            .filter(|s| s.is_hard_constraint())
            .collect()
    }

    fn supplementary_indices_sorted_ascending(&self) -> Vec<usize> {
        let mut supp: Vec<(usize, u8)> = self
            .sections
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s.kind() {
                SectionKind::Supplementary(priority) => Some((i, priority)),
                SectionKind::HardConstraint => None,
            })
            .collect();
        supp.sort_by_key(|(_, p)| *p);
        supp.into_iter().map(|(i, _)| i).collect()
    }

    fn estimate_total_tokens(&self) -> usize {
        self.sections
            .iter()
            .map(|s| s.token_estimate(self.format))
            .sum()
    }

    fn estimate_hard_constraint_tokens(&self) -> usize {
        self.hard_constraints()
            .iter()
            .map(|s| s.token_estimate(self.format))
            .sum()
    }

    pub fn enforce_budget(&mut self) -> Result<BudgetReport, BundleError> {
        let budget = match self.budget_tokens {
            Some(b) => b,
            None => {
                self.budget_enforced = true;
                return Ok(BudgetReport {
                    estimated_tokens: self.estimate_total_tokens(),
                    budget_tokens: usize::MAX,
                    truncated_section_ids: Vec::new(),
                });
            }
        };

        let hard_cost = self.estimate_hard_constraint_tokens();
        if hard_cost > budget {
            return Err(BundleError {
                message: format!(
                    "budget_tokens ({budget}) too small to fit hard constraints \
                     (estimated {hard_cost} tokens). \
                     Increase budget_tokens or reduce world rules."
                ),
            });
        }

        let mut estimated = self.estimate_total_tokens();
        if estimated <= budget {
            self.budget_enforced = true;
            return Ok(BudgetReport {
                estimated_tokens: estimated,
                budget_tokens: budget,
                truncated_section_ids: Vec::new(),
            });
        }

        let mut truncated = Vec::new();
        for idx in self.supplementary_indices_sorted_ascending() {
            if estimated <= budget {
                break;
            }
            let id = self.sections[idx].id().to_string();
            let cost_before = self.sections[idx].token_estimate(self.format);
            self.sections[idx].clear_content();
            let cost_after = self.sections[idx].token_estimate(self.format);
            estimated = estimated
                .saturating_sub(cost_before)
                .saturating_add(cost_after);
            truncated.push(id);
        }

        self.budget_enforced = true;
        Ok(BudgetReport {
            estimated_tokens: estimated,
            budget_tokens: budget,
            truncated_section_ids: truncated,
        })
    }

    fn render_markdown(&self) -> String {
        let mut parts = Vec::new();
        for section in &self.sections {
            if section.is_empty() {
                continue;
            }
            let md = section.to_markdown();
            if !md.is_empty() {
                parts.push(md);
            }
        }
        parts.join("\n\n")
    }

    fn render_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for section in &self.sections {
            if section.is_empty() {
                continue;
            }
            map.insert(section.id().to_string(), section.to_json_value());
        }
        serde_json::Value::Object(map)
    }

    pub fn render(&mut self) -> Result<String, BundleError> {
        if !self.budget_enforced {
            self.enforce_budget()?;
        }
        Ok(match self.format {
            ContextFormat::Markdown => self.render_markdown(),
            ContextFormat::Json => {
                let val = self.render_json();
                serde_json::to_string_pretty(&val).unwrap_or_default()
            }
        })
    }
}

pub fn estimate_text_tokens(text: &str) -> usize {
    text.chars().count() / 4
}

pub fn estimate_json_tokens(value: &serde_json::Value) -> usize {
    value.to_string().chars().count() / 4
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SimpleSection {
        id: String,
        kind: SectionKind,
        markdown: String,
        json: serde_json::Value,
        override_tokens: Option<usize>,
    }

    impl SimpleSection {
        fn hard(id: &str, markdown: &str, json: serde_json::Value) -> Self {
            Self {
                id: id.to_string(),
                kind: SectionKind::HardConstraint,
                markdown: markdown.to_string(),
                json,
                override_tokens: None,
            }
        }

        fn supplementary(id: &str, priority: u8, markdown: &str, json: serde_json::Value) -> Self {
            Self {
                id: id.to_string(),
                kind: SectionKind::Supplementary(priority),
                markdown: markdown.to_string(),
                json,
                override_tokens: None,
            }
        }

        fn with_tokens(mut self, tokens: usize) -> Self {
            self.override_tokens = Some(tokens);
            self
        }
    }

    impl Section for SimpleSection {
        fn kind(&self) -> SectionKind {
            self.kind
        }

        fn id(&self) -> &str {
            &self.id
        }

        fn is_empty(&self) -> bool {
            self.markdown.is_empty() && self.json.is_null()
        }

        fn token_estimate(&self, _format: ContextFormat) -> usize {
            self.override_tokens
                .unwrap_or_else(|| estimate_text_tokens(&self.markdown))
        }

        fn to_markdown(&self) -> String {
            self.markdown.clone()
        }

        fn to_json_value(&self) -> serde_json::Value {
            self.json.clone()
        }

        fn clear_content(&mut self) {
            self.markdown = String::new();
            self.json = serde_json::Value::Null;
            self.override_tokens = Some(0);
        }
    }

    fn make_section(id: &str, tokens: usize, kind: SectionKind) -> Box<dyn Section> {
        let padding = "x".repeat(tokens * 4);
        let md = format!("## {id}\n{padding}");
        let json = serde_json::json!({ id: padding });
        Box::new(
            match kind {
                SectionKind::HardConstraint => SimpleSection::hard(id, &md, json),
                SectionKind::Supplementary(p) => SimpleSection::supplementary(id, p, &md, json),
            }
            .with_tokens(tokens),
        )
    }

    #[test]
    fn hard_constraints_are_preserved_under_budget() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(10000);
        bundle.push_section(make_section("rules", 50, SectionKind::HardConstraint));
        bundle.push_section(make_section("knowledge", 30, SectionKind::Supplementary(0)));

        let report = bundle.enforce_budget().unwrap();
        assert!(!report.is_over_budget());
        assert!(report.truncated_section_ids.is_empty());
        assert!(bundle.sections()[0].to_markdown().contains("rules"));
        assert!(bundle.sections()[1].to_markdown().contains("knowledge"));
    }

    #[test]
    fn supplementary_sections_are_trimmed_when_over_budget() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(200);
        bundle.push_section(make_section("rules", 50, SectionKind::HardConstraint));
        bundle.push_section(make_section(
            "knowledge",
            200,
            SectionKind::Supplementary(1),
        ));
        bundle.push_section(make_section("timeline", 200, SectionKind::Supplementary(2)));

        let report = bundle.enforce_budget().unwrap();
        assert!(!report.is_over_budget());
        assert!(
            report
                .truncated_section_ids
                .contains(&"knowledge".to_string())
        );
        assert!(bundle.sections()[0].to_markdown().contains("rules"));
        assert!(bundle.sections()[1].is_empty());
    }

    #[test]
    fn exact_budget_fits() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(300);
        bundle.push_section(make_section("rules", 100, SectionKind::HardConstraint));
        bundle.push_section(make_section("extra", 200, SectionKind::Supplementary(0)));

        let report = bundle.enforce_budget().unwrap();
        assert!(!report.is_over_budget());
        assert!(report.truncated_section_ids.is_empty());
    }

    #[test]
    fn over_budget_hard_constraint_returns_error() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(50);
        bundle.push_section(make_section("rules", 500, SectionKind::HardConstraint));

        let result = bundle.enforce_budget();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("too small to fit hard constraints"));
    }

    #[test]
    fn markdown_and_json_output_parity() {
        let mut bundle_md = ContextBundle::new(ContextFormat::Markdown).with_budget(10000);
        bundle_md.push_section(make_section("rules", 50, SectionKind::HardConstraint));
        bundle_md.push_section(make_section("knowledge", 30, SectionKind::Supplementary(0)));

        let md_output = bundle_md.render().unwrap();
        assert!(md_output.contains("rules"));
        assert!(md_output.contains("knowledge"));

        let mut bundle_json = ContextBundle::new(ContextFormat::Json).with_budget(10000);
        bundle_json.push_section(make_section("rules", 50, SectionKind::HardConstraint));
        bundle_json.push_section(make_section("knowledge", 30, SectionKind::Supplementary(0)));

        let json_output = bundle_json.render().unwrap();
        assert!(json_output.contains("rules"));
        assert!(json_output.contains("knowledge"));
    }

    #[test]
    fn cleared_sections_omitted_from_both_formats() {
        let mut bundle_md = ContextBundle::new(ContextFormat::Markdown).with_budget(200);
        bundle_md.push_section(make_section("rules", 50, SectionKind::HardConstraint));
        bundle_md.push_section(make_section(
            "knowledge",
            200,
            SectionKind::Supplementary(1),
        ));
        bundle_md.push_section(make_section("timeline", 200, SectionKind::Supplementary(2)));

        let report = bundle_md.enforce_budget().unwrap();
        assert!(!report.truncated_section_ids.is_empty());

        let md = bundle_md.render().unwrap();
        assert!(md.contains("rules"));
        assert!(!md.contains("knowledge"));

        let mut bundle_json = ContextBundle::new(ContextFormat::Json).with_budget(200);
        bundle_json.push_section(make_section("rules", 50, SectionKind::HardConstraint));
        bundle_json.push_section(make_section(
            "knowledge",
            200,
            SectionKind::Supplementary(1),
        ));
        bundle_json.push_section(make_section("timeline", 200, SectionKind::Supplementary(2)));
        bundle_json.enforce_budget().unwrap();

        let json_str = bundle_json.render().unwrap();
        assert!(json_str.contains("rules"));
        assert!(!json_str.contains("knowledge"));
    }

    #[test]
    fn lower_numeric_priority_trimmed_first() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(200);
        bundle.push_section(make_section("rules", 100, SectionKind::HardConstraint));
        bundle.push_section(make_section("p0", 200, SectionKind::Supplementary(0)));
        bundle.push_section(make_section("p1", 200, SectionKind::Supplementary(1)));
        bundle.push_section(make_section("p2", 200, SectionKind::Supplementary(2)));

        let report = bundle.enforce_budget().unwrap();
        assert_eq!(report.truncated_section_ids[0], "p0");
        assert!(bundle.sections()[0].to_markdown().contains("rules"));
    }

    #[test]
    fn budget_trims_until_under_then_stops() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(500);
        bundle.push_section(make_section("hard", 100, SectionKind::HardConstraint));
        bundle.push_section(make_section("s1", 300, SectionKind::Supplementary(0)));
        bundle.push_section(make_section("s2", 300, SectionKind::Supplementary(1)));
        bundle.push_section(make_section("s3", 100, SectionKind::Supplementary(2)));

        let report = bundle.enforce_budget().unwrap();
        assert!(!report.is_over_budget());
        assert_eq!(report.truncated_section_ids.len(), 1);
        assert_eq!(report.truncated_section_ids[0], "s1");
    }

    #[test]
    fn error_when_budget_zero_and_hard_constraints_present() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(0);
        bundle.push_section(make_section("rules", 50, SectionKind::HardConstraint));
        let result = bundle.enforce_budget();
        assert!(result.is_err());
    }

    #[test]
    fn empty_bundle_enforces_ok() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(100);
        let report = bundle.enforce_budget().unwrap();
        assert!(!report.is_over_budget());
        assert!(report.truncated_section_ids.is_empty());
    }

    #[test]
    fn no_budget_means_no_enforcement() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown);
        bundle.push_section(make_section("rules", 50, SectionKind::HardConstraint));
        bundle.push_section(make_section(
            "knowledge",
            500,
            SectionKind::Supplementary(0),
        ));

        let report = bundle.enforce_budget().unwrap();
        assert!(!report.is_over_budget());
        assert!(report.truncated_section_ids.is_empty());
        assert!(bundle.sections()[0].to_markdown().contains("rules"));
        assert!(bundle.sections()[1].to_markdown().contains("knowledge"));
    }

    #[test]
    fn render_enforces_budget_automatically() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(200);
        bundle.push_section(make_section("rules", 50, SectionKind::HardConstraint));
        bundle.push_section(make_section(
            "knowledge",
            500,
            SectionKind::Supplementary(0),
        ));

        let output = bundle.render().unwrap();
        assert!(output.contains("rules"));
        assert!(!output.contains("knowledge"));
    }

    #[test]
    fn render_returns_error_when_hard_constraints_exceed_budget() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(50);
        bundle.push_section(make_section("rules", 500, SectionKind::HardConstraint));

        let result = bundle.render();
        assert!(result.is_err());
    }

    #[test]
    fn render_after_explicit_enforce_budget_skips_double_enforcement() {
        let mut bundle = ContextBundle::new(ContextFormat::Markdown).with_budget(10000);
        bundle.push_section(make_section("rules", 50, SectionKind::HardConstraint));
        bundle.push_section(make_section("knowledge", 30, SectionKind::Supplementary(0)));
        bundle.enforce_budget().unwrap();

        let output = bundle.render().unwrap();
        assert!(output.contains("rules"));
        assert!(output.contains("knowledge"));
    }
}
