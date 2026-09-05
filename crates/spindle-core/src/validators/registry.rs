use crate::models::TextByteRange;

#[derive(Debug, Clone)]
pub struct SceneSnapshot {
    pub scene_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub full_text: String,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct CanonicalFactSnapshot {
    /// None for planned-and-pending facts registered before their scene
    /// exists (V0038); such a fact never matches the establishing-scene skip
    /// and applies from its placement onward.
    pub scene_id: Option<String>,
    pub book_number: i32,
    pub chapter_number: i32,
    pub fact_type: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct WorldRuleSnapshot {
    pub rule_id: String,
    pub rule_name: String,
    pub scan_pattern: Option<String>,
    pub established_in: Option<(i32, i32)>,
    /// Free-form classification from the world_rule row; the
    /// world_rule_semantic_drift validator uses it (with the rule name and
    /// description) to scope secrecy-class rules to dialogue spans.
    pub rule_type: String,
    /// Carried so secrecy-class classification agrees with the commit-gate
    /// scan path, which classifies on the full rule description.
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct CharacterVoiceProfileSnapshot {
    pub character_id: String,
    pub character_name: String,
    pub forbidden_words: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TimelineEventSnapshot {
    pub event_id: String,
    pub title: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
}

#[derive(Debug, Clone)]
pub struct TemporalInterventionSnapshot {
    pub intervention_id: String,
    pub title: String,
    pub source_event_id: Option<String>,
    pub target_event_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValidatorContext {
    pub project_id: String,
    pub branch_id: String,
    pub scenes: Vec<SceneSnapshot>,
    pub canonical_facts: Vec<CanonicalFactSnapshot>,
    pub world_rules: Vec<WorldRuleSnapshot>,
    pub voice_profiles: Vec<CharacterVoiceProfileSnapshot>,
    pub timeline_events: Vec<TimelineEventSnapshot>,
    pub temporal_interventions: Vec<TemporalInterventionSnapshot>,
    /// Project style contract, for the `style_compliance` validator. `None`
    /// when the project has no style signal (then the validator is a no-op).
    pub style_directive: Option<crate::style::StyleDirective>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidatorSeverity {
    Error,
    Warning,
    Info,
}

impl ValidatorSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ValidatorFinding {
    pub check_type: &'static str,
    pub severity: ValidatorSeverity,
    pub message: String,
    pub byte_range: Option<TextByteRange>,
}

pub trait SceneValidator: Send + Sync {
    fn validator_id(&self) -> &'static str;
    fn check_type(&self) -> &'static str;
    fn validate_scene(
        &self,
        scene: &SceneSnapshot,
        context: &ValidatorContext,
    ) -> Result<Vec<ValidatorFinding>, String>;
}

pub struct ValidatorRegistry {
    validators: Vec<Box<dyn SceneValidator>>,
}

impl ValidatorRegistry {
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }

    pub fn register<V>(&mut self, validator: V)
    where
        V: SceneValidator + 'static,
    {
        self.validators.push(Box::new(validator));
    }

    pub fn len(&self) -> usize {
        self.validators.len()
    }

    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
    }

    pub fn validate_scene(
        &self,
        scene: &SceneSnapshot,
        context: &ValidatorContext,
    ) -> Result<Vec<ValidatorFinding>, String> {
        let mut findings = Vec::new();
        for validator in &self.validators {
            match validator.validate_scene(scene, context) {
                Ok(mut validator_findings) => findings.append(&mut validator_findings),
                Err(error) => {
                    return Err(format!(
                        "validator '{}' failed: {error}",
                        validator.validator_id()
                    ));
                }
            }
        }
        Ok(findings)
    }
}

impl Default for ValidatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
