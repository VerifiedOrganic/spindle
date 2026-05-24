//! Phase-4 validator suite (SQLite backend).
//!
//! Ports `crates/spindle-adapters/src/validators.rs` from the SurrealDB
//! reference at 705b835^. The validators (canonical_fact_prose_drift,
//! world_rule_semantic_drift, voice_drift, retcon_reachability, and
//! style_compliance) plug into the `spindle_core::validators::ValidatorRegistry`
//! and are invoked by the service layer through
//! `run_phase_four_validator_checks_for_scenes`.

use spindle_core::models::{CharacterVoiceProfileData, TextByteRange, WorldRuleSeverity};
use spindle_core::style::{StyleDriftSeverity, StyleScanInput};
use spindle_core::validators::{
    SceneSnapshot, SceneValidator, TemporalInterventionSnapshot, ValidatorContext,
    ValidatorFinding, ValidatorRegistry, ValidatorSeverity,
};
use spindle_core::voice::{VoiceDriftCharacter, check_voice_drift};
use spindle_core::world_rules::{ScanRule, scan_prose_for_world_rules};

pub fn phase_four_validator_registry() -> ValidatorRegistry {
    let mut registry = ValidatorRegistry::new();
    registry.register(CanonicalFactConsistencyValidator);
    registry.register(WorldRuleComplianceValidator);
    registry.register(VoiceDriftValidator);
    registry.register(RetconReachabilityValidator);
    registry.register(StyleComplianceValidator);
    registry
}

pub struct CanonicalFactConsistencyValidator;

impl SceneValidator for CanonicalFactConsistencyValidator {
    fn validator_id(&self) -> &'static str {
        "canonical_fact_consistency"
    }

    fn check_type(&self) -> &'static str {
        "canonical_fact_prose_drift"
    }

    fn validate_scene(
        &self,
        scene: &SceneSnapshot,
        context: &ValidatorContext,
    ) -> Result<Vec<ValidatorFinding>, String> {
        let mut findings = Vec::new();
        for fact in &context.canonical_facts {
            if fact.scene_id == scene.scene_id {
                continue;
            }
            if !placement_leq(
                (fact.book_number, fact.chapter_number, 0),
                (scene.book_number, scene.chapter_number, scene.scene_order),
            ) {
                continue;
            }
            if fact.key.trim().is_empty() || fact.value.trim().is_empty() {
                continue;
            }
            let key_hits = find_all_case_insensitive_word(&scene.full_text, &fact.key);
            if key_hits.is_empty() || contains_case_insensitive_word(&scene.full_text, &fact.value)
            {
                continue;
            }
            for range in key_hits.into_iter().take(3) {
                findings.push(ValidatorFinding {
                    check_type: self.check_type(),
                    severity: ValidatorSeverity::Warning,
                    message: format!(
                        "scene references canonical key '{}' but not canonical value '{}'",
                        fact.key, fact.value
                    ),
                    byte_range: Some(range),
                });
            }
        }
        Ok(findings)
    }
}

pub struct WorldRuleComplianceValidator;

impl SceneValidator for WorldRuleComplianceValidator {
    fn validator_id(&self) -> &'static str {
        "world_rule_compliance"
    }

    fn check_type(&self) -> &'static str {
        "world_rule_semantic_drift"
    }

    fn validate_scene(
        &self,
        scene: &SceneSnapshot,
        context: &ValidatorContext,
    ) -> Result<Vec<ValidatorFinding>, String> {
        // Delegate to the shared scanner in spindle-core so the commit gate
        // and check_consistency agree on what counts as a violation.
        let rules: Vec<ScanRule> = context
            .world_rules
            .iter()
            .filter(|rule| match rule.established_in {
                Some((book_number, chapter_number)) => placement_leq(
                    (book_number, chapter_number, 0),
                    (scene.book_number, scene.chapter_number, scene.scene_order),
                ),
                None => true,
            })
            .map(|rule| ScanRule {
                rule_id: rule.rule_id.clone(),
                scan_pattern: rule.scan_pattern.clone(),
                rule_name: rule.rule_name.clone(),
                // Description isn't required for adjacency-based severity
                // assignment; spindle-core's scanner derives severity from
                // prose context, not rule metadata.
                description: String::new(),
            })
            .collect();

        let rule_names: std::collections::BTreeMap<String, String> = context
            .world_rules
            .iter()
            .map(|rule| (rule.rule_id.clone(), rule.rule_name.clone()))
            .collect();

        let hits = scan_prose_for_world_rules(&scene.full_text, &rules);

        let mut findings = Vec::new();
        for hit in hits {
            let severity = match hit.severity {
                WorldRuleSeverity::Likely => ValidatorSeverity::Warning,
                WorldRuleSeverity::Possible => ValidatorSeverity::Info,
            };
            let rule_name = rule_names
                .get(&hit.rule_id)
                .cloned()
                .unwrap_or_else(|| hit.rule_id.clone());
            let message = match hit.severity {
                WorldRuleSeverity::Likely => format!(
                    "scene likely violates world rule '{}'; surrounding prose suggests violation",
                    rule_name
                ),
                WorldRuleSeverity::Possible => format!(
                    "scene references world rule '{}' (no violation context detected)",
                    rule_name
                ),
            };
            findings.push(ValidatorFinding {
                check_type: self.check_type(),
                severity,
                message,
                byte_range: Some(TextByteRange {
                    start: hit.byte_range.start,
                    end: hit.byte_range.end,
                }),
            });
        }
        Ok(findings)
    }
}

pub struct VoiceDriftValidator;

impl SceneValidator for VoiceDriftValidator {
    fn validator_id(&self) -> &'static str {
        "voice_drift"
    }

    fn check_type(&self) -> &'static str {
        "voice_drift"
    }

    fn validate_scene(
        &self,
        scene: &SceneSnapshot,
        context: &ValidatorContext,
    ) -> Result<Vec<ValidatorFinding>, String> {
        // Delegate to spindle-core's speaker-attributed implementation so we
        // do not flag character A's forbidden words against character B's
        // attributed dialogue.
        let mut findings = Vec::new();
        for profile in &context.voice_profiles {
            let character = VoiceDriftCharacter {
                id: profile.character_id.clone(),
                name: profile.character_name.clone(),
            };
            let profile_data = CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: profile.forbidden_words.clone(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            };
            for drift in check_voice_drift(&scene.full_text, &character, &profile_data) {
                findings.push(ValidatorFinding {
                    check_type: self.check_type(),
                    severity: ValidatorSeverity::Warning,
                    message: drift.message,
                    byte_range: drift.byte_range.map(|r| TextByteRange {
                        start: r.start,
                        end: r.end,
                    }),
                });
            }
        }
        Ok(findings)
    }
}

pub struct RetconReachabilityValidator;

impl SceneValidator for RetconReachabilityValidator {
    fn validator_id(&self) -> &'static str {
        "retcon_reachability"
    }

    fn check_type(&self) -> &'static str {
        "retcon_reachability"
    }

    fn validate_scene(
        &self,
        scene: &SceneSnapshot,
        context: &ValidatorContext,
    ) -> Result<Vec<ValidatorFinding>, String> {
        let mut findings = Vec::new();
        for (intervention, terms) in intervention_anchor_terms(context) {
            let Some(_target_position) = intervention_anchor_position(scene, context, intervention)
            else {
                continue;
            };
            for term in terms {
                for range in find_all_case_insensitive_word(&scene.full_text, &term)
                    .into_iter()
                    .take(3)
                {
                    findings.push(ValidatorFinding {
                        check_type: self.check_type(),
                        severity: ValidatorSeverity::Info,
                        message: format!(
                            "scene references temporal intervention '{}' before its reachable timeline anchor",
                            intervention.title
                        ),
                        byte_range: Some(range),
                    });
                }
            }
        }
        Ok(findings)
    }
}

/// Genre-voice compliance. Scans prose against the project style contract
/// (reader contract style_notes, `style` world rules, narrator voice) and flags
/// literary-contemplative drift on a genre that asks otherwise — most notably a
/// contemplative chapter ending where the narrator voice wants a hook. Coarse,
/// high-precision heuristics; nuanced judgement is left to the review persona.
///
/// Unlike the `save_scene_draft` gate, this validator has whole-chapter context
/// (`context.scenes`), so it can determine whether a scene is the chapter's
/// last and run the ending-beat check. It does not see the author-declared tone
/// string (that lives only on the save input), so that check stays on save.
pub struct StyleComplianceValidator;

impl SceneValidator for StyleComplianceValidator {
    fn validator_id(&self) -> &'static str {
        "style_compliance"
    }

    fn check_type(&self) -> &'static str {
        "style_compliance"
    }

    fn validate_scene(
        &self,
        scene: &SceneSnapshot,
        context: &ValidatorContext,
    ) -> Result<Vec<ValidatorFinding>, String> {
        let Some(directive) = context.style_directive.as_ref() else {
            return Ok(Vec::new());
        };
        if directive.is_empty() {
            return Ok(Vec::new());
        }

        // The scene closes its chapter when no other scene in the same chapter
        // sits after it.
        let is_chapter_end = !context.scenes.iter().any(|other| {
            other.scene_id != scene.scene_id
                && other.book_number == scene.book_number
                && other.chapter_number == scene.chapter_number
                && other.scene_order > scene.scene_order
        });

        let hits = directive.scan(&StyleScanInput {
            prose: &scene.full_text,
            declared_tone: None,
            is_chapter_end,
        });

        Ok(hits
            .into_iter()
            .map(|hit| ValidatorFinding {
                check_type: self.check_type(),
                severity: match hit.severity {
                    StyleDriftSeverity::Warning => ValidatorSeverity::Warning,
                    StyleDriftSeverity::Info => ValidatorSeverity::Info,
                },
                message: hit.message,
                byte_range: None,
            })
            .collect())
    }
}

fn intervention_anchor_position(
    scene: &SceneSnapshot,
    context: &ValidatorContext,
    intervention: &TemporalInterventionSnapshot,
) -> Option<(i32, i32, i32)> {
    let events_by_id = context
        .timeline_events
        .iter()
        .map(|event| (&event.event_id, event))
        .collect::<std::collections::BTreeMap<_, _>>();
    let target = intervention
        .target_event_id
        .as_ref()
        .and_then(|id| events_by_id.get(id))
        .map(|event| (event.book_number, event.chapter_number, event.scene_order));
    let source = intervention
        .source_event_id
        .as_ref()
        .and_then(|id| events_by_id.get(id))
        .map(|event| (event.book_number, event.chapter_number, event.scene_order));
    target.or(source).filter(|position| {
        !placement_leq(
            *position,
            (scene.book_number, scene.chapter_number, scene.scene_order),
        )
    })
}

fn intervention_anchor_terms(
    context: &ValidatorContext,
) -> Vec<(&TemporalInterventionSnapshot, Vec<String>)> {
    let events_by_id = context
        .timeline_events
        .iter()
        .map(|event| (&event.event_id, event))
        .collect::<std::collections::BTreeMap<_, _>>();

    context
        .temporal_interventions
        .iter()
        .map(|intervention| {
            let mut terms = Vec::new();
            if !intervention.title.trim().is_empty() {
                terms.push(intervention.title.clone());
            }
            if let Some(event) = intervention
                .source_event_id
                .as_ref()
                .and_then(|id| events_by_id.get(id))
                && !event.title.trim().is_empty()
            {
                terms.push(event.title.clone());
            }
            if let Some(event) = intervention
                .target_event_id
                .as_ref()
                .and_then(|id| events_by_id.get(id))
                && !event.title.trim().is_empty()
            {
                terms.push(event.title.clone());
            }
            (intervention, terms)
        })
        .collect()
}

fn find_all_case_insensitive_word(haystack: &str, needle: &str) -> Vec<TextByteRange> {
    let needle = needle.trim();
    if haystack.is_empty() || needle.is_empty() {
        return Vec::new();
    }

    let needle_len = needle.len();
    let mut results = Vec::new();
    for (start, _) in haystack.char_indices() {
        let Some(end) = start.checked_add(needle_len) else {
            break;
        };
        if end > haystack.len() {
            break;
        }
        if !haystack.is_char_boundary(end) {
            continue;
        }
        let segment = &haystack[start..end];
        if segment.eq_ignore_ascii_case(needle) && has_word_boundaries(haystack, start, end) {
            results.push(TextByteRange { start, end });
        }
    }
    results
}

fn placement_leq(lhs: (i32, i32, i32), rhs: (i32, i32, i32)) -> bool {
    lhs <= rhs
}

fn contains_case_insensitive_word(haystack: &str, needle: &str) -> bool {
    !find_all_case_insensitive_word(haystack, needle).is_empty()
}

fn has_word_boundaries(haystack: &str, start: usize, end: usize) -> bool {
    let prev_is_alnum = haystack[..start]
        .chars()
        .next_back()
        .map(|c| c.is_alphanumeric())
        .unwrap_or(false);
    if prev_is_alnum {
        return false;
    }
    let next_is_alnum = haystack[end..]
        .chars()
        .next()
        .map(|c| c.is_alphanumeric())
        .unwrap_or(false);
    !next_is_alnum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_all_case_insensitive_word_uses_boundaries() {
        let haystack = "The siren grew louder while the sir warned us.";
        let matches = find_all_case_insensitive_word(haystack, "sir");
        assert_eq!(matches.len(), 1);
        assert_eq!(&haystack[matches[0].start..matches[0].end], "sir");
    }

    #[test]
    fn registry_registers_all_phase_four_validators() {
        let registry = phase_four_validator_registry();
        assert_eq!(registry.len(), 5);
    }
}
