use crate::subject::SubjectTable;
use crate::subject_snapshot::SubjectSnapshot;
use crate::subject_snapshot::{
    CharacterStateSummary as SnapshotCharacterStateSummary, SceneAppearanceSummary,
    VoiceProfileSummary,
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, JsonSchema, PartialEq, Eq, Default)]
pub enum ContentRating {
    #[default]
    General,
    Teen,
    Mature,
    Explicit,
}

impl ContentRating {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Teen => "teen",
            Self::Mature => "mature",
            Self::Explicit => "explicit",
        }
    }

    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Teen => "Teen",
            Self::Mature => "Mature",
            Self::Explicit => "Explicit",
        }
    }
}

impl Serialize for ContentRating {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ContentRating {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.to_ascii_lowercase().as_str() {
            "general" => Ok(Self::General),
            "teen" => Ok(Self::Teen),
            "mature" => Ok(Self::Mature),
            "explicit" => Ok(Self::Explicit),
            _ => Err(serde::de::Error::custom(format!(
                "invalid content rating: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReaderContract {
    pub promise: String,
    #[serde(default)]
    pub style_notes: Vec<String>,
    #[serde(default)]
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EstablishedIn {
    pub book_number: i32,
    pub chapter_number: i32,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FlexRange {
    pub low: Option<String>,
    pub high: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CharacterVoiceProfileData {
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub vocabulary: Vec<String>,
    #[serde(default)]
    pub sentence_structure: Vec<String>,
    #[serde(default)]
    pub tics: Vec<String>,
    #[serde(default)]
    pub forbidden_words: Vec<String>,
    #[serde(default)]
    pub example_lines: Vec<String>,
    #[serde(default)]
    pub established_in_scene_id: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CharacterEmotionalProfileData {
    #[serde(default)]
    pub base_emotions: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub suppressed: Vec<String>,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub defense_mechanisms: Vec<String>,
    pub flex_range: Option<FlexRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CharacterStatePatch {
    #[serde(default)]
    pub emotional_state: BTreeMap<String, serde_json::Value>,
    pub goals: Option<Vec<String>>,
    pub status: Option<Vec<String>>,
    pub notes: Option<Vec<String>>,
    pub source_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct WorldStateInput {
    pub controlling_faction: Option<String>,
    pub status: Option<String>,
    pub prosperity: Option<String>,
    pub stability: Option<String>,
    pub threat_level: Option<String>,
    #[serde(default)]
    pub sensory_details: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoryPlacement {
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: Option<i32>,
    pub note: Option<String>,
}

/// In-world placement of a scene or event on the project's story clock. Every
/// field is optional: a project that never declares story-time leaves them unset
/// and behaves exactly as before. Distinct from [`StoryPlacement`], which is the
/// structural (manuscript) position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct StoryClock {
    /// In-world day index from the project epoch (monotonic, spans books).
    pub day_index: Option<i64>,
    /// Minutes from midnight, in `0..(hours_per_day * 60)`.
    pub time_of_day: Option<i32>,
    /// In-world span of the scene/event, in days.
    pub duration_days: Option<f64>,
    /// Display/tolerance granularity: `minute|hour|day|week|month|year`.
    pub precision: Option<String>,
}

/// One month in an invented calendar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CalendarMonth {
    pub name: String,
    pub days: i32,
}

/// Per-project calendar definition mapping the abstract day index to/from a
/// human-facing date. Supports non-24h days and fully invented month/week
/// schemes, so an invented fantasy calendar is handled identically to Gregorian.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CalendarDef {
    pub days_per_week: i32,
    /// Hours in an in-world day (typically 24); governs the `time_of_day` range
    /// and the total-order index so non-24h calendars still order correctly.
    pub hours_per_day: i32,
    #[serde(default)]
    pub week_day_names: Vec<String>,
    #[serde(default)]
    pub months: Vec<CalendarMonth>,
    pub days_per_year: i32,
    #[serde(default)]
    pub epoch_label: Option<String>,
}

impl CalendarDef {
    /// Minutes in one in-world day. Folds `(day_index, time_of_day)` into a single
    /// total-order index.
    pub fn minutes_per_day(&self) -> i64 {
        self.hours_per_day.max(0) as i64 * 60
    }

    /// Validate the calendar's internal consistency.
    pub fn validate(&self) -> Result<(), String> {
        if self.days_per_week < 1 {
            return Err(format!(
                "days_per_week must be >= 1 (got {})",
                self.days_per_week
            ));
        }
        if self.hours_per_day < 1 {
            return Err(format!(
                "hours_per_day must be >= 1 (got {})",
                self.hours_per_day
            ));
        }
        if self.days_per_year < 1 {
            return Err(format!(
                "days_per_year must be >= 1 (got {})",
                self.days_per_year
            ));
        }
        if !self.months.is_empty() {
            for month in &self.months {
                if month.days < 1 {
                    return Err(format!("calendar month '{}' must have >= 1 day", month.name));
                }
            }
            let total: i32 = self.months.iter().map(|month| month.days).sum();
            if total != self.days_per_year {
                return Err(format!(
                    "calendar months sum to {total} days but days_per_year is {}",
                    self.days_per_year
                ));
            }
        }
        if !self.week_day_names.is_empty()
            && self.week_day_names.len() as i32 != self.days_per_week
        {
            return Err(format!(
                "week_day_names has {} entries but days_per_week is {}",
                self.week_day_names.len(),
                self.days_per_week
            ));
        }
        Ok(())
    }
}

impl StoryClock {
    /// Fold this clock into a single total-order index (in-world minutes from the
    /// epoch) under `calendar`. Returns None when no `day_index` is set.
    pub fn total_index(&self, calendar: &CalendarDef) -> Option<i64> {
        let day_index = self.day_index?;
        Some(day_index * calendar.minutes_per_day() + self.time_of_day.unwrap_or(0) as i64)
    }

    /// Validate this clock against `calendar`.
    pub fn validate(&self, calendar: &CalendarDef) -> Result<(), String> {
        if let Some(day_index) = self.day_index
            && day_index < 0
        {
            return Err(format!("day_index must be >= 0 (got {day_index})"));
        }
        if let Some(duration) = self.duration_days
            && duration < 0.0
        {
            return Err(format!("duration_days must be >= 0 (got {duration})"));
        }
        if let Some(time_of_day) = self.time_of_day {
            let minutes_per_day = calendar.minutes_per_day();
            if time_of_day < 0 || time_of_day as i64 >= minutes_per_day {
                return Err(format!(
                    "time_of_day must be in 0..{minutes_per_day} (got {time_of_day})"
                ));
            }
        }
        if let Some(precision) = self.precision.as_deref()
            && !matches!(
                precision,
                "minute" | "hour" | "day" | "week" | "month" | "year"
            )
        {
            return Err(format!("unknown precision '{precision}'"));
        }
        Ok(())
    }
}

// =============================================================================
// Quantity-continuity layer (V0020): per-project quantity schemes + stamped
// per-subject quantity state. Money is the first vertical; the primitive is
// generic (LitRPG/cultivation stats, reputation reuse it). Additive and
// optional — a project that declares no scheme behaves exactly as before.
// =============================================================================

/// A denomination within a measure's currency (e.g. a gold piece is worth 100
/// of the base unit). Keeps multi-denomination currencies internally consistent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QuantityDenomination {
    pub name: String,
    /// How many base (smallest) units one of this denomination is worth.
    pub per_base: i64,
}

/// An ordered band/tier within a measure (destitute < comfortable < wealthy, or
/// Bronze < Silver < Gold). Order is the band's position in
/// [`QuantityScheme::bands`]; an optional `lower_bound` ties bands to amounts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QuantityBand {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lower_bound: Option<i64>,
}

/// Per-project, per-measure quantity scheme: the denominations and ordered bands
/// a measure's values validate against. Band-primary by design; amounts are an
/// optional refinement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub struct QuantityScheme {
    pub measure: String,
    #[serde(default)]
    pub denominations: Vec<QuantityDenomination>,
    #[serde(default)]
    pub bands: Vec<QuantityBand>,
    /// How many ordered bands one stamp may cross without an explicit
    /// `change_reason` (defaults to 1 when unset). Consumed by the future
    /// `QuantityDrift` validator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_band_jump: Option<i32>,
}

impl QuantityScheme {
    /// 0-based ordinal of `band` within this scheme's ordered bands, if declared.
    pub fn band_ordinal(&self, band: &str) -> Option<usize> {
        self.bands.iter().position(|b| b.name == band)
    }

    /// Number of ordered bands one stamp may cross without an explicit
    /// `change_reason` (1 when unset).
    pub fn max_band_jump_or_default(&self) -> i32 {
        self.max_band_jump.unwrap_or(1)
    }

    /// Unsigned number of ordered tiers between two declared band names. `None`
    /// when either band is not declared in this scheme.
    pub fn band_jump(&self, from: &str, to: &str) -> Option<i32> {
        let from = self.band_ordinal(from)? as i32;
        let to = self.band_ordinal(to)? as i32;
        Some((from - to).abs())
    }

    /// Convert `amount` of `denomination` into base (smallest-unit) value, if the
    /// denomination is declared in this scheme. Enables cross-denomination price
    /// consistency and affordability checks.
    pub fn amount_in_base(&self, amount: f64, denomination: &str) -> Option<f64> {
        self.denominations
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(denomination))
            .map(|d| amount * d.per_base as f64)
    }

    /// Validate the scheme's internal consistency.
    pub fn validate(&self) -> Result<(), String> {
        if self.measure.trim().is_empty() {
            return Err("measure must not be empty".to_string());
        }
        for denom in &self.denominations {
            if denom.name.trim().is_empty() {
                return Err("denomination name must not be empty".to_string());
            }
            if denom.per_base < 1 {
                return Err(format!(
                    "denomination '{}' per_base must be >= 1 (got {})",
                    denom.name, denom.per_base
                ));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut last_bound: Option<i64> = None;
        for band in &self.bands {
            if band.name.trim().is_empty() {
                return Err("band name must not be empty".to_string());
            }
            if !seen.insert(band.name.as_str()) {
                return Err(format!("duplicate band name '{}'", band.name));
            }
            if let Some(bound) = band.lower_bound {
                if let Some(prev) = last_bound
                    && bound <= prev
                {
                    return Err(format!(
                        "band '{}' lower_bound {bound} must exceed the previous band's {prev}",
                        band.name
                    ));
                }
                last_bound = Some(bound);
            }
        }
        if let Some(jump) = self.max_band_jump
            && jump < 1
        {
            return Err(format!("max_band_jump must be >= 1 (got {jump})"));
        }
        Ok(())
    }
}

/// A stamped quantity reading for a subject's measure at a story position.
/// Append-only; `band` is the primary signal and `amount`/`unit` an optional
/// refinement. A `change_reason` marks a deliberate large jump as legitimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct QuantityState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_reason: Option<String>,
}

impl QuantityState {
    /// Validate this reading against its scheme when one is declared. A declared
    /// band must be one the scheme lists; a negative amount is always rejected.
    pub fn validate(&self, scheme: Option<&QuantityScheme>) -> Result<(), String> {
        if let Some(amount) = self.amount
            && amount < 0.0
        {
            return Err(format!("amount must be >= 0 (got {amount})"));
        }
        if let (Some(band), Some(scheme)) = (self.band.as_ref(), scheme)
            && !scheme.bands.is_empty()
            && scheme.band_ordinal(band).is_none()
        {
            return Err(format!(
                "band '{band}' is not declared in the '{}' scheme",
                scheme.measure
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetProjectQuantitySchemeInput {
    pub project_id: String,
    pub scheme: QuantityScheme,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetProjectQuantitySchemeOutput {
    pub project_id: String,
    pub measure: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitQuantityStateInput {
    pub project_id: String,
    /// Subject table the quantity belongs to (e.g. "character", "faction").
    pub subject_table: String,
    pub subject_id: String,
    /// Measure name; validated against a declared scheme for the same measure.
    pub measure: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    #[serde(default)]
    pub scene_id: Option<String>,
    pub state: QuantityState,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitQuantityStateOutput {
    pub quantity_state_id: String,
    /// Advisory warnings (e.g. an unexplained band jump). Empty on a clean
    /// commit; band jumps are surfaced, not blocked (design §9.5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeriveQuantitySchemeFromOverlayInput {
    pub project_id: String,
    pub system_overlay_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeriveQuantitySchemeFromOverlayOutput {
    /// The scheme measure (the overlay's progression_currency, else its name).
    pub measure: String,
    pub band_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScanScenePricesInput {
    pub project_id: String,
    pub scene_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScanScenePricesOutput {
    pub mentions: Vec<PriceMention>,
}

/// A detected price mention in prose: a number immediately followed by a known
/// currency unit (e.g. "5 silver"). Review-gated extraction, not auto-registered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PriceMention {
    pub amount: f64,
    pub unit: String,
    /// The matched "<amount> <unit>" text.
    pub matched: String,
}

/// Scan `text` for "<number> <unit>" price mentions, where `unit` is one of
/// `units` (case-insensitive, a trailing plural 's' tolerated). Pure and
/// dependency-free: a token-window scan, deliberately high-precision/low-recall.
pub fn extract_price_mentions(text: &str, units: &[String]) -> Vec<PriceMention> {
    let unit_set: std::collections::BTreeSet<String> =
        units.iter().map(|u| u.to_ascii_lowercase()).collect();
    if unit_set.is_empty() {
        return Vec::new();
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    for window in tokens.windows(2) {
        let raw_amount = window[0].trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
        let Ok(amount) = raw_amount.parse::<f64>() else {
            continue;
        };
        let raw_unit = window[1]
            .trim_matches(|c: char| !c.is_ascii_alphanumeric())
            .to_ascii_lowercase();
        let singular = raw_unit.strip_suffix('s').unwrap_or(raw_unit.as_str());
        let unit = if unit_set.contains(&raw_unit) {
            Some(raw_unit.clone())
        } else if unit_set.contains(singular) {
            Some(singular.to_string())
        } else {
            None
        };
        if let Some(unit) = unit {
            out.push(PriceMention {
                amount,
                unit,
                matched: format!("{} {}", window[0], window[1]),
            });
        }
    }
    out
}

#[cfg(test)]
mod quantity_tests {
    use super::*;

    fn coin_scheme() -> QuantityScheme {
        QuantityScheme {
            measure: "wealth".into(),
            denominations: vec![
                QuantityDenomination {
                    name: "gold".into(),
                    per_base: 100,
                },
                QuantityDenomination {
                    name: "silver".into(),
                    per_base: 10,
                },
                QuantityDenomination {
                    name: "copper".into(),
                    per_base: 1,
                },
            ],
            bands: vec![
                QuantityBand {
                    name: "destitute".into(),
                    lower_bound: Some(0),
                },
                QuantityBand {
                    name: "comfortable".into(),
                    lower_bound: Some(100),
                },
                QuantityBand {
                    name: "wealthy".into(),
                    lower_bound: Some(10_000),
                },
            ],
            max_band_jump: Some(1),
        }
    }

    #[test]
    fn valid_scheme_passes() {
        assert!(coin_scheme().validate().is_ok());
    }

    #[test]
    fn band_ordinal_reflects_order() {
        let s = coin_scheme();
        assert_eq!(s.band_ordinal("destitute"), Some(0));
        assert_eq!(s.band_ordinal("wealthy"), Some(2));
        assert_eq!(s.band_ordinal("mythic"), None);
    }

    #[test]
    fn band_jump_counts_tiers() {
        let s = coin_scheme();
        assert_eq!(s.band_jump("destitute", "comfortable"), Some(1));
        assert_eq!(s.band_jump("destitute", "wealthy"), Some(2));
        assert_eq!(s.band_jump("wealthy", "destitute"), Some(2));
        assert_eq!(s.band_jump("destitute", "mythic"), None);
    }

    #[test]
    fn max_band_jump_defaults_to_one() {
        let mut s = coin_scheme();
        s.max_band_jump = None;
        assert_eq!(s.max_band_jump_or_default(), 1);
        s.max_band_jump = Some(2);
        assert_eq!(s.max_band_jump_or_default(), 2);
    }

    #[test]
    fn amount_in_base_uses_denomination_rate() {
        let s = coin_scheme(); // gold=100, silver=10, copper=1
        assert_eq!(s.amount_in_base(5.0, "silver"), Some(50.0));
        assert_eq!(s.amount_in_base(2.0, "gold"), Some(200.0));
        assert_eq!(s.amount_in_base(7.0, "copper"), Some(7.0));
        assert_eq!(s.amount_in_base(1.0, "Silver"), Some(10.0)); // case-insensitive
        assert_eq!(s.amount_in_base(1.0, "mithril"), None);
    }

    #[test]
    fn extract_price_mentions_finds_amount_unit_pairs() {
        let units = vec!["silver".to_string(), "gold".to_string()];
        let m = extract_price_mentions("A loaf costs 5 silver and a sword 3 gold.", &units);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].amount, m[0].unit.as_str()), (5.0, "silver"));
        assert_eq!((m[1].amount, m[1].unit.as_str()), (3.0, "gold"));
        // trailing plural + punctuation tolerated
        let plural = extract_price_mentions("He paid 12 silvers.", &units);
        assert_eq!(plural.len(), 1);
        assert_eq!((plural[0].amount, plural[0].unit.as_str()), (12.0, "silver"));
        // unknown unit and empty vocabulary yield nothing
        assert!(extract_price_mentions("10 apples", &units).is_empty());
        assert!(extract_price_mentions("5 silver", &[]).is_empty());
    }

    #[test]
    fn duplicate_band_rejected() {
        let mut s = coin_scheme();
        s.bands.push(QuantityBand {
            name: "wealthy".into(),
            lower_bound: Some(20_000),
        });
        assert!(s.validate().is_err());
    }

    #[test]
    fn non_increasing_band_bounds_rejected() {
        let mut s = coin_scheme();
        s.bands[2].lower_bound = Some(50); // below comfortable's 100
        assert!(s.validate().is_err());
    }

    #[test]
    fn zero_per_base_denomination_rejected() {
        let mut s = coin_scheme();
        s.denominations[0].per_base = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn empty_measure_rejected() {
        let mut s = coin_scheme();
        s.measure = "  ".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn negative_amount_rejected() {
        let st = QuantityState {
            amount: Some(-1.0),
            ..Default::default()
        };
        assert!(st.validate(Some(&coin_scheme())).is_err());
    }

    #[test]
    fn band_must_be_declared_in_scheme() {
        let bad = QuantityState {
            band: Some("mythic".into()),
            ..Default::default()
        };
        assert!(bad.validate(Some(&coin_scheme())).is_err());
        let good = QuantityState {
            band: Some("wealthy".into()),
            ..Default::default()
        };
        assert!(good.validate(Some(&coin_scheme())).is_ok());
    }

    #[test]
    fn band_unchecked_without_scheme() {
        let st = QuantityState {
            band: Some("anything".into()),
            ..Default::default()
        };
        assert!(st.validate(None).is_ok());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetProjectCalendarInput {
    pub project_id: String,
    pub calendar: CalendarDef,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetProjectCalendarOutput {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetSceneClockInput {
    pub project_id: String,
    pub scene_id: String,
    #[serde(default)]
    pub clock: StoryClock,
    /// `linear | flashback | flashforward | concurrent` (default `linear`).
    #[serde(default)]
    pub temporal_mode: Option<String>,
    /// Parallel-timeline key; scenes on different threads may overlap in time.
    #[serde(default)]
    pub thread_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetSceneClockOutput {
    pub scene_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetTimelineEventClockInput {
    pub project_id: String,
    pub timeline_event_id: String,
    #[serde(default)]
    pub clock: StoryClock,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetTimelineEventClockOutput {
    pub timeline_event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetCharacterBirthInput {
    pub project_id: String,
    pub character_id: String,
    #[serde(default)]
    pub clock: StoryClock,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetCharacterBirthOutput {
    pub character_id: String,
}

#[cfg(test)]
mod story_clock_tests {
    use super::*;

    fn gregorian() -> CalendarDef {
        CalendarDef {
            days_per_week: 7,
            hours_per_day: 24,
            week_day_names: Vec::new(),
            months: Vec::new(),
            days_per_year: 365,
            epoch_label: None,
        }
    }

    #[test]
    fn total_index_is_none_without_day_index() {
        assert_eq!(StoryClock::default().total_index(&gregorian()), None);
    }

    #[test]
    fn total_index_folds_day_and_time() {
        let clock = StoryClock {
            day_index: Some(3),
            time_of_day: Some(120),
            ..Default::default()
        };
        assert_eq!(clock.total_index(&gregorian()), Some(3 * 1440 + 120));
    }

    #[test]
    fn total_index_respects_non_24h_calendar() {
        let mut cal = gregorian();
        cal.hours_per_day = 10; // 600 minutes/day
        let clock = StoryClock {
            day_index: Some(2),
            time_of_day: Some(30),
            ..Default::default()
        };
        assert_eq!(clock.total_index(&cal), Some(2 * 600 + 30));
    }

    #[test]
    fn invented_calendar_months_must_sum_to_year() {
        let mut cal = gregorian();
        cal.days_per_week = 5;
        cal.months = vec![
            CalendarMonth {
                name: "Frost".into(),
                days: 30,
            },
            CalendarMonth {
                name: "Thaw".into(),
                days: 40,
            },
        ];
        assert!(cal.validate().is_err(), "70 != 365 must be rejected");
        cal.days_per_year = 70;
        assert!(cal.validate().is_ok());
    }

    #[test]
    fn clock_rejects_out_of_range_time_of_day() {
        let cal = gregorian();
        assert!(
            StoryClock {
                day_index: Some(0),
                time_of_day: Some(1440),
                ..Default::default()
            }
            .validate(&cal)
            .is_err()
        );
        assert!(
            StoryClock {
                day_index: Some(0),
                time_of_day: Some(1439),
                ..Default::default()
            }
            .validate(&cal)
            .is_ok()
        );
    }

    #[test]
    fn clock_rejects_negative_day_index_and_unknown_precision() {
        let cal = gregorian();
        assert!(
            StoryClock {
                day_index: Some(-1),
                ..Default::default()
            }
            .validate(&cal)
            .is_err()
        );
        assert!(
            StoryClock {
                precision: Some("fortnight".into()),
                ..Default::default()
            }
            .validate(&cal)
            .is_err()
        );
        assert!(
            StoryClock {
                precision: Some("day".into()),
                ..Default::default()
            }
            .validate(&cal)
            .is_ok()
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriterPosition {
    pub project_id: String,
    pub branch_id: String,
    pub book_id: Option<String>,
    pub chapter_id: Option<String>,
    pub scene_id: Option<String>,
    pub intent: String,
    pub next_focus: Option<String>,
    pub updated_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionActivity {
    pub project_id: String,
    pub branch_id: String,
    pub kind: String,
    pub subject_table: Option<String>,
    pub subject_id: Option<String>,
    pub summary: String,
    pub details_json: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProgressionEvent {
    pub project_id: String,
    pub branch_id: String,
    pub subject_table: String,
    pub subject_id: String,
    pub overlay_id: Option<String>,
    pub kind: String,
    pub delta_json: serde_json::Value,
    pub source_scene_id: Option<String>,
    pub placement: Option<StoryPlacement>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TryFailCycleStep {
    pub attempt_order: i32,
    pub label: String,
    pub outcome: String,
    pub cost: Option<String>,
    pub revelation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StatedConsequence {
    pub description: String,
    pub stated_at: Option<StoryPlacement>,
    pub must_demonstrate_by: Option<String>,
    pub delivered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CharacterArcMilestone {
    pub label: String,
    pub placement: Option<StoryPlacement>,
    pub description: String,
    #[serde(default)]
    pub unlocks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct PlannedScene {
    pub scene_order: i32,
    pub summary: String,
    #[serde(default)]
    pub beat_structure: Vec<String>,
    #[serde(default)]
    pub character_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_rating: Option<ContentRating>,
    pub purpose: String,
    #[serde(default)]
    pub research_required: Option<bool>,
    #[serde(default)]
    pub research_tags: Vec<String>,
    #[serde(default)]
    pub explicit_query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChapterOutlineBeat {
    pub order: i32,
    pub summary: String,
    pub scene_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BookOutline {
    pub book_id: String,
    pub branch_id: String,
    pub format: String,
    pub content: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChapterOutline {
    pub chapter_id: String,
    pub branch_id: String,
    pub format: String,
    pub content: String,
    #[serde(default)]
    pub beats: Vec<ChapterOutlineBeat>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AnnotatedBeat {
    pub beat_type: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateProjectInput {
    pub name: String,
    pub project_type: String,
    pub genre: String,
    pub reader_contract: ReaderContract,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateProjectOutput {
    pub project_id: String,
    /// Id of the project's main bible branch. Each project gets its own main
    /// branch (per-project main branch design, Phase 6); the SurrealDB era
    /// had a singleton `bible_branch:main` shared globally — that assumption
    /// is no longer valid. `#[serde(default)]` keeps older clients that
    /// don't send this field on round-trips compatible.
    #[serde(default)]
    pub branch_id: String,
    pub book_id: String,
    pub chapter_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListProjectsOutput {
    pub projects: Vec<ProjectSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetActiveProjectInput {
    pub project_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetActiveProjectOutput {
    pub project_id: String,
    pub branch_id: String,
    pub branch_name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectSummary {
    /// Record id to use with other tools (e.g. "project:abc123def")
    pub project_id: String,
    pub name: String,
    pub project_type: String,
    pub genre: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_style_profile_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateCharacterInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub name: String,
    pub summary: String,
    pub role: String,
    pub realm: Option<String>,
    pub voice_profile: CharacterVoiceProfileData,
    pub emotional_profile: CharacterEmotionalProfileData,
    pub initial_state: Option<CharacterStatePatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateCharacterOutput {
    pub character_id: String,
    pub voice_profile_id: String,
    pub emotional_profile_id: String,
    pub state_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateLocationInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub name: String,
    #[serde(default, alias = "type")]
    pub kind: String,
    pub realm: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub initial_state: WorldStateInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateLocationOutput {
    pub location_id: String,
    pub world_state_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateRelationshipInput {
    /// Record id returned by create_character (e.g. "character:abc123")
    pub character_a_id: String,
    /// Record id returned by create_character (e.g. "character:xyz789")
    pub character_b_id: String,
    pub relationship_type: String,
    pub initial_trust: i32,
    pub initial_tension: i32,
    #[serde(default)]
    pub dynamics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateRelationshipOutput {
    pub relationship_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateWorldRuleInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub rule_name: String,
    pub rule_type: String,
    pub description: String,
    #[serde(default)]
    pub scan_pattern: Option<String>,
    #[serde(default)]
    pub relevance_tags: Vec<String>,
    pub established_in: Option<EstablishedIn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateWorldRuleOutput {
    pub world_rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSceneContextInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    #[serde(default)]
    pub book_number: i32,
    #[serde(default)]
    pub chapter_number: i32,
    #[serde(default)]
    pub chapter_id: Option<String>,
    pub scene_order: i32,
    /// Record ids returned by create_character (e.g. ["character:abc123"])
    pub character_ids: Vec<String>,
    /// Optional opt-in cap for resolved scene characters. When omitted, all
    /// requested characters are included. If set, the first N character ids in
    /// caller order are used for the scene-layer roster and character-dependent
    /// briefings.
    #[serde(default)]
    pub max_character_count: Option<usize>,
    /// Record id returned by create_location (e.g. "location:xyz789")
    pub location_id: String,
    /// Render format. Defaults to markdown when omitted.
    #[serde(default)]
    pub format: Option<ContextFormat>,
    /// Preferred token budget for temporary inline trimming. Mandatory hard
    /// constraints may expand the effective budget rather than being dropped.
    #[serde(default)]
    pub budget_tokens: Option<usize>,
    /// Legacy budget field retained for backwards compatibility.
    #[serde(default)]
    pub token_budget: Option<usize>,
    /// Optional section filter. When omitted, the full scene context is
    /// returned. Supported top-level groups: "standards", "novel", "scene".
    /// Supported novel sections: "reader_contract", "world_rules",
    /// "system_overlays", "timeline_briefing", "future_knowledge_briefing",
    /// "pacing_directives", "narrative_promises_due", "knowledge_briefing",
    /// "semantic_references". Supported scene sections: "location",
    /// "world_state", "characters", "relationships", "agency_check".
    #[serde(default)]
    pub sections: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextFormat {
    Markdown,
    Json,
}

impl GetSceneContextInput {
    pub fn wants_standards(&self) -> bool {
        self.sections
            .as_ref()
            .is_none_or(|sections| sections.iter().any(|section| section == "standards"))
    }

    pub fn wants_novel_section(&self, name: &str) -> bool {
        self.wants_grouped_section("novel", name)
    }

    pub fn wants_scene_section(&self, name: &str) -> bool {
        self.wants_grouped_section("scene", name)
    }

    fn wants_grouped_section(&self, group: &str, name: &str) -> bool {
        self.sections.as_ref().is_none_or(|sections| {
            sections
                .iter()
                .any(|section| section == group || section == name)
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneContextNovelLayer {
    pub reader_contract: ReaderContract,
    /// Consolidated, forcefully-framed style contract (reader contract +
    /// `style`-typed world rules + narrator voice). This is the PRIMARY
    /// genre-voice enforcement surface; rendered prominently and never trimmed.
    /// `None` only when the project has no style signal at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_directive: Option<crate::style::StyleDirective>,
    pub world_rules: Vec<WorldRuleSummary>,
    #[serde(default)]
    pub subjects: Vec<SubjectSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_style_profile_id: Option<String>,
    #[serde(default)]
    pub system_overlays: Vec<SystemOverlaySummary>,
    #[serde(default)]
    pub timeline_briefing: Vec<TimelineEventSummary>,
    #[serde(default)]
    pub future_knowledge_briefing: Vec<FutureKnowledgeSummary>,
    #[serde(default)]
    pub pacing_directives: Vec<PacingDirectiveSummary>,
    #[serde(default)]
    pub narrative_promises_due: Vec<NarrativePromiseDueSummary>,
    #[serde(default)]
    pub knowledge_briefing: Vec<KnowledgeBriefingItem>,
    #[serde(default)]
    pub semantic_references: Vec<SearchBibleResultItem>,
    /// Economy entities in play (static worldbuilding lore: currency, scarce
    /// resources, trade goods). Reference material so the drafting model knows
    /// what trade system is in force; price *values* ride canonical facts.
    #[serde(default)]
    pub economy_briefing: Vec<EconomySummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorldRuleSummary {
    pub rule_name: String,
    pub rule_type: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SystemOverlaySummary {
    pub system_name: String,
    pub system_type: String,
    pub visibility: String,
    pub rules: String,
    #[serde(default)]
    pub stats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TimelineEventSummary {
    pub title: String,
    pub event_type: String,
    pub placement: StoryPlacement,
    pub summary: String,
}

/// An economy in play, projected for scene-context briefing. Static lore
/// (currency, scarce resources, trade goods) — quantitative price *values*
/// are carried separately as numeric canonical facts.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EconomySummary {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scarce_resources: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trade_goods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FutureKnowledgeSummary {
    pub character_id: String,
    pub knowledge_summary: String,
    pub source: String,
    pub learned_at: StoryPlacement,
    pub expires_at: Option<StoryPlacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PacingDirectiveSummary {
    pub character_arc_id: String,
    pub tracker_id: String,
    pub character_id: String,
    pub status: String,
    pub current_progress: f64,
    pub budget_remaining: f64,
    pub velocity: String,
    pub next_milestone: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NarrativePromiseDueSummary {
    pub narrative_promise_id: String,
    pub promise_type: String,
    pub description: String,
    pub status: String,
    pub planted_at: StoryPlacement,
    pub planned_payoff: Option<StoryPlacement>,
    pub urgency: String,
    pub chapters_since_plant: i32,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeBriefingItem {
    pub character_id: String,
    pub scope: String,
    pub fact: String,
    pub source: String,
    pub learned_at: Option<StoryPlacement>,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CharacterStateSummary {
    pub character_id: String,
    pub name: String,
    pub summary: String,
    pub role: String,
    pub emotional_state: BTreeMap<String, serde_json::Value>,
    pub goals: Vec<String>,
    pub status: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RelationshipSummary {
    pub relationship_id: String,
    pub source_character_id: String,
    pub target_character_id: String,
    pub relationship_type: String,
    pub trust: i32,
    pub tension: i32,
    pub dynamics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneContextSceneLayer {
    pub location: LocationSummary,
    pub world_state: WorldStateSummary,
    pub characters: Vec<CharacterStateSummary>,
    pub relationships: Vec<RelationshipSummary>,
    pub agency_check: AgencyCheckSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgencyCheckSummary {
    pub protagonist_character_id: Option<String>,
    pub scenes_since_active_choice: usize,
    pub needs_active_choice: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LocationSummary {
    pub location_id: String,
    pub name: String,
    pub kind: String,
    pub realm: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorldStateSummary {
    pub controlling_faction: Option<String>,
    pub status: Option<String>,
    pub prosperity: Option<String>,
    pub stability: Option<String>,
    pub threat_level: Option<String>,
    pub sensory_details: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneContextBudgetMeta {
    pub estimated_tokens: usize,
    pub token_budget: Option<usize>,
    pub novel_layer_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HardConstraint {
    pub id: String,
    pub statement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalFactReadModel {
    pub canonical_fact_id: String,
    pub subject_table: String,
    #[serde(default)]
    pub subject_id: Option<String>,
    pub predicate: String,
    pub value_kind: String,
    #[serde(default)]
    pub value_text: Option<String>,
    #[serde(default)]
    pub value_number: Option<f64>,
    #[serde(default)]
    pub value_unit: Option<String>,
    #[serde(default)]
    pub value_json: Option<serde_json::Value>,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub scope: String,
    #[serde(default)]
    pub valid_from: Option<StoryPlacement>,
    #[serde(default)]
    pub valid_until: Option<StoryPlacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneContextOutput {
    #[serde(default)]
    pub hard_constraints: Vec<HardConstraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub canonical_facts: Vec<CanonicalFactReadModel>,
    pub novel: SceneContextNovelLayer,
    pub scene: SceneContextSceneLayer,
    pub budget: SceneContextBudgetMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneContextEnvelope {
    #[serde(default)]
    pub hard_constraints: Vec<HardConstraint>,
    pub standards: String,
    pub novel: SceneContextNovelLayer,
    pub scene: SceneContextSceneLayer,
    pub budget: SceneContextBudgetMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_markdown: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetChapterBriefingInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    #[serde(default)]
    pub scene_order: Option<i32>,
    /// Record ids returned by create_character (e.g. ["character:abc123"])
    #[serde(default)]
    pub character_ids: Vec<String>,
    /// Record id returned by create_location (e.g. "location:xyz789")
    #[serde(default)]
    pub location_id: Option<String>,
    /// Render format. Defaults to markdown when omitted.
    #[serde(default)]
    pub format: Option<ContextFormat>,
    /// Preferred token budget for temporary inline trimming.
    #[serde(default)]
    pub budget_tokens: Option<usize>,
    /// Number of prior chapter summaries to include, newest first. Defaults to 3.
    pub recent_chapter_limit: Option<usize>,
    /// Token budget passed through to the bundled scene-context slice.
    /// Defaults to the service's chapter-briefing budget when omitted.
    #[serde(default)]
    pub token_budget: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChapterSummaryBriefing {
    pub book_number: i32,
    pub chapter_number: i32,
    pub summary: String,
    #[serde(default)]
    pub key_events: Vec<String>,
    #[serde(default)]
    pub character_changes: Vec<String>,
    #[serde(default)]
    pub relationship_shifts: Vec<String>,
    #[serde(default)]
    pub arc_advances: Vec<String>,
    #[serde(default)]
    pub promise_events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChapterPlanBriefing {
    pub synopsis: String,
    pub pov_character_id: Option<String>,
    #[serde(default)]
    pub target_theme_ids: Vec<String>,
    #[serde(default)]
    pub target_conflict_ids: Vec<String>,
    #[serde(default)]
    pub target_plot_line_ids: Vec<String>,
    #[serde(default)]
    pub scenes: Vec<PlannedScene>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChapterBriefingSceneSeed {
    pub scene_order: Option<i32>,
    #[serde(default)]
    pub character_ids: Vec<String>,
    pub location_id: Option<String>,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    pub scene_context_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetChapterBriefingOutput {
    #[serde(default)]
    pub hard_constraints: Vec<HardConstraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub canonical_facts: Vec<CanonicalFactReadModel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub continuity_sheets: Vec<SubjectSnapshot>,
    pub briefing_markdown: String,
    #[serde(default)]
    pub recent_chapter_summaries: Vec<ChapterSummaryBriefing>,
    pub chapter_outline: Option<ChapterOutline>,
    pub book_outline: Option<BookOutline>,
    pub chapter_plan: Option<ChapterPlanBriefing>,
    pub scene_seed: ChapterBriefingSceneSeed,
    pub scene_context: Option<SceneContextOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetWriterStateInput {
    /// Record id returned by create_project (for example "project:abc123def")
    pub project_id: String,
    /// Optional explicit branch. Defaults to the active branch.
    #[serde(default)]
    pub branch_id: Option<String>,
    /// Optional explicit scene cursor. When omitted, Spindle derives the cursor
    /// from the most recent written scene on the branch.
    #[serde(default)]
    pub at_scene_id: Option<String>,
    /// Render format. Defaults to markdown when omitted.
    #[serde(default)]
    pub format: Option<ContextFormat>,
    /// Preferred token budget for markdown rendering metadata.
    #[serde(default)]
    pub budget_tokens: Option<usize>,
    /// Include derived subject snapshots. Defaults to true.
    #[serde(default)]
    pub include_subjects: Option<bool>,
    /// Include derived recent activity. Defaults to true.
    #[serde(default)]
    pub include_recent_activity: Option<bool>,
    /// Maximum number of recent activity entries to include. Defaults to 20.
    #[serde(default)]
    pub recent_activity_limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriterStateEnvelope {
    pub current: WriterStateCurrent,
    pub next: WriterStateNext,
    #[serde(default)]
    pub hard_constraints: Vec<HardConstraint>,
    #[serde(default)]
    pub subjects: Vec<WriterStateSubjectSnapshot>,
    #[serde(default)]
    pub recent_scenes: Vec<RecentSceneSummary>,
    #[serde(default)]
    pub open_promises_due_now: Vec<WriterStateNarrativePromiseSummary>,
    #[serde(default)]
    pub active_overlays: Vec<OverlayWithTrajectory>,
    #[serde(default)]
    pub drift_warnings: Vec<DriftWarning>,
    #[serde(default)]
    pub unsynced_local_files: Vec<UnsyncedFileEntry>,
    #[serde(default)]
    pub recent_session_activity: Vec<SessionActivitySummary>,
    #[serde(default)]
    pub chapter_outline: Option<ChapterOutline>,
    #[serde(default)]
    pub book_outline: Option<BookOutline>,
    pub bundle_summary: ContextBundleSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writer_state_markdown: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetEntityInput {
    /// Record id returned by create_project (for example "project:abc123def")
    pub project_id: String,
    /// Subject table kind for the requested entity.
    pub table: SubjectTable,
    /// Record id for the requested entity (for example "character:abc123def")
    pub entity_id: String,
    /// Optional explicit branch. Defaults to the active branch.
    #[serde(default)]
    pub branch_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum EntityResolutionConfidence {
    ExactName,
    SemanticMatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FindEntityInput {
    /// Record id returned by create_project (for example "project:abc123def")
    pub project_id: String,
    /// Freeform lookup text. Can be an exact name or an alias/descriptor.
    pub query: String,
    /// Optional explicit branch. Defaults to the active branch.
    #[serde(default)]
    pub branch_id: Option<String>,
    /// Optional table filter for disambiguation.
    #[serde(default)]
    pub table: Option<SubjectTable>,
    /// Maximum matches to return (1..=20). Defaults to 5.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FindEntityMatch {
    /// Canonical record id (for example "character:abc123def").
    pub entity_id: String,
    pub confidence: EntityResolutionConfidence,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FindEntityOutput {
    #[serde(default)]
    pub matches: Vec<FindEntityMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetCharacterSnapshotInput {
    /// Record id returned by create_project (for example "project:abc123def")
    pub project_id: String,
    /// Record id returned by create_character (for example "character:abc123def")
    pub character_id: String,
    /// Optional explicit branch. Defaults to the active branch.
    #[serde(default)]
    pub branch_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CharacterSnapshotOutput {
    pub snapshot: SubjectSnapshot,
    #[serde(default)]
    pub voice_profile: Option<VoiceProfileSummary>,
    #[serde(default)]
    pub current_state: Option<SnapshotCharacterStateSummary>,
    #[serde(default)]
    pub recent_appearances: Vec<SceneAppearanceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriterState {
    pub current: WriterStateCurrent,
    pub next: WriterStateNext,
    #[serde(default)]
    pub hard_constraints: Vec<HardConstraint>,
    #[serde(default)]
    pub subjects: Vec<WriterStateSubjectSnapshot>,
    #[serde(default)]
    pub recent_scenes: Vec<RecentSceneSummary>,
    #[serde(default)]
    pub open_promises_due_now: Vec<WriterStateNarrativePromiseSummary>,
    #[serde(default)]
    pub active_overlays: Vec<OverlayWithTrajectory>,
    #[serde(default)]
    pub drift_warnings: Vec<DriftWarning>,
    #[serde(default)]
    pub unsynced_local_files: Vec<UnsyncedFileEntry>,
    #[serde(default)]
    pub recent_session_activity: Vec<SessionActivitySummary>,
    #[serde(default)]
    pub chapter_outline: Option<ChapterOutline>,
    #[serde(default)]
    pub book_outline: Option<BookOutline>,
    pub bundle_summary: ContextBundleSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriterStateCurrent {
    pub project: ProjectSummary,
    pub branch: BranchSummary,
    #[serde(default)]
    pub book: Option<BookSummary>,
    #[serde(default)]
    pub chapter: Option<ChapterPositionSummary>,
    #[serde(default)]
    pub scene: Option<ScenePositionSummary>,
    #[serde(default)]
    pub last_completed_scene_summary: Option<String>,
    pub intent: WriterIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriterStateNext {
    #[serde(default)]
    pub intended_focus: Option<String>,
    #[serde(default)]
    pub outline_section_ref: Option<OutlineRef>,
    #[serde(default)]
    pub suggested_subjects: Vec<SubjectRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BookSummary {
    pub book_id: String,
    pub book_number: i32,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChapterPositionSummary {
    pub chapter_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScenePositionSummary {
    pub scene_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub summary: String,
    #[serde(default)]
    pub tone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WriterIntent {
    Drafting,
    Planning,
    Revising,
    Idle,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutlineRef {
    #[serde(default)]
    pub chapter_id: Option<String>,
    #[serde(default)]
    pub scene_order: Option<i32>,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubjectRef {
    pub subject_id: String,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
/// Subject snapshot specific to writer-state context.
/// Distinct from `subject_snapshot::SubjectSnapshot`, which carries deep provenance
/// and rendering metadata used by scene/chapter context tooling.
pub struct WriterStateSubjectSnapshot {
    pub subject: SubjectRef,
    pub summary: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Vec<String>,
    #[serde(default)]
    pub relationships: Vec<RelationshipSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecentSceneSummary {
    pub scene_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub summary: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriterStateNarrativePromiseSummary {
    pub narrative_promise_id: String,
    pub promise_type: String,
    pub description: String,
    pub status: String,
    pub planted_at: StoryPlacement,
    #[serde(default)]
    pub planned_payoff: Option<StoryPlacement>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OverlayWithTrajectory {
    pub overlay_id: String,
    pub name: String,
    pub current_value: serde_json::Value,
    pub trajectory_delta_since_last_chapter: serde_json::Value,
    #[serde(default)]
    pub recent_events: Vec<ProgressionEventSummary>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProgressionEventSummary {
    pub summary: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Provenance {
    pub source: String,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DriftWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UnsyncedFileEntry {
    pub scene_id: String,
    pub source_path: String,
    pub kind: DivergenceKind,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionActivitySummary {
    pub kind: String,
    #[serde(default)]
    pub subject_table: Option<String>,
    #[serde(default)]
    pub subject_id: Option<String>,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContextBundleSummary {
    pub format: ContextFormat,
    pub estimated_tokens: usize,
    #[serde(default)]
    pub token_budget: Option<usize>,
    pub truncated: bool,
    #[serde(default)]
    pub included_sections: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSceneDeleteImpactInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneDeleteImpactTarget {
    pub scene_id: String,
    pub branch_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneDeleteImpactGroup {
    pub dependency_type: String,
    pub count: usize,
    #[serde(default)]
    pub sample_record_ids: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SceneDeleteReadiness {
    Clear,
    NeedsFollowup,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSceneDeleteImpactOutput {
    pub active_branch_id: String,
    pub active_branch_name: String,
    pub scene: SceneDeleteImpactTarget,
    pub delete_readiness: SceneDeleteReadiness,
    #[serde(default)]
    pub hard_blockers: Vec<SceneDeleteImpactGroup>,
    #[serde(default)]
    pub semantic_risks: Vec<SceneDeleteImpactGroup>,
    #[serde(default)]
    pub chapter_artifacts: Vec<SceneDeleteImpactGroup>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSceneMoveImpactInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub from_book_number: i32,
    pub from_chapter_number: i32,
    pub from_scene_order: i32,
    pub to_book_number: i32,
    pub to_chapter_number: i32,
    pub to_scene_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneMoveImpactDestination {
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub existing_scene_id: Option<String>,
    pub existing_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneMoveImpactGroup {
    pub dependency_type: String,
    pub count: usize,
    #[serde(default)]
    pub sample_record_ids: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SceneMoveReadiness {
    Clear,
    NeedsFollowup,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSceneMoveImpactOutput {
    pub active_branch_id: String,
    pub active_branch_name: String,
    pub scene: SceneDeleteImpactTarget,
    pub destination: SceneMoveImpactDestination,
    pub move_readiness: SceneMoveReadiness,
    #[serde(default)]
    pub hard_blockers: Vec<SceneMoveImpactGroup>,
    #[serde(default)]
    pub semantic_risks: Vec<SceneMoveImpactGroup>,
    #[serde(default)]
    pub chapter_artifacts: Vec<SceneMoveImpactGroup>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MoveSceneInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub from_book_number: i32,
    pub from_chapter_number: i32,
    pub from_scene_order: i32,
    pub to_book_number: i32,
    pub to_chapter_number: i32,
    pub to_scene_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MoveSceneOutput {
    pub scene_id: String,
    pub branch_id: String,
    pub status: String,
    pub from_book_number: i32,
    pub from_chapter_number: i32,
    pub from_scene_order: i32,
    pub to_book_number: i32,
    pub to_chapter_number: i32,
    pub to_scene_order: i32,
    pub left_source_scene_order_gap: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteSceneInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteSceneOutput {
    pub scene_id: String,
    pub branch_id: String,
    pub status: String,
    pub left_scene_order_gap: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OperatorDeleteSceneInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OperatorDeleteSceneOutput {
    pub scene_id: String,
    pub branch_id: String,
    pub status: String,
    pub left_scene_order_gap: bool,
    #[serde(default)]
    pub removed_scene_source_link_ids: Vec<String>,
    #[serde(default)]
    pub invalidated_chapter_plan_ids: Vec<String>,
    #[serde(default)]
    pub invalidated_chapter_summary_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct SaveSceneDraftInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    /// Optional explicit book placement. When omitted, provide `chapter_id`.
    #[serde(default)]
    pub book_number: i32,
    /// Optional explicit chapter placement. When omitted, provide `chapter_id`.
    #[serde(default)]
    pub chapter_number: i32,
    /// Optional chapter reference. When provided, Spindle resolves book/chapter
    /// placement from the existing chapter instead of requiring numeric fields.
    #[serde(default)]
    pub chapter_id: Option<String>,
    pub scene_order: i32,
    #[serde(alias = "content", alias = "text")]
    pub full_text: String,
    pub summary: String,
    pub content_rating: ContentRating,
    pub tone: Option<String>,
    /// Optional server-side receipt id returned by `continue_generation`.
    /// Required when saving explicit sexual prose with `content_rating:
    /// "explicit"`. When provided for an explicit-rated save, Spindle persists
    /// the server-held generation output as `full_text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    /// Optional path to the local source file this scene was written from.
    /// When provided, Spindle tracks the file for divergence detection.
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub research_source_ids: Vec<String>,
    #[serde(default)]
    pub research_note_ids: Vec<String>,
    #[serde(default)]
    pub research_claim_ids: Vec<String>,
    #[serde(default)]
    pub research_query_pack_input: Option<String>,
    #[serde(default)]
    pub research_context_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SaveSceneDraftOutput {
    pub scene_id: String,
    pub status: String,
    /// Origin recorded for the saved prose, e.g. `client` or `agent:venice`.
    pub draft_origin: String,
    #[serde(default)]
    pub pacing_warnings: Vec<String>,
    #[serde(default)]
    pub agency_warning: Option<AgencyWarning>,
    pub tone_deviation: bool,
    /// Genre-voice mismatches against the project style contract (reader
    /// contract style_notes, `style` world rules, narrator voice). A non-empty
    /// list is a failed style gate — revise before committing. Coarse by
    /// design; an empty list is not proof the scene is on-genre.
    #[serde(default)]
    pub style_warnings: Vec<String>,
    pub content_rating_valid: bool,
    #[serde(default)]
    pub content_rating_warnings: Vec<String>,
    #[serde(default)]
    pub diff: Vec<TextDiffChunk>,
    #[serde(default)]
    pub byte_offsets_changed: Vec<TextByteRange>,
    pub chars_added: usize,
    pub chars_deleted: usize,
    #[serde(default)]
    pub world_rule_hits: Vec<WorldRuleHit>,
    #[serde(default)]
    pub voice_drift: Vec<VoiceDriftFinding>,
    #[serde(default)]
    pub retcon_findings: Vec<RetconFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgencyWarningKind {
    Passive,
    /// Reserved for Phase 4 validator expansion; intentionally unused in Phase 1.
    OffScreenResolution,
    /// Reserved for Phase 4 validator expansion; intentionally unused in Phase 1.
    NoChoice,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct AgencyEvidence {
    pub scene_id: String,
    pub byte_range: TextByteRange,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct AgencyWarning {
    pub kind: AgencyWarningKind,
    pub message: String,
    #[serde(default)]
    pub character_id: Option<String>,
    #[serde(default)]
    pub character_name: Option<String>,
    #[serde(default)]
    pub evidence: Vec<AgencyEvidence>,
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListChapterScenesInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    /// Optional chapter reference. When provided, Spindle resolves book/chapter
    /// placement from the existing chapter instead of requiring numeric fields.
    #[serde(default)]
    pub chapter_id: Option<String>,
    /// Optional explicit book placement. When omitted, provide `chapter_id`.
    #[serde(default)]
    pub book_number: i32,
    /// Optional explicit chapter placement. When omitted, provide `chapter_id`.
    #[serde(default)]
    pub chapter_number: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneSpineEntry {
    pub scene_id: String,
    pub scene_order: i32,
    pub word_count: usize,
    pub summary_first_line: String,
    pub has_canonical_facts: bool,
    pub content_rating: ContentRating,
    #[serde(default)]
    pub tone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListChapterScenesOutput {
    pub project_id: String,
    pub branch_id: String,
    pub book_id: String,
    pub chapter_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub scenes: Vec<SceneSpineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListBookChaptersInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    /// Optional book reference. When provided, Spindle resolves book_number
    /// from the existing book instead of requiring numeric fields.
    #[serde(default)]
    pub book_id: Option<String>,
    /// Optional explicit book placement. When omitted, provide `book_id`.
    #[serde(default)]
    pub book_number: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChapterSpineEntry {
    pub chapter_id: String,
    pub chapter_number: i32,
    #[serde(default)]
    pub title: Option<String>,
    pub scene_count: usize,
    #[serde(default)]
    pub scenes: Vec<SceneSpineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListBookChaptersOutput {
    pub project_id: String,
    pub branch_id: String,
    pub book_id: String,
    pub book_number: i32,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub chapters: Vec<ChapterSpineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListSceneVersionsInput {
    pub project_id: String,
    pub scene_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneVersionSummary {
    pub scene_version_id: String,
    pub version_number: i32,
    pub saved_at: String,
    pub word_count: usize,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListSceneVersionsOutput {
    pub scene_id: String,
    #[serde(default)]
    pub versions: Vec<SceneVersionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RestoreSceneVersionInput {
    pub project_id: String,
    pub scene_id: String,
    pub scene_version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RestoreSceneVersionOutput {
    pub scene_id: String,
    pub restored_from_version_id: String,
    pub restored_version_number: i32,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitCharacterStateInput {
    /// Record id returned by create_character (e.g. "character:abc123")
    pub character_id: String,
    /// Record id returned by save_scene_draft (e.g. "scene:abc123")
    pub scene_id: String,
    pub changes: CharacterStatePatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitCharacterStateOutput {
    pub state_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateRelationshipInput {
    /// Record id returned by create_character (e.g. "character:abc123")
    pub character_a_id: String,
    /// Record id returned by create_character (e.g. "character:xyz789")
    pub character_b_id: String,
    pub trust_delta: i32,
    pub tension_delta: i32,
    pub reason: String,
    /// Record id returned by save_scene_draft (e.g. "scene:abc123")
    pub scene_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateRelationshipOutput {
    pub relationship_id: String,
    pub trust: i32,
    pub tension: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneChangesInput {
    pub project_id: String,
    pub scene_id: String,
    #[serde(default)]
    pub character_states: Vec<CharacterStatePatchEntry>,
    #[serde(default)]
    pub canonical_facts: Vec<CanonicalFactEntry>,
    #[serde(default)]
    pub relationship_updates: Vec<RelationshipUpdateEntry>,
    #[serde(default)]
    pub accept_world_rule_risks: bool,
    /// When true, bypass the write-time continuity gate (canonical-fact
    /// contradictions and prose retcons) even under `BlockErrors`.
    #[serde(default)]
    pub accept_continuity_risks: bool,
    /// How the write-time continuity gate behaves. Defaults to `BlockErrors`
    /// (block the commit on continuity errors unless `accept_continuity_risks`).
    #[serde(default)]
    pub continuity_gate: Option<CommitContinuityGate>,
}

/// Behavior of the write-time continuity gate on `commit_scene_changes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommitContinuityGate {
    /// Do not run the gate.
    Off,
    /// Run the gate and return findings, but never block the commit.
    WarnOnly,
    /// Block the commit when continuity errors are found (the default).
    #[default]
    BlockErrors,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CharacterStatePatchEntry {
    pub character_id: String,
    pub changes: CharacterStatePatch,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CanonicalFactEntry {
    #[serde(default)]
    pub fact_type: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub subject_table: Option<String>,
    #[serde(default)]
    pub subject_id: Option<String>,
    #[serde(default)]
    pub predicate: Option<String>,
    #[serde(default)]
    pub value_kind: Option<String>,
    #[serde(default)]
    pub value_text: Option<String>,
    #[serde(default)]
    pub value_number: Option<f64>,
    #[serde(default)]
    pub value_unit: Option<String>,
    #[serde(default)]
    pub value_json: Option<serde_json::Value>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub scope: Option<CanonicalFactScope>,
    #[serde(default)]
    pub valid_from: Option<StoryPlacement>,
    #[serde(default)]
    pub valid_until: Option<StoryPlacement>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub supersedes_fact_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RelationshipUpdateEntry {
    pub character_a_id: String,
    pub character_b_id: String,
    pub trust_delta: i32,
    pub tension_delta: i32,
    pub reason: String,
}

impl<'de> Deserialize<'de> for CharacterStatePatchEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Structured {
                character_id: String,
                changes: CharacterStatePatch,
            },
            Summary {
                character_id: String,
                #[serde(alias = "summary")]
                state: String,
            },
        }

        match Repr::deserialize(deserializer)? {
            Repr::Structured {
                character_id,
                changes,
            } => Ok(Self {
                character_id,
                changes,
            }),
            Repr::Summary {
                character_id,
                state,
            } => Ok(Self {
                character_id,
                changes: CharacterStatePatch {
                    emotional_state: BTreeMap::new(),
                    goals: None,
                    status: None,
                    notes: Some(vec![state.clone()]),
                    source_summary: Some(state),
                },
            }),
        }
    }
}

impl<'de> Deserialize<'de> for CanonicalFactEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Structured {
                #[serde(default)]
                fact_type: Option<String>,
                #[serde(default)]
                key: Option<String>,
                #[serde(default)]
                value: Option<String>,
                #[serde(default)]
                subject_table: Option<String>,
                #[serde(default)]
                subject_id: Option<String>,
                #[serde(default)]
                predicate: Option<String>,
                #[serde(default)]
                value_kind: Option<String>,
                #[serde(default)]
                value_text: Option<String>,
                #[serde(default)]
                value_number: Option<f64>,
                #[serde(default)]
                value_unit: Option<String>,
                #[serde(default)]
                value_json: Option<serde_json::Value>,
                #[serde(default)]
                aliases: Vec<String>,
                #[serde(default)]
                scope: Option<CanonicalFactScope>,
                #[serde(default)]
                valid_from: Option<StoryPlacement>,
                #[serde(default)]
                valid_until: Option<StoryPlacement>,
                #[serde(default)]
                context: Option<String>,
                #[serde(default)]
                supersedes_fact_id: Option<String>,
            },
        }

        match Repr::deserialize(deserializer)? {
            Repr::Structured {
                fact_type,
                key,
                value,
                subject_table,
                subject_id,
                predicate,
                value_kind,
                value_text,
                value_number,
                value_unit,
                value_json,
                aliases,
                scope,
                valid_from,
                valid_until,
                context,
                supersedes_fact_id,
            } => Ok(Self {
                fact_type,
                key,
                value,
                subject_table,
                subject_id,
                predicate,
                value_kind,
                value_text,
                value_number,
                value_unit,
                value_json,
                aliases,
                scope,
                valid_from,
                valid_until,
                context,
                supersedes_fact_id,
            }),
        }
    }
}

impl<'de> Deserialize<'de> for RelationshipUpdateEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Structured {
                character_a_id: String,
                character_b_id: String,
                trust_delta: i32,
                tension_delta: i32,
                reason: String,
            },
            Summary {
                #[serde(alias = "character_id_1")]
                character_a_id: String,
                #[serde(alias = "character_id_2")]
                character_b_id: String,
                #[serde(default)]
                trust_delta: Option<i32>,
                #[serde(default)]
                tension_delta: Option<i32>,
                #[serde(alias = "summary")]
                reason: String,
            },
        }

        match Repr::deserialize(deserializer)? {
            Repr::Structured {
                character_a_id,
                character_b_id,
                trust_delta,
                tension_delta,
                reason,
            } => Ok(Self {
                character_a_id,
                character_b_id,
                trust_delta,
                tension_delta,
                reason,
            }),
            Repr::Summary {
                character_a_id,
                character_b_id,
                trust_delta,
                tension_delta,
                reason,
            } => Ok(Self {
                character_a_id,
                character_b_id,
                trust_delta: trust_delta.unwrap_or(0),
                tension_delta: tension_delta.unwrap_or(0),
                reason,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneChangesOutput {
    pub scene_id: String,
    #[serde(default)]
    pub character_states: Vec<CommitSceneCharacterStateResult>,
    #[serde(default)]
    pub canonical_facts: Vec<CommitSceneCanonicalFactResult>,
    #[serde(default)]
    pub relationship_updates: Vec<CommitSceneRelationshipResult>,
    #[serde(default)]
    pub world_rule_hits: Vec<WorldRuleHit>,
    #[serde(default)]
    pub findings_summary: CommitSceneFindingsSummary,
    /// Continuity findings (canonical-fact contradictions + retcons) detected by
    /// the write-time gate. Error-severity entries are what would block the
    /// commit under `BlockErrors`.
    #[serde(default)]
    pub blocking_continuity_findings: Vec<ConsistencyIssue>,
    /// Structured retcon findings surfaced by the write-time gate.
    #[serde(default)]
    pub retcon_findings: Vec<RetconFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct CommitSceneFindingsSummary {
    #[serde(default)]
    pub total_count: usize,
    #[serde(default)]
    pub error_count: usize,
    #[serde(default)]
    pub warning_count: usize,
    #[serde(default)]
    pub info_count: usize,
    #[serde(default)]
    pub by_check: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneCharacterStateResult {
    pub character_id: String,
    #[serde(default)]
    pub state_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneCanonicalFactResult {
    pub fact_type: String,
    pub key: String,
    #[serde(default)]
    pub canonical_fact_id: Option<String>,
    #[serde(default)]
    pub superseded_fact_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneRelationshipResult {
    pub character_a_id: String,
    pub character_b_id: String,
    #[serde(default)]
    pub relationship_id: Option<String>,
    #[serde(default)]
    pub trust: Option<i32>,
    #[serde(default)]
    pub tension: Option<i32>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateBookInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateBookOutput {
    pub book_id: String,
    pub book_number: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateChapterInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub book_number: Option<i32>,
    pub book_id: Option<String>,
    pub chapter_number: Option<i32>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateChapterOutput {
    pub chapter_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateBranchInput {
    pub project_id: String,
    pub name: String,
    pub branch_type: String,
    pub description: Option<String>,
    pub parent_branch_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateBranchOutput {
    pub branch_id: String,
    pub parent_branch_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SwitchBranchInput {
    pub project_id: String,
    pub branch_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SwitchBranchOutput {
    pub branch_id: String,
    pub branch_name: String,
}

/// Set (or clear) the project's narrator-voice directive — the prose-level
/// narration style that governs the whole reading experience, distinct from
/// per-character dialogue voice profiles. Pass all-empty fields to clear it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetNarratorVoiceInput {
    pub project_id: String,
    #[serde(default)]
    pub narrator_voice: crate::style::NarratorVoice,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetNarratorVoiceOutput {
    pub project_id: String,
    pub narrator_voice: crate::style::NarratorVoice,
    /// True when the call cleared the directive (all fields empty).
    pub cleared: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateSavePointInput {
    pub project_id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateSavePointOutput {
    pub save_point_id: String,
    pub branch_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RestoreSavePointInput {
    pub project_id: String,
    pub save_point_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RestoreSavePointOutput {
    pub save_point_id: String,
    pub branch_id: String,
    pub backup_save_point_id: String,
    pub status: String,
    pub restored_tables: usize,
    pub restored_records: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BranchSummary {
    pub branch_id: String,
    pub name: String,
    pub status: String,
    pub branch_type: Option<String>,
    pub description: Option<String>,
    pub parent_branch_id: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiffBranchesInput {
    pub project_id: String,
    pub base_branch_id: String,
    pub compare_branch_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiffBranchesOutput {
    pub base_branch: BranchSummary,
    pub compare_branch: BranchSummary,
    pub scene_diffs: Vec<SceneDiffItem>,
    pub character_state_diffs: Vec<CharacterStateDiffItem>,
    pub relationship_diffs: Vec<RelationshipDiffItem>,
    pub pacing_diffs: Vec<PacingDiffItem>,
    pub narrative_impact_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneDiffItem {
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub base_summary: Option<String>,
    pub compare_summary: Option<String>,
    pub change_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CharacterStateDiffItem {
    pub character_id: String,
    pub character_name: String,
    pub position: String,
    pub base_status: Vec<String>,
    pub compare_status: Vec<String>,
    pub base_goals: Vec<String>,
    pub compare_goals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RelationshipDiffItem {
    pub source_character_id: String,
    pub target_character_id: String,
    pub relationship_type: String,
    pub base_trust: Option<i32>,
    pub compare_trust: Option<i32>,
    pub base_tension: Option<i32>,
    pub compare_tension: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PacingDiffItem {
    pub character_arc_id: String,
    pub tracker_id: String,
    pub base_progress: Option<f64>,
    pub compare_progress: Option<f64>,
    pub base_status: Option<String>,
    pub compare_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MergeBranchInput {
    pub project_id: String,
    pub source_branch_id: String,
    pub target_branch_id: Option<String>,
    pub merge_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MergeConflictItem {
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub source_scene_id: String,
    pub target_scene_id: String,
    pub target_origin_branch_id: String,
    pub source_summary: String,
    pub target_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MergeBranchOutput {
    pub source_branch_id: String,
    pub target_branch_id: String,
    pub merge_type: String,
    pub applied_scenes: usize,
    pub applied_character_states: usize,
    pub applied_relationships: usize,
    pub applied_pacing_trackers: usize,
    pub has_conflicts: bool,
    pub conflicts: Vec<MergeConflictItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviseSceneInput {
    pub project_id: String,
    pub scene_id: String,
    pub full_text: String,
    pub summary: String,
    pub content_rating: ContentRating,
    pub tone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviseSceneOutput {
    pub scene_id: String,
    pub states_invalidated: Vec<RevisionStateInvalidation>,
    pub downstream_scenes_flagged: Vec<RevisionSceneFlag>,
    pub pacing_impact: Vec<RevisionPacingImpact>,
    #[serde(default)]
    pub diff: Vec<TextDiffChunk>,
    #[serde(default)]
    pub byte_offsets_changed: Vec<TextByteRange>,
    pub chars_added: usize,
    pub chars_deleted: usize,
    #[serde(default)]
    pub world_rule_hits: Vec<WorldRuleHit>,
    #[serde(default)]
    pub voice_drift: Vec<VoiceDriftFinding>,
    #[serde(default)]
    pub retcon_findings: Vec<RetconFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RetconFinding {
    OutOfBandsKnowledge {
        character_id: String,
        knowledge_summary: String,
        learned_at: StoryPlacement,
        message: String,
    },
    MissingFutureKnowledgeAnchor {
        intervention_id: String,
        intervention_title: String,
        message: String,
    },
    DeadCharacterAct {
        character_id: String,
        character_name: String,
        status: String,
        message: String,
    },
    /// A character references, in prose, a `knowledge_fact` they do not learn
    /// until a later story position. High-precision/low-recall advisory.
    PrematureKnowledge {
        character_id: String,
        fact: String,
        learned_at: StoryPlacement,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RevisionStateInvalidation {
    pub state_id: String,
    pub character_id: String,
    pub position: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RevisionSceneFlag {
    pub scene_id: String,
    pub position: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RevisionPacingImpact {
    pub character_arc_id: String,
    pub tracker_id: String,
    pub status: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GenerateAlternativesInput {
    pub project_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub character_ids: Vec<String>,
    pub location_id: String,
    pub alternatives: Option<usize>,
    pub variation_strategy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GenerateAlternativesOutput {
    pub context: SceneContextOutput,
    pub alternatives: Vec<GeneratedAlternative>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GeneratedAlternative {
    pub branch_id: String,
    pub branch_name: String,
    pub summary: String,
    pub scene_id: String,
    pub variation_strategy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompareAlternativesInput {
    pub project_id: String,
    pub branch_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompareAlternativesOutput {
    pub alternatives: Vec<AlternativeComparison>,
    pub recommended_branch_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AlternativeComparison {
    pub branch_id: String,
    pub branch_name: String,
    pub summary: String,
    pub quality_score: i32,
    pub strongest_trait: String,
    pub pacing_note: String,
    pub hook_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SelectAlternativeInput {
    pub project_id: String,
    pub branch_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SelectAlternativeOutput {
    pub selected_branch_id: String,
    pub target_branch_id: String,
    pub merge_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListRevisionMarkersInput {
    pub project_id: String,
    pub scene_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListRevisionMarkersOutput {
    pub scene_id: String,
    pub markers: Vec<RevisionMarkerSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RevisionMarkerSummary {
    pub marker_id: String,
    pub marker_type: String,
    pub target_record_id: Option<String>,
    pub position: String,
    pub note: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolveRevisionMarkerInput {
    pub marker_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolveRevisionMarkerOutput {
    pub marker_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateFactionInput {
    pub project_id: String,
    pub name: String,
    pub faction_type: String,
    pub realm: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateFactionOutput {
    pub faction_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateReligionInput {
    pub project_id: String,
    pub name: String,
    pub deity_or_principle: String,
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateReligionOutput {
    pub religion_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateEconomyInput {
    pub project_id: String,
    pub name: String,
    pub realm: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub scarce_resources: Vec<String>,
    #[serde(default)]
    pub trade_goods: Vec<String>,
    pub currency: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateEconomyOutput {
    pub economy_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateTermInput {
    pub project_id: String,
    pub term_text: String,
    pub pronunciation: Option<String>,
    pub definition: String,
    pub usage_context: Option<String>,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateTermOutput {
    pub term_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateTermItem {
    pub term_text: String,
    pub pronunciation: Option<String>,
    pub definition: String,
    pub usage_context: Option<String>,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateTermsInput {
    pub project_id: String,
    #[serde(default)]
    pub items: Vec<BatchCreateTermItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateTermsOutput {
    #[serde(default)]
    pub term_ids: Vec<String>,
    pub created: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateEntityInput {
    pub entity_type: String,
    pub entity_id: String,
    #[schemars(schema_with = "any_object_schema")]
    pub changes: serde_json::Value,
}

fn any_object_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    serde_json::from_value(serde_json::json!({
        "type": "object",
        "description": "Key-value map of fields to update on the entity"
    }))
    .expect("valid schema")
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateEntityOutput {
    pub entity_type: String,
    pub entity_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateWorldRuleInput {
    pub world_rule_id: String,
    #[schemars(schema_with = "any_object_schema")]
    pub changes: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateWorldRuleOutput {
    pub world_rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetCharacterVoiceProfileInput {
    pub project_id: String,
    pub character_id: String,
    pub branch_id: String,
    pub profile: CharacterVoiceProfileData,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetCharacterVoiceProfileOutput {
    pub character_id: String,
    pub branch_id: String,
    pub profile: CharacterVoiceProfileData,
    #[serde(default)]
    pub activity_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchSetCharacterVoiceProfileItem {
    pub character_id: String,
    pub profile: CharacterVoiceProfileData,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchSetCharacterVoiceProfilesInput {
    pub project_id: String,
    pub branch_id: String,
    #[serde(default)]
    pub items: Vec<BatchSetCharacterVoiceProfileItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchSetCharacterVoiceProfilesOutput {
    #[serde(default)]
    pub profiles: Vec<SetCharacterVoiceProfileOutput>,
    pub updated: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveEntityInput {
    pub entity_type: String,
    pub entity_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveEntityOutput {
    pub entity_type: String,
    pub entity_id: String,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatePlotLineInput {
    pub project_id: String,
    pub name: String,
    pub plot_type: String,
    pub summary: String,
    pub status: Option<String>,
    #[serde(default)]
    pub convergence_points: Vec<StoryPlacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatePlotLineOutput {
    pub plot_line_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateConflictInput {
    pub project_id: String,
    pub name: String,
    pub conflict_type: String,
    pub stakes: String,
    #[serde(default)]
    pub escalation_stages: Vec<String>,
    pub expected_total_cycles: Option<i32>,
    #[serde(default)]
    pub try_fail_cycles: Vec<TryFailCycleStep>,
    #[serde(default)]
    pub stated_consequences: Vec<StatedConsequence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateConflictOutput {
    pub conflict_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateThemeInput {
    pub project_id: String,
    pub theme_statement: String,
    pub thesis_antithesis: String,
    pub introduction_point: Option<StoryPlacement>,
    pub resolution_point: Option<StoryPlacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateThemeOutput {
    pub theme_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateMotifInput {
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub max_uses_per_chapter: Option<i32>,
    #[serde(default)]
    pub connected_theme_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateMotifOutput {
    pub motif_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateMotifItem {
    pub name: String,
    pub description: String,
    pub max_uses_per_chapter: Option<i32>,
    #[serde(default)]
    pub connected_theme_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateMotifsInput {
    pub project_id: String,
    #[serde(default)]
    pub items: Vec<BatchCreateMotifItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateMotifsOutput {
    #[serde(default)]
    pub motif_ids: Vec<String>,
    pub created: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateNarrativePromiseInput {
    pub project_id: String,
    pub promise_type: String,
    pub description: String,
    pub planted_at: StoryPlacement,
    pub planned_payoff: Option<StoryPlacement>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateNarrativePromiseOutput {
    pub narrative_promise_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateNarrativePromiseItem {
    pub promise_type: String,
    pub description: String,
    pub planted_at: StoryPlacement,
    pub planned_payoff: Option<StoryPlacement>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateNarrativePromisesInput {
    pub project_id: String,
    #[serde(default)]
    pub items: Vec<BatchCreateNarrativePromiseItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateNarrativePromisesOutput {
    #[serde(default)]
    pub narrative_promise_ids: Vec<String>,
    pub created: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdatePromiseStatusInput {
    #[serde(alias = "promise_id")]
    pub narrative_promise_id: String,
    #[serde(alias = "new_status")]
    pub status: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdatePromiseStatusOutput {
    pub narrative_promise_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateCharacterArcInput {
    pub project_id: String,
    pub character_id: String,
    pub arc_type: String,
    pub starting_state: String,
    pub ending_state: String,
    #[serde(default)]
    pub milestones: Vec<CharacterArcMilestone>,
    pub thematic_purpose: String,
    #[serde(default)]
    pub connected_theme_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateCharacterArcOutput {
    pub character_arc_id: String,
    pub pacing_tracker_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatePacingConfigInput {
    pub project_id: String,
    pub total_planned_books: i32,
    pub avg_chapters_per_book: i32,
    pub avg_scenes_per_chapter: i32,
    pub tension_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatePacingConfigOutput {
    pub pacing_config_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatePacingCurveInput {
    pub project_id: String,
    pub book_number: i32,
    pub act_breakpoints: BTreeMap<String, f64>,
    pub scene_type_density: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatePacingCurveOutput {
    pub pacing_curve_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetArcPacingConstraintsInput {
    pub project_id: String,
    pub character_arc_id: String,
    pub per_book_budget: BTreeMap<String, f64>,
    pub max_progress_per_chapter: Option<f64>,
    pub milestone_spacing: Option<i32>,
    pub sprint_allowance: Option<i32>,
    pub regression_budget: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetArcPacingConstraintsOutput {
    pub pacing_tracker_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct PlanChapterSceneInput {
    pub scene_order: i32,
    pub summary: String,
    #[serde(default)]
    pub beat_structure: Vec<String>,
    #[serde(default)]
    pub character_ids: Vec<String>,
    /// Record id returned by create_location (e.g. "location:xyz789").
    /// Required by authoring_prepare_run / authoring_start_run for drafting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_id: Option<String>,
    /// Planned content rating for this scene. Required by authoring_prepare_run
    /// / authoring_start_run so draft/review routing can select safe backends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_rating: Option<ContentRating>,
    pub purpose: String,
    #[serde(default)]
    pub research_required: Option<bool>,
    #[serde(default)]
    pub research_tags: Vec<String>,
    #[serde(default)]
    pub explicit_query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanChapterInput {
    pub project_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub pov_character_id: Option<String>,
    pub synopsis: String,
    #[serde(default)]
    pub target_theme_ids: Vec<String>,
    #[serde(default)]
    pub target_conflict_ids: Vec<String>,
    #[serde(default)]
    pub target_plot_line_ids: Vec<String>,
    #[serde(default)]
    pub scenes: Vec<PlanChapterSceneInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanChapterOutput {
    pub chapter_plan_id: String,
    /// Genre-voice mismatches detected in the planned scenes' tone/beat
    /// descriptors against the project style contract — caught at planning
    /// time, before any prose is drafted. Empty when the plan is on-genre or
    /// the project declares no style signal.
    #[serde(default)]
    pub style_warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AnnotatedBeatInput {
    pub beat_type: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AnnotateSceneBeatsInput {
    pub project_id: String,
    pub scene_id: String,
    #[serde(default)]
    pub beats: Vec<AnnotatedBeatInput>,
    #[serde(default)]
    pub motif_ids: Vec<String>,
    #[serde(default)]
    pub theme_ids: Vec<String>,
    #[serde(default)]
    pub conflict_ids: Vec<String>,
    /// Realized 0.0-1.0 scene intensity, for pacing-drift detection. When None
    /// on a re-annotation, the previously recorded value is preserved.
    #[serde(default)]
    pub intensity: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AnnotateSceneBeatsOutput {
    pub scene_annotation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SaveSummaryInput {
    pub project_id: String,
    #[serde(default)]
    pub book_number: i32,
    #[serde(default)]
    pub chapter_number: i32,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default, alias = "chapter_id")]
    pub entity_id: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub key_events: Vec<String>,
    #[serde(default)]
    pub character_changes: Vec<String>,
    #[serde(default)]
    pub relationship_shifts: Vec<String>,
    #[serde(default)]
    pub arc_advances: Vec<String>,
    #[serde(default)]
    pub promise_events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SaveSummaryOutput {
    pub chapter_summary_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetBookOutlineInput {
    pub project_id: String,
    #[serde(default)]
    pub book_number: i32,
    #[serde(default)]
    pub book_id: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetBookOutlineOutput {
    pub outline: BookOutline,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetChapterOutlineInput {
    pub project_id: String,
    #[serde(default)]
    pub book_number: i32,
    #[serde(default)]
    pub chapter_number: i32,
    #[serde(default, alias = "chapter_id")]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub beats: Vec<ChapterOutlineBeat>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetChapterOutlineOutput {
    pub outline: ChapterOutline,
}

/// Internal enum used by service logic for scope matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ConsistencyScope {
    Full,
    Book {
        book_number: i32,
    },
    ChapterRange {
        start_book_number: i32,
        start_chapter_number: i32,
        end_book_number: i32,
        end_chapter_number: i32,
    },
}

/// Flat scope specification that survives JSON Schema sanitization.
/// `scope_type` must be `"full"`, `"book"`, or `"chapter_range"`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsistencyScopeInput {
    /// One of: "full", "book", "chapter_range"
    pub scope_type: String,
    /// Required when scope_type is "book"
    pub book_number: Option<i32>,
    /// Required when scope_type is "chapter_range"
    pub start_book_number: Option<i32>,
    /// Required when scope_type is "chapter_range"
    pub start_chapter_number: Option<i32>,
    /// Required when scope_type is "chapter_range"
    pub end_book_number: Option<i32>,
    /// Required when scope_type is "chapter_range"
    pub end_chapter_number: Option<i32>,
}

impl ConsistencyScopeInput {
    pub fn full() -> Self {
        Self {
            scope_type: "full".to_string(),
            book_number: None,
            start_book_number: None,
            start_chapter_number: None,
            end_book_number: None,
            end_chapter_number: None,
        }
    }

    pub fn book(book_number: i32) -> Self {
        Self {
            scope_type: "book".to_string(),
            book_number: Some(book_number),
            start_book_number: None,
            start_chapter_number: None,
            end_book_number: None,
            end_chapter_number: None,
        }
    }

    pub fn chapter_range(
        start_book_number: i32,
        start_chapter_number: i32,
        end_book_number: i32,
        end_chapter_number: i32,
    ) -> Self {
        Self {
            scope_type: "chapter_range".to_string(),
            book_number: None,
            start_book_number: Some(start_book_number),
            start_chapter_number: Some(start_chapter_number),
            end_book_number: Some(end_book_number),
            end_chapter_number: Some(end_chapter_number),
        }
    }

    pub fn to_scope(&self) -> Result<ConsistencyScope, String> {
        match self.scope_type.as_str() {
            "full" => Ok(ConsistencyScope::Full),
            "book" => {
                let book_number = self
                    .book_number
                    .ok_or("book_number required for scope_type 'book'")?;
                Ok(ConsistencyScope::Book { book_number })
            }
            "chapter_range" => {
                let start_book_number = self
                    .start_book_number
                    .ok_or("start_book_number required for scope_type 'chapter_range'")?;
                let start_chapter_number = self
                    .start_chapter_number
                    .ok_or("start_chapter_number required for scope_type 'chapter_range'")?;
                let end_book_number = self
                    .end_book_number
                    .ok_or("end_book_number required for scope_type 'chapter_range'")?;
                let end_chapter_number = self
                    .end_chapter_number
                    .ok_or("end_chapter_number required for scope_type 'chapter_range'")?;
                Ok(ConsistencyScope::ChapterRange {
                    start_book_number,
                    start_chapter_number,
                    end_book_number,
                    end_chapter_number,
                })
            }
            other => Err(format!(
                "invalid scope_type '{other}': expected 'full', 'book', or 'chapter_range'"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckConsistencyInput {
    pub project_id: String,
    pub scope: ConsistencyScopeInput,
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub severity_filter: Vec<String>,
    #[serde(default)]
    pub deep_check: Option<bool>,
    /// Optional subject narrowing. Each entry is a record id of the form
    /// `<table>:<id>` (for example `character:abc123`). When non-empty, the
    /// check runs only against scenes that reference at least one listed
    /// subject. Subject resolution mirrors `find_scenes_referencing` and
    /// supports characters, locations, factions, religions, economies, terms,
    /// plot lines, conflicts, themes, motifs, world rules, narrative
    /// promises, and system overlays.
    #[serde(default)]
    pub subjects: Vec<String>,
    /// Render format for the optional `markdown` field on the response.
    /// When `markdown`, the output includes a markdown-rendered report.
    /// JSON callers can ignore this and read `issues` and `report_sections`
    /// directly. Defaults to `json`.
    #[serde(default)]
    pub format: Option<ContextFormat>,
    /// Token budget for inline markdown trimming. Errors are protected from
    /// trimming; warnings and info findings are dropped from the rendered
    /// markdown when the budget would be exceeded. Has no effect on the
    /// structured `issues` and `report_sections` fields.
    #[serde(default)]
    pub budget_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsistencyIssue {
    pub severity: String,
    pub check_type: String,
    pub message: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
    pub suggested_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsistencySummary {
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsistencySceneFindings {
    pub scene_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub findings: Vec<ConsistencyIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsistencySection {
    /// Stable validator identifier (matches the validator's `check_type`).
    pub validator_id: String,
    pub scenes: Vec<ConsistencySceneFindings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckConsistencyOutput {
    pub issues: Vec<ConsistencyIssue>,
    pub summary: ConsistencySummary,
    /// Per-validator grouping of Phase 4 validator findings. Each section
    /// lists the scenes that produced findings, with positions, in
    /// `(book, chapter, scene_order)` order. Non-validator checks (scene
    /// spine, narrative promise, pacing, etc.) appear only in `issues`.
    /// Sections for validators that produced no findings are omitted.
    #[serde(default)]
    pub report_sections: Vec<ConsistencySection>,
    /// Markdown rendering of the report. Populated when the input
    /// `format` is `markdown`. Errors are listed under `## Hard
    /// constraints` and are never trimmed; warnings and info findings
    /// follow under per-validator headings and may be dropped if the
    /// budget would be exceeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchBibleInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub query: String,
    pub limit: Option<usize>,
    #[serde(default)]
    pub mode: Option<SearchBibleMode>,
    #[serde(default)]
    pub field: Option<SearchBibleField>,
    #[serde(default)]
    pub subject_table: Option<String>,
    /// Render format. Defaults to json when omitted.
    #[serde(default)]
    pub format: Option<ContextFormat>,
    /// Preferred token budget for temporary inline trimming.
    #[serde(default)]
    pub budget_tokens: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchBibleMode {
    Semantic,
    Exact,
    Fuzzy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchBibleField {
    Name,
    Content,
    Tags,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RebuildSearchIndexInput {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RebuildSearchIndexOutput {
    pub indexed_records: usize,
    pub embedding_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BackfillSceneSourceOffsetsInput {
    pub project_id: String,
    pub branch_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BackfillSceneSourceOffsetsOutput {
    pub scanned_links: usize,
    pub updated_links: usize,
    pub unresolved_links: usize,
    pub skipped_links: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PullChapterFromFileInput {
    pub chapter_id: String,
    pub source_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PushChapterToFileInput {
    pub chapter_id: String,
    pub target_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceBridgeDivergenceStatus {
    pub scene_id: String,
    pub source_path: String,
    pub kind: DivergenceKind,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PullReport {
    pub chapter_id: String,
    pub source_path: String,
    pub source_size_bytes: usize,
    pub scenes: Vec<PullSceneEntry>,
    pub unmatched_text_ranges: Vec<TextByteRange>,
    pub status: PullStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum PullStatus {
    Clean,
    Diverged,
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PullSceneEntry {
    pub scene_id: String,
    pub position: u32,
    pub byte_range_in_source: TextByteRange,
    pub status: SceneSyncStatus,
    pub diff: Option<TextDiffSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum SceneSyncStatus {
    Match,
    Diverged,
    Updated,
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TextDiffSummary {
    pub chars_added: usize,
    pub chars_deleted: usize,
    pub chunks: Vec<TextDiffChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PushReport {
    pub chapter_id: String,
    pub target_path: String,
    pub scenes: Vec<PushSceneEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PushSceneEntry {
    pub scene_id: String,
    pub position: u32,
    pub byte_range_in_file: TextByteRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchBibleResultItem {
    pub entity_type: String,
    pub entity_id: String,
    pub title: String,
    pub excerpt: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchBibleOutput {
    pub results: Vec<SearchBibleResultItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub canonical_facts: Vec<CanonicalFactReadModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TextByteRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextDiffKind {
    Insert,
    Delete,
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldRuleSeverity {
    Possible,
    Likely,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct WorldRuleHit {
    pub rule_id: String,
    pub byte_range: TextByteRange,
    pub severity: WorldRuleSeverity,
    pub surrounding_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VoiceDriftKind {
    ForbiddenPhrase,
    LexicalSignatureMissing,
    ToneMarkerMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct VoiceDriftFinding {
    pub character_id: String,
    pub character_name: String,
    pub kind: VoiceDriftKind,
    pub message: String,
    #[serde(default)]
    pub forbidden_phrase: Option<String>,
    #[serde(default)]
    pub matched_text: Option<String>,
    #[serde(default)]
    pub byte_range: Option<TextByteRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TextDiffChunk {
    pub kind: TextDiffKind,
    pub text: String,
    #[serde(default)]
    pub byte_range: Option<TextByteRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FindScenesReferencingInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    pub query: SceneReferenceQuery,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SceneReferenceQuery {
    Subject { subject_id: String },
    Phrase { phrase: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SceneReferenceByteRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneReferenceItem {
    pub scene_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub snippet: String,
    #[serde(default)]
    pub byte_range: Option<SceneReferenceByteRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FindScenesReferencingOutput {
    pub results: Vec<SceneReferenceItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateFutureKnowledgeInput {
    pub project_id: String,
    pub character_id: String,
    pub knowledge_summary: String,
    pub source: String,
    pub learned_at: StoryPlacement,
    pub expires_at: Option<StoryPlacement>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateFutureKnowledgeOutput {
    pub future_knowledge_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateTimelineEventInput {
    pub project_id: String,
    pub title: String,
    pub event_type: String,
    pub placement: StoryPlacement,
    pub summary: String,
    #[serde(default)]
    pub related_entity_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateTimelineEventOutput {
    pub timeline_event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateTemporalInterventionInput {
    pub project_id: String,
    pub title: String,
    pub intervention_type: String,
    pub source_event_id: Option<String>,
    pub target_event_id: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub consequences: Vec<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateTemporalInterventionOutput {
    pub temporal_intervention_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateSystemOverlayInput {
    pub project_id: String,
    pub system_name: String,
    pub system_type: String,
    pub rules: String,
    pub visibility: String,
    pub progression_currency: Option<String>,
    #[serde(default)]
    pub stats: Vec<String>,
    #[serde(default)]
    pub advancement_tiers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateSystemOverlayOutput {
    pub system_overlay_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunDualPersonaReviewInput {
    pub project_id: String,
    pub branch_id: Option<String>,
    pub scene_id: String,
    pub rounds: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PersonaReviewNotes {
    pub persona: String,
    #[serde(default)]
    pub strengths: Vec<String>,
    #[serde(default)]
    pub concerns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DualPersonaReviewRound {
    pub round: usize,
    pub literary_critic: PersonaReviewNotes,
    pub craft_technician: PersonaReviewNotes,
    /// Genre-voice judgement from the perspective of the declared genre's
    /// target reader (populated from the project style contract). Defaulted for
    /// backward compatibility with reviews persisted before this persona
    /// existed and empty when the project has no style signal.
    #[serde(default)]
    pub genre_reader: PersonaReviewNotes,
    #[serde(default)]
    pub priority_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunDualPersonaReviewOutput {
    pub scene_id: String,
    pub branch_id: String,
    pub rounds_completed: usize,
    pub review_id: String,
    pub status: String,
    #[serde(default)]
    pub review_rounds: Vec<DualPersonaReviewRound>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PersistedDualPersonaReview {
    pub review_id: String,
    pub scene_id: String,
    pub branch_id: String,
    pub rounds_completed: usize,
    pub status: String,
    #[serde(default)]
    pub review_rounds: Vec<DualPersonaReviewRound>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportSourceFormat {
    Txt,
    Md,
    Html,
    Epub,
    Docx,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportDuplicateStrategy {
    Reject,
    CreateNewSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportHydrationMode {
    NewProject,
    ExistingProject,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportSessionStatus {
    Pending,
    Running,
    ReviewNeeded,
    ReadyToHydrate,
    Hydrated,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportHydrationStatus {
    Pending,
    Running,
    Completed,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportPassName {
    StructuralAnalysis,
    EntityExtraction,
    EntityConsolidation,
    CharacterAnalysis,
    WorldExtraction,
    NarrativeAnalysis,
    FinalState,
    Hydration,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportConfidenceLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportPovGuessSource {
    Heuristic,
    Model,
    ReviewDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportEntityKind {
    Character,
    Location,
    Faction,
    Religion,
    Economy,
    Term,
    WorldRule,
    PlotLine,
    Conflict,
    NarrativePromise,
    Theme,
    Motif,
    CharacterArc,
    Knowledge,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportReviewItemKind {
    Structure,
    Entity,
    Character,
    World,
    Narrative,
    FinalState,
    Knowledge,
    ContentRating,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportReviewSeverity {
    Info,
    Warning,
    RequiresReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportReviewStatus {
    Open,
    Applied,
    Resolved,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportSessionProgress {
    pub total_documents: usize,
    pub processed_documents: usize,
    pub total_segments: usize,
    pub processed_segments: usize,
    pub total_review_items: usize,
    pub open_review_items: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportSessionSummary {
    pub session_id: String,
    pub project_id: Option<String>,
    pub target_branch_id: Option<String>,
    pub status: ImportSessionStatus,
    pub active_pass: ImportPassName,
    pub source_format: Option<ImportSourceFormat>,
    pub hydrate_mode: ImportHydrationMode,
    pub progress: ImportSessionProgress,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportSourceDocumentSummary {
    pub document_id: String,
    pub display_name: String,
    pub source_path: String,
    pub copied_path: String,
    pub source_format: ImportSourceFormat,
    pub original_sha256: String,
    pub normalized_sha256: String,
    pub word_count: usize,
    pub chapter_hint: Option<String>,
    pub source_order: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportPovGuess {
    pub character_name: Option<String>,
    pub cluster_id: Option<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
    pub source: ImportPovGuessSource,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportSceneSlice {
    pub segment_id: String,
    pub chapter_segment_id: Option<String>,
    pub scene_index: usize,
    pub label: Option<String>,
    pub start_offset: usize,
    pub end_offset: usize,
    pub word_count: usize,
    pub character_count: usize,
    pub pov_guess: Option<ImportPovGuess>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportChapterSlice {
    pub segment_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub title: Option<String>,
    pub start_offset: usize,
    pub end_offset: usize,
    pub word_count: usize,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
    #[serde(default)]
    pub scenes: Vec<ImportSceneSlice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportStructuralAnalysisSummary {
    #[serde(default)]
    pub source_documents: Vec<ImportSourceDocumentSummary>,
    #[serde(default)]
    pub chapters: Vec<ImportChapterSlice>,
    pub review_items_created: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportEntityMentionSummary {
    pub mention_id: String,
    pub segment_id: String,
    pub entity_kind: ImportEntityKind,
    pub surface_form: String,
    pub normalized_name: String,
    pub alias_hint: Option<String>,
    pub surrounding_text: Option<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportEntityExtractionReport {
    #[serde(default)]
    pub mentions: Vec<ImportEntityMentionSummary>,
    #[serde(default)]
    pub completed_segment_ids: Vec<String>,
    pub review_items_created: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportEntityClusterSummary {
    pub cluster_id: String,
    pub entity_kind: ImportEntityKind,
    pub canonical_name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub first_segment_id: Option<String>,
    pub last_segment_id: Option<String>,
    pub mention_count: usize,
    pub importance_rank: i32,
    pub merge_confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
    pub review_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportEntityConsolidationReport {
    #[serde(default)]
    pub clusters: Vec<ImportEntityClusterSummary>,
    pub review_items_created: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportCharacterRelationshipInference {
    pub other_character_cluster_id: String,
    pub summary: String,
    pub trust_signal: Option<String>,
    pub tension_signal: Option<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportCharacterStateTrajectoryPoint {
    pub segment_id: String,
    pub placement: Option<StoryPlacement>,
    pub summary: String,
    #[serde(default)]
    pub emotional_state: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub goals: Vec<String>,
    #[serde(default)]
    pub status: Vec<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportCharacterDossierSummary {
    pub cluster_id: String,
    pub canonical_name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub importance_rank: i32,
    pub voice_profile: CharacterVoiceProfileData,
    pub emotional_profile: CharacterEmotionalProfileData,
    #[serde(default)]
    pub state_trajectory: Vec<ImportCharacterStateTrajectoryPoint>,
    #[serde(default)]
    pub relationship_inferences: Vec<ImportCharacterRelationshipInference>,
    #[serde(default)]
    pub decision_patterns: Vec<String>,
    #[serde(default)]
    pub dialogue_samples: Vec<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportWorldRuleCandidate {
    pub rule_name: String,
    pub rule_type: String,
    pub description: String,
    #[serde(default)]
    pub source_segment_ids: Vec<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportLocationCandidate {
    pub cluster_id: Option<String>,
    pub name: String,
    pub kind: String,
    pub realm: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub source_segment_ids: Vec<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportWorldEntityCandidate {
    pub entity_kind: ImportEntityKind,
    pub cluster_id: Option<String>,
    pub canonical_name: String,
    pub summary: String,
    pub realm: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source_segment_ids: Vec<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportSystemSignalSummary {
    pub signal_type: String,
    pub summary: String,
    #[serde(default)]
    pub source_segment_ids: Vec<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportWorldDossierSummary {
    #[serde(default)]
    pub world_rules: Vec<ImportWorldRuleCandidate>,
    #[serde(default)]
    pub locations: Vec<ImportLocationCandidate>,
    #[serde(default)]
    pub entities: Vec<ImportWorldEntityCandidate>,
    #[serde(default)]
    pub system_signals: Vec<ImportSystemSignalSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportPlotLineCandidate {
    pub plot_line_id: Option<String>,
    pub name: String,
    pub plot_type: String,
    pub summary: String,
    pub status: Option<String>,
    #[serde(default)]
    pub convergence_points: Vec<StoryPlacement>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportConflictCandidate {
    pub conflict_id: Option<String>,
    pub name: String,
    pub conflict_type: String,
    pub stakes: String,
    #[serde(default)]
    pub escalation_stages: Vec<String>,
    #[serde(default)]
    pub try_fail_cycles: Vec<TryFailCycleStep>,
    #[serde(default)]
    pub stated_consequences: Vec<StatedConsequence>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportNarrativePromiseCandidate {
    pub narrative_promise_id: Option<String>,
    pub promise_type: String,
    pub description: String,
    pub planted_at: StoryPlacement,
    pub planned_payoff: Option<StoryPlacement>,
    pub status: String,
    #[serde(default)]
    pub notes: Vec<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportThemeCandidate {
    pub theme_statement: String,
    pub thesis_antithesis: String,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportMotifCandidate {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub connected_theme_statements: Vec<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportArcCandidate {
    pub character_cluster_id: String,
    pub arc_type: String,
    pub starting_state: String,
    pub ending_state: String,
    #[serde(default)]
    pub milestones: Vec<CharacterArcMilestone>,
    pub thematic_purpose: String,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportReaderContractDraft {
    pub reader_contract: ReaderContract,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportPacingHint {
    pub book_number: Option<i32>,
    pub summary: String,
    #[serde(default)]
    pub act_breakpoints: BTreeMap<String, f64>,
    #[serde(default)]
    pub scene_type_density: BTreeMap<String, f64>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportNarrativeDossierSummary {
    #[serde(default)]
    pub plot_lines: Vec<ImportPlotLineCandidate>,
    #[serde(default)]
    pub conflicts: Vec<ImportConflictCandidate>,
    #[serde(default)]
    pub narrative_promises: Vec<ImportNarrativePromiseCandidate>,
    #[serde(default)]
    pub arcs: Vec<ImportArcCandidate>,
    #[serde(default)]
    pub themes: Vec<ImportThemeCandidate>,
    #[serde(default)]
    pub motifs: Vec<ImportMotifCandidate>,
    pub reader_contract: ImportReaderContractDraft,
    #[serde(default)]
    pub pacing_hints: Vec<ImportPacingHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportFinalCharacterStateSnapshot {
    pub cluster_id: String,
    pub canonical_name: String,
    #[serde(default)]
    pub emotional_state: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub goals: Vec<String>,
    #[serde(default)]
    pub status: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportFinalRelationshipSnapshot {
    pub source_character_cluster_id: String,
    pub target_character_cluster_id: String,
    pub relationship_type: String,
    pub trust: i32,
    pub tension: i32,
    pub summary: String,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportFinalLocationStateSnapshot {
    pub location_name: String,
    pub controlling_faction: Option<String>,
    pub status: Option<String>,
    pub prosperity: Option<String>,
    pub stability: Option<String>,
    pub threat_level: Option<String>,
    #[serde(default)]
    pub sensory_details: Vec<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportPlotThreadSnapshot {
    pub name: String,
    pub status: String,
    pub next_expected_beat: Option<String>,
    #[serde(default)]
    pub source_cluster_ids: Vec<String>,
    pub confidence: f64,
    pub confidence_level: ImportConfidenceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportResumeSnapshotSummary {
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: Option<i32>,
    pub summary: String,
    #[serde(default)]
    pub characters: Vec<ImportFinalCharacterStateSnapshot>,
    #[serde(default)]
    pub relationships: Vec<ImportFinalRelationshipSnapshot>,
    #[serde(default)]
    pub locations: Vec<ImportFinalLocationStateSnapshot>,
    #[serde(default)]
    pub plot_threads: Vec<ImportPlotThreadSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportCorrectionPayload {
    Structural {
        chapter_number: Option<i32>,
        scene_order: Option<i32>,
        pov_character_name: Option<String>,
        #[serde(default)]
        segment_ids: Vec<String>,
    },
    Entity {
        entity_kind: ImportEntityKind,
        canonical_name: Option<String>,
        #[serde(default)]
        merge_cluster_ids: Vec<String>,
        #[serde(default)]
        split_aliases: Vec<String>,
    },
    Character {
        cluster_id: String,
        preferred_name: Option<String>,
        #[serde(default)]
        relationship_notes: Vec<String>,
    },
    World {
        entity_kind: ImportEntityKind,
        canonical_name: String,
        replacement_summary: Option<String>,
    },
    Narrative {
        target_id: Option<String>,
        status: Option<String>,
        thematic_purpose: Option<String>,
        note: Option<String>,
    },
    FinalState {
        target_record_id: Option<String>,
        corrected_summary: String,
    },
    Knowledge {
        character_id: String,
        fact: String,
        #[serde(default)]
        revoke_fact_ids: Vec<String>,
    },
    ContentRating {
        detected_rating: String,
        confidence: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportReviewItemSummary {
    pub review_item_id: String,
    pub session_id: String,
    pub pass_name: ImportPassName,
    pub kind: ImportReviewItemKind,
    pub severity: ImportReviewSeverity,
    pub status: ImportReviewStatus,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub related_segment_ids: Vec<String>,
    #[serde(default)]
    pub related_entity_ids: Vec<String>,
    pub confidence: Option<f64>,
    pub proposed_correction: Option<ImportCorrectionPayload>,
    pub resolver_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportHydrationRecordCount {
    pub entity_type: String,
    pub created: usize,
    pub updated: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportHydrationReport {
    pub session_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub status: ImportHydrationStatus,
    pub created_project: bool,
    pub target_branch_id: String,
    #[serde(default)]
    pub record_counts: Vec<ImportHydrationRecordCount>,
    #[serde(default)]
    pub skipped_sections: Vec<String>,
    #[serde(default)]
    pub review_item_ids: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeFactSummary {
    pub knowledge_fact_id: String,
    pub branch_id: String,
    pub character_id: String,
    pub fact: String,
    pub source_summary: String,
    pub learned_at: Option<StoryPlacement>,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub reader_visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportManuscriptInput {
    #[serde(default)]
    pub source_paths: Vec<String>,
    pub target_project_id: Option<String>,
    pub target_branch_id: Option<String>,
    pub create_project_name: Option<String>,
    pub source_format_hint: Option<ImportSourceFormat>,
    pub duplicate_strategy: Option<ImportDuplicateStrategy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportManuscriptOutput {
    pub session: ImportSessionSummary,
    pub structure: ImportStructuralAnalysisSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportStatusInput {
    pub project_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportStatusOutput {
    pub session: ImportSessionSummary,
    pub structure: Option<ImportStructuralAnalysisSummary>,
    pub entity_extraction: Option<ImportEntityExtractionReport>,
    pub entity_consolidation: Option<ImportEntityConsolidationReport>,
    #[serde(default)]
    pub characters: Vec<ImportCharacterDossierSummary>,
    pub world: Option<ImportWorldDossierSummary>,
    pub narrative: Option<ImportNarrativeDossierSummary>,
    pub final_state: Option<ImportResumeSnapshotSummary>,
    #[serde(default)]
    pub review_items: Vec<ImportReviewItemSummary>,
    pub hydration_report: Option<ImportHydrationReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportExtractEntitiesInput {
    pub project_id: String,
    pub session_id: String,
    #[serde(default)]
    pub segment_ids: Vec<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportExtractEntitiesOutput {
    pub session_id: String,
    pub report: ImportEntityExtractionReport,
    #[serde(default)]
    pub review_items: Vec<ImportReviewItemSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportConsolidateEntitiesInput {
    pub project_id: String,
    pub session_id: String,
    #[serde(default)]
    pub entity_kinds: Vec<ImportEntityKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportConsolidateEntitiesOutput {
    pub session_id: String,
    pub report: ImportEntityConsolidationReport,
    #[serde(default)]
    pub review_items: Vec<ImportReviewItemSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportAnalyzeCharacterInput {
    pub project_id: String,
    pub session_id: String,
    #[serde(default)]
    pub cluster_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportAnalyzeCharacterOutput {
    pub session_id: String,
    #[serde(default)]
    pub characters: Vec<ImportCharacterDossierSummary>,
    #[serde(default)]
    pub review_items: Vec<ImportReviewItemSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportExtractWorldInput {
    pub project_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportExtractWorldOutput {
    pub session_id: String,
    pub world: ImportWorldDossierSummary,
    #[serde(default)]
    pub review_items: Vec<ImportReviewItemSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportAnalyzeNarrativeInput {
    pub project_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportAnalyzeNarrativeOutput {
    pub session_id: String,
    pub narrative: ImportNarrativeDossierSummary,
    #[serde(default)]
    pub review_items: Vec<ImportReviewItemSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportComputeFinalStateInput {
    pub project_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportComputeFinalStateOutput {
    pub session_id: String,
    pub resume_snapshot: ImportResumeSnapshotSummary,
    #[serde(default)]
    pub review_items: Vec<ImportReviewItemSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportHydrateBibleInput {
    pub project_id: String,
    pub session_id: String,
    pub target_project_id: Option<String>,
    pub target_branch_id: Option<String>,
    pub create_project_name: Option<String>,
    pub hydrate_mode: Option<ImportHydrationMode>,
    pub include_scenes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportHydrateBibleOutput {
    pub session_id: String,
    pub report: ImportHydrationReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportReviewDecisionInput {
    pub review_item_id: String,
    pub resolution: ImportReviewStatus,
    pub correction: Option<ImportCorrectionPayload>,
    pub resolver_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportApplyReviewDecisionsInput {
    pub project_id: String,
    pub session_id: String,
    #[serde(default)]
    pub decisions: Vec<ImportReviewDecisionInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportApplyReviewDecisionsOutput {
    pub session_id: String,
    #[serde(default)]
    pub updated_review_items: Vec<ImportReviewItemSummary>,
    pub remaining_open_items: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecordKnowledgeInput {
    pub project_id: String,
    pub branch_id: Option<String>,
    pub character_id: String,
    pub fact: String,
    pub source_summary: String,
    pub learned_at: Option<StoryPlacement>,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub reader_visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecordKnowledgeOutput {
    pub fact: KnowledgeFactSummary,
    pub knows_edge_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecordNoteInput {
    pub project_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecordNoteOutput {
    pub activity_id: String,
    pub branch_id: String,
    pub kind: String,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateWriterPositionInput {
    pub project_id: String,
    pub branch_id: String,
    #[serde(default)]
    pub book_id: Option<String>,
    #[serde(default)]
    pub chapter_id: Option<String>,
    #[serde(default)]
    pub scene_id: Option<String>,
    pub intent: String,
    #[serde(default)]
    pub next_focus: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModelRouteSummary {
    pub route_name: String,
    pub adapter_kind: String,
    pub model_name: String,
    pub purpose: String,
    /// Content rating this route binding serves, when the rule was per-rating.
    /// `None` indicates the default rule for the route. Two entries with the
    /// same `route_name` but different `rating` may both appear in a listing —
    /// use `rating` to disambiguate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
    /// True when the resolved adapter spawns its own MCP-capable agent
    /// (currently the `grok-cli` provider, adapter_kind `"grok"`). Callers
    /// that build prompts should send a SHORT brief to such routes — the
    /// spawned agent pulls bible canon on demand via the spindle MCP server,
    /// so any pre-packed context in the prompt is wasted tokens. False for
    /// stateless adapters (`http`, `local`, raw `cli`) where the caller must
    /// pre-pack everything the model needs to see.
    #[serde(default)]
    pub caller_should_send_brief: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigureAgentsInput {
    pub config_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct EmptyInput {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct InitGrokSkillsInput {
    /// Optional target directory. If omitted, uses the current working directory.
    /// Ignored if `global` is true.
    pub target_dir: Option<String>,

    /// Install into the user's global `~/.grok/skills/` directory.
    /// This is the default behavior because Spindle projects are database-driven.
    #[serde(default = "default_true")]
    pub global: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InitGrokSkillsOutput {
    pub target_dir: String,
    pub files_written: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigureAgentsOutput {
    pub source_path: Option<String>,
    pub agents_loaded: usize,
    pub routing_rules_loaded: usize,
    pub health_checks_enabled: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentConfigStatus {
    Active,
    MissingApiKey,
    Unreachable,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct AgentHealthSummary {
    pub checked: bool,
    pub reachable: bool,
    pub status_code: Option<u16>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentSummary {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub max_context: Option<usize>,
    #[serde(default)]
    pub ratings: Vec<String>,
    pub quality_tier: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub notes: Option<String>,
    pub status: AgentConfigStatus,
    pub health: AgentHealthSummary,
    #[serde(default)]
    pub route_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListAgentsOutput {
    pub source_path: Option<String>,
    pub health_checks_enabled: bool,
    #[serde(default)]
    pub agents: Vec<AgentSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentRoutingRuleSummary {
    pub route_name: String,
    pub agent_id: String,
    pub fallback_agent_id: Option<String>,
    pub purpose: Option<String>,
    /// Optional configured system prompt. When omitted, the route purpose is
    /// used as the system prompt by the model router.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    #[serde(default)]
    pub stop: Vec<String>,
    /// Optional content rating this rule serves (general/teen/mature/explicit).
    /// `None` indicates the default rule for the route — used when no
    /// rating-specific override matches the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
    /// Resolved adapter the rule's agent uses (`http`, `grok`, `cli`,
    /// `local`). Surfaced here so callers can pick a prompt-building strategy
    /// without joining against `list_agents`.
    #[serde(default)]
    pub adapter_kind: String,
    /// True when the resolved adapter spawns its own MCP-capable agent
    /// (currently the `grok-cli` provider). Callers that build prompts should
    /// send a SHORT brief to such routes — the spawned agent pulls bible
    /// canon on demand via the spindle MCP server, so any pre-packed context
    /// in the prompt is wasted tokens. False for stateless adapters where
    /// the caller must pre-pack everything the model needs to see.
    #[serde(default)]
    pub caller_should_send_brief: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentRoutingConfigOutput {
    pub source_path: Option<String>,
    pub health_checks_enabled: bool,
    #[serde(default)]
    pub rules: Vec<AgentRoutingRuleSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TestAgentInput {
    pub agent_id: String,
    pub test_prompt: Option<String>,
    /// Optional logical route to force for this call. When omitted, Spindle
    /// preserves the legacy behavior of choosing the first configured route
    /// for the agent, preferring a matching rating when provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Optional content rating used when this test call is actually driving a
    /// draft route. Harness callers use this to select per-rating draft
    /// routes, especially explicit overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TestAgentOutput {
    pub agent_id: String,
    pub route_name: String,
    pub adapter_kind: String,
    pub model_name: String,
    pub health_checked: bool,
    pub output: String,
    /// True when the model hit its token limit and the output is incomplete.
    #[serde(default)]
    pub truncated: bool,
    /// Server-side receipt id for draft-route output. Present only when the
    /// call produced non-empty draft text through Spindle's model router.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_output_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContinueGenerationInput {
    /// The route name used for the original generation (e.g. "draft").
    pub route: String,
    /// The original prompt that produced the truncated output.
    pub original_prompt: String,
    /// The truncated output from the previous call.
    pub prior_output: String,
    /// Optional content rating to direct rating-aware routing on this
    /// continuation. When set, the router prefers a per-rating routing rule
    /// for the given `route`; falls back to the default rule when no match.
    /// Use this to keep continuations on the same explicit-capable agent
    /// that produced the original draft.
    #[serde(default)]
    pub rating: Option<String>,
    /// Project id this generation belongs to. Required for adapters that
    /// run the drafting model in their own MCP session (currently `grok-cli`)
    /// so the spawned agent can call `set_active_project` before pulling
    /// canon — its session does not inherit the caller's active project.
    /// When omitted, the MCP layer falls back to the session's
    /// `active_project_id` if one has been set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Book id this generation belongs to. Optional context passed through to
    /// the drafting agent for richer bootstrap signaling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_id: Option<String>,
    /// Chapter id this generation belongs to. Optional context passed through
    /// to the drafting agent for richer bootstrap signaling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    /// Scene id this generation belongs to. Optional context passed through
    /// to the drafting agent for richer bootstrap signaling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContinueGenerationOutput {
    /// The continuation fragment (does not repeat prior text).
    pub output: String,
    /// True if this continuation was also truncated and needs another call.
    #[serde(default)]
    pub truncated: bool,
    /// Server-side receipt id for the completed generated output. Explicit
    /// sexual drafts saved through `save_scene_draft` must reference a recent
    /// receipt generated through `continue_generation` or `revise_generation`
    /// with `route: "draft"` and `rating: "explicit"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    /// Agent id that produced the generation receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_agent_id: Option<String>,
    /// SHA-256 of the normalized completed output tracked by the generation
    /// receipt. For continuations, this hashes `prior_output + output`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_output_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviseGenerationInput {
    /// Server-side receipt id returned by `continue_generation` or a prior
    /// `revise_generation` call. Any rating the source receipt was produced
    /// at is revisable — `general`, `teen`, `mature`, and `explicit` all
    /// flow through this path. Use it for surgical edits without a full
    /// `continue_generation` re-roll.
    pub generation_id: String,
    /// Instructions for revising the generated prose. The revision is sent
    /// back through the same draft route the source receipt used, preserving
    /// its rating (so explicit receipts stay on the explicit-capable agent,
    /// mature receipts stay on the mature route, and so on).
    pub edit_instructions: String,
    /// Optional continuity/context notes to include in the revision prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviseGenerationOutput {
    /// The revised full prose text returned by the draft route that produced
    /// the source receipt.
    pub output: String,
    /// True when the model hit its token limit and the output is incomplete.
    #[serde(default)]
    pub truncated: bool,
    /// Source receipt id that was revised.
    pub source_generation_id: String,
    /// Server-side receipt id for the revised output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    /// Agent id that produced the revised generation receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_agent_id: Option<String>,
    /// SHA-256 of the normalized revised output tracked by the receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_output_sha256: Option<String>,
}

// ── Research query and routed research workflow ─────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
pub enum SourcePolicy {
    #[default]
    #[serde(rename = "require_sources")]
    RequireSources,
    #[serde(rename = "allow_unsourced_leads")]
    AllowUnsourcedLeads,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ResearchSourceOutputItem {
    pub title: String,
    pub source_type: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub published_date: Option<String>,
    pub reliability: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ResearchNoteOutputItem {
    #[serde(default)]
    pub source_index: Option<usize>,
    pub note: String,
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub locator: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ResearchClaimOutputItem {
    #[serde(default)]
    pub note_index: Option<usize>,
    pub claim: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub time_period: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    pub confidence: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchQueryInput {
    /// The project to provide context for the research query.
    pub project_id: String,
    /// The factual question to research (e.g. "What does decompression sickness feel like?").
    pub query: String,
    /// Optional hint to narrow the research context (e.g. "chapter 5 dive scene").
    #[serde(default)]
    pub context_hint: Option<String>,
    #[serde(default)]
    pub branch_id: Option<String>,
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub scene_id: Option<String>,
    #[serde(default = "default_true")]
    pub store: bool,
    #[serde(default)]
    pub source_policy: Option<SourcePolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchQueryOutput {
    pub summary: String,
    pub sources: Vec<ResearchSourceOutputItem>,
    pub notes: Vec<ResearchNoteOutputItem>,
    pub claims: Vec<ResearchClaimOutputItem>,
    pub tags: Vec<String>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub uncertainty_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchIngestReportInput {
    pub project_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub title: String,
    pub report: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub source_policy: Option<SourcePolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchIngestReportOutput {
    pub summary: String,
    pub sources: Vec<ResearchSourceOutputItem>,
    pub notes: Vec<ResearchNoteOutputItem>,
    pub claims: Vec<ResearchClaimOutputItem>,
    pub tags: Vec<String>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub uncertainty_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchPlanForSceneInput {
    pub project_id: String,
    pub scene_id: String,
    #[serde(default)]
    pub planned_scene_summary: Option<String>,
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchPlanForSceneOutput {
    pub suggested_queries: Vec<String>,
    pub missing_tags: Vec<String>,
    pub await_research: bool,
}

// ── Research Library Models & DTOs ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ResearchSource {
    pub id: String,
    pub project_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub title: String,
    pub source_type: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub published_date: Option<String>,
    pub accessed_at: i64,
    pub reliability: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ResearchNote {
    pub id: String,
    pub project_id: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub note: String,
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub locator: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ResearchClaim {
    pub id: String,
    pub project_id: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub note_id: Option<String>,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub claim: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub time_period: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    pub confidence: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ResearchUsage {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub run_id: String,
    #[serde(default)]
    pub step_checkpoint_id: Option<String>,
    pub scene_id: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub note_ids: Vec<String>,
    #[serde(default)]
    pub claim_ids: Vec<String>,
    pub query_pack_input: String,
    pub context_hash: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchAddSourceInput {
    pub project_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub title: String,
    pub source_type: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub published_date: Option<String>,
    #[serde(default)]
    pub accessed_at: Option<i64>,
    pub reliability: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchAddSourceOutput {
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchAddNoteInput {
    pub project_id: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub note: String,
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub locator: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchAddNoteOutput {
    pub note_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchAddClaimInput {
    pub project_id: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub note_id: Option<String>,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub claim: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub time_period: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    pub confidence: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchAddClaimOutput {
    pub claim_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchSearchInput {
    pub project_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub query: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub time_period: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ResearchSearchResultItem {
    pub item_type: String, // "source", "note", or "claim"
    pub id: String,
    pub title_or_summary: String,
    pub preview: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub source_title: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub source_author: Option<String>,
    #[serde(default)]
    pub locator: Option<String>,
    #[serde(default)]
    pub confidence_or_reliability: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchSearchOutput {
    pub results: Vec<ResearchSearchResultItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchPackForSceneInput {
    pub project_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
    #[serde(default)]
    pub scene_summary: Option<String>,
    #[serde(default)]
    pub scene_location: Option<String>,
    #[serde(default)]
    pub character_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub explicit_query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchPackForSceneOutput {
    pub sources: Vec<ResearchSource>,
    pub notes: Vec<ResearchNote>,
    pub claims: Vec<ResearchClaim>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportEpubInput {
    pub project_id: String,
    /// Optional author name for the title page. If omitted, no author line is
    /// rendered.
    #[serde(default)]
    pub author: Option<String>,
    /// Restrict export to a single book number. If omitted, all books in the
    /// project are included.
    #[serde(default)]
    pub book_number: Option<i32>,
    /// Optional inclusive start chapter for partial export. Requires
    /// `book_number`.
    #[serde(default, alias = "chapter_start", alias = "from_chapter_number")]
    pub start_chapter_number: Option<i32>,
    /// Optional inclusive end chapter for partial export. Requires
    /// `book_number`.
    #[serde(default, alias = "chapter_end", alias = "to_chapter_number")]
    pub end_chapter_number: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreflightBookExportInput {
    pub project_id: String,
    /// Restrict validation to a single book number. If omitted, all books in
    /// the project are checked.
    #[serde(default)]
    pub book_number: Option<i32>,
    /// Optional inclusive start chapter for partial export validation.
    /// Requires `book_number`.
    #[serde(default, alias = "chapter_start", alias = "from_chapter_number")]
    pub start_chapter_number: Option<i32>,
    /// Optional inclusive end chapter for partial export validation. Requires
    /// `book_number`.
    #[serde(default, alias = "chapter_end", alias = "to_chapter_number")]
    pub end_chapter_number: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum ExportIssueSeverity {
    Blocking,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportIssue {
    pub severity: ExportIssueSeverity,
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub book_number: Option<i32>,
    #[serde(default)]
    pub chapter_number: Option<i32>,
    #[serde(default)]
    pub scene_order: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreflightBookExportOutput {
    pub project_id: String,
    #[serde(default)]
    pub book_number: Option<i32>,
    #[serde(default)]
    pub start_chapter_number: Option<i32>,
    #[serde(default)]
    pub end_chapter_number: Option<i32>,
    #[serde(default)]
    pub issues: Vec<ExportIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportBibleInput {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportBibleOutput {
    /// Absolute path to the written JSON export file.
    pub file_path: String,
    /// Suggested filename.
    pub filename: String,
    pub exported_tables: usize,
    pub exported_records: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportEpubOutput {
    /// Absolute path to the written EPUB file.
    pub file_path: String,
    /// Suggested filename.
    pub filename: String,
    pub total_chapters: usize,
    pub total_scenes: usize,
    /// Non-blocking issues found during export preflight.
    #[serde(default)]
    pub preflight_warnings: Vec<ExportIssue>,
    /// Scenes whose Spindle content differs from their linked local files.
    #[serde(default)]
    pub divergence_warnings: Vec<DivergenceWarning>,
}

/// A warning that a Spindle scene has diverged from its local source file.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DivergenceWarning {
    pub scene_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub source_path: String,
    pub kind: DivergenceKind,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum DivergenceKind {
    /// Local file content differs from Spindle scene content.
    ContentMismatch,
    /// Local file no longer exists.
    SourceMissing,
    /// Divergence could not be determined from current source tracking data.
    Unknown,
}

// ── Canonical fact registry ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterCanonicalFactInput {
    pub project_id: String,
    pub scene_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    /// The kind of fact: "pull_result", "stat_change", "item_acquired",
    /// "ability_gained", "death", "relationship_change", etc.
    #[serde(default)]
    pub fact_type: Option<String>,
    /// Unique key identifying the fact (e.g. "Kael:Muscle Memory").
    #[serde(default)]
    pub key: Option<String>,
    /// The canonical value (e.g. "Muscle Memory (Mild)").
    #[serde(default)]
    pub value: Option<String>,
    /// Optional context about where/how this fact was established.
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub subject_table: Option<String>,
    #[serde(default)]
    pub subject_id: Option<String>,
    #[serde(default)]
    pub predicate: Option<String>,
    #[serde(default)]
    pub value_kind: Option<String>,
    #[serde(default)]
    pub value_text: Option<String>,
    #[serde(default)]
    pub value_number: Option<f64>,
    #[serde(default)]
    pub value_unit: Option<String>,
    #[serde(default)]
    pub value_json: Option<serde_json::Value>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub scope: Option<CanonicalFactScope>,
    #[serde(default)]
    pub valid_from: Option<StoryPlacement>,
    #[serde(default)]
    pub valid_until: Option<StoryPlacement>,
    #[serde(default)]
    pub legacy_untyped: Option<bool>,
    /// If this fact supersedes a previous one, supply the old fact's id.
    #[serde(default)]
    pub supersedes_fact_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterCanonicalFactOutput {
    pub canonical_fact_id: String,
    pub superseded_fact_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractCanonicalFactsFromSceneInput {
    pub scene_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractCanonicalFactProposal {
    pub subject_table: String,
    #[serde(default)]
    pub subject_id: Option<String>,
    pub predicate: String,
    pub value_kind: String,
    #[serde(default)]
    pub value_text: Option<String>,
    #[serde(default)]
    pub value_number: Option<f64>,
    #[serde(default)]
    pub value_unit: Option<String>,
    #[serde(default)]
    pub value_json: Option<serde_json::Value>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub scope: Option<CanonicalFactScope>,
    #[serde(default)]
    pub valid_from: Option<StoryPlacement>,
    #[serde(default)]
    pub valid_until: Option<StoryPlacement>,
    #[serde(default)]
    pub source_excerpt: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractCanonicalFactsFromSceneOutput {
    pub scene_id: String,
    pub proposals: Vec<ExtractCanonicalFactProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalFactUpgradeSpec {
    pub subject_table: String,
    #[serde(default)]
    pub subject_id: Option<String>,
    pub predicate: String,
    pub value_kind: String,
    #[serde(default)]
    pub value_text: Option<String>,
    #[serde(default)]
    pub value_number: Option<f64>,
    #[serde(default)]
    pub value_unit: Option<String>,
    #[serde(default)]
    pub value_json: Option<serde_json::Value>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub scope: Option<CanonicalFactScope>,
    #[serde(default)]
    pub valid_from: Option<StoryPlacement>,
    #[serde(default)]
    pub valid_until: Option<StoryPlacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MigrateCanonicalFactInput {
    pub fact_id: String,
    pub upgrade_spec: CanonicalFactUpgradeSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MigrateCanonicalFactOutput {
    pub canonical_fact_id: String,
    pub superseded_fact_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalFactSummary {
    pub canonical_fact_id: String,
    pub fact_type: String,
    pub key: String,
    pub value: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub context: Option<String>,
    pub superseded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalFactScope {
    Invariant,
    Evolving,
    Conditional,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CanonicalValue {
    Number {
        value: f64,
        unit: Option<String>,
    },
    Date(StoryPlacement),
    Text(String),
    Enum {
        choice: String,
        choices: Vec<String>,
    },
    Range {
        min: f64,
        max: f64,
        unit: Option<String>,
    },
    Boolean(bool),
    List {
        required: Vec<String>,
        forbidden: Vec<String>,
    },
}

// =============================================================================
// Authoring Supervisor DTOs
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStartRunInput {
    pub project_id: String,
    pub book_number: i32,
    pub start_chapter: i32,
    #[serde(default)]
    pub end_chapter: Option<i32>,
    #[serde(default)]
    pub chapter_count: Option<i32>,
    pub checkpoint_interval: usize,
    #[serde(default)]
    pub editorial_directives: Option<Vec<String>>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStartRunOutput {
    pub run_id: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStatusInput {
    pub project_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStatusOutput {
    pub run_id: String,
    pub project_id: String,
    pub status: String,
    pub next_action: String,
    pub blocked_reason: Option<String>,
    pub checkpoint_state: Option<String>,
    pub start_chapter: i32,
    pub end_chapter: i32,
    pub completed_chapter_count: usize,
    pub total_chapter_count: usize,
    pub chapters: Vec<AuthoringStatusChapter>,
    pub checkpoint_reports: Vec<AuthoringStatusCheckpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStatusChapter {
    pub chapter_number: i32,
    pub status: String,
    pub summary_saved: bool,
    pub summary_artifact_path: Option<String>,
    pub scenes: Vec<AuthoringStatusScene>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStatusScene {
    pub scene_order: i32,
    pub phase: String,
    pub scene_id: Option<String>,
    pub scene_artifact_path: Option<String>,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStatusCheckpoint {
    pub start_chapter: i32,
    pub end_chapter: i32,
    pub save_point_id: String,
    pub status: String,
    pub report_artifact_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringExecuteNextInput {
    pub project_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    /// Optional execution mode for drafting steps.
    ///
    /// Default is "hybrid": the host assistant drafts non-explicit prose in
    /// the active chat using Spindle context/tools, while explicit scenes may
    /// still be routed through the configured explicit draft backend.
    ///
    /// Set to "agent" only for fully automated test/offload runs.
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringExecuteNextOutput {
    pub run_id: String,
    pub next_action: String,
    pub executed_action: String,
    pub message: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringSaveSceneDraftInput {
    pub project_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub book_number: i32,
    #[serde(default)]
    pub chapter_number: i32,
    #[serde(default)]
    pub chapter_id: Option<String>,
    pub scene_order: i32,
    #[serde(alias = "content", alias = "text")]
    pub full_text: String,
    pub summary: String,
    pub content_rating: ContentRating,
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub research_source_ids: Vec<String>,
    #[serde(default)]
    pub research_note_ids: Vec<String>,
    #[serde(default)]
    pub research_claim_ids: Vec<String>,
    #[serde(default)]
    pub research_query_pack_input: Option<String>,
    #[serde(default)]
    pub research_context_hash: Option<String>,
    #[serde(default)]
    pub character_states: Vec<CharacterStatePatchEntry>,
    #[serde(default)]
    pub canonical_facts: Vec<CanonicalFactEntry>,
    #[serde(default)]
    pub relationship_updates: Vec<RelationshipUpdateEntry>,
    #[serde(default)]
    pub beats: Vec<AnnotatedBeatInput>,
    #[serde(default)]
    pub continuity_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringSaveSceneDraftOutput {
    pub run_id: String,
    pub scene_id: String,
    pub scene_artifact_path: String,
    pub status: String,
    pub structured_update_count: usize,
    pub save_output: SaveSceneDraftOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringReviewCheckpointInput {
    pub project_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    pub start_chapter: i32,
    pub end_chapter: i32,
    pub directives: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringRecordCheckpointAuditInput {
    pub project_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    pub start_chapter: i32,
    pub end_chapter: i32,
    /// The structured output returned by check_consistency with deep_check=true
    /// for this checkpoint's chapter range.
    pub deep_consistency: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringRecordCheckpointAuditOutput {
    pub run_id: String,
    pub message: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringReviewCheckpointOutput {
    pub run_id: String,
    pub message: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringResolveBlockInput {
    pub project_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub target_phase: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringResolveBlockOutput {
    pub run_id: String,
    pub message: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringCancelRunInput {
    pub project_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringCancelRunOutput {
    pub run_id: String,
    pub message: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringPrepareRunInput {
    pub project_id: String,
    pub book_number: i32,
    pub start_chapter: i32,
    #[serde(default)]
    pub end_chapter: Option<i32>,
    #[serde(default)]
    pub chapter_count: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringPrepareRunOutput {
    pub project_id: String,
    pub book_number: i32,
    pub start_chapter: i32,
    pub end_chapter: i32,
    pub ready_to_draft: bool,
    pub missing_requirements: Vec<String>,
    pub details: Vec<AuthoringPrepareChapterDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringPrepareChapterDetails {
    pub chapter_number: i32,
    pub ready: bool,
    pub missing_items: Vec<String>,
}

pub fn normalize_name(input: &str) -> String {
    input.trim().to_lowercase()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchUsageForSceneInput {
    pub project_id: String,
    pub scene_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResearchUsageForSceneOutput {
    pub usages: Vec<ResearchUsage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_manuscript_contract_round_trips() {
        let payload = ImportManuscriptInput {
            source_paths: vec!["/tmp/manuscript.md".to_string()],
            target_project_id: None,
            target_branch_id: Some("bible_branch:main".to_string()),
            create_project_name: Some("Imported Novel".to_string()),
            source_format_hint: Some(ImportSourceFormat::Md),
            duplicate_strategy: Some(ImportDuplicateStrategy::Reject),
        };

        let json = serde_json::to_string(&payload).expect("serialize import manuscript input");
        let decoded: ImportManuscriptInput =
            serde_json::from_str(&json).expect("deserialize import manuscript input");
        assert_eq!(decoded.source_paths, payload.source_paths);
        assert!(matches!(
            decoded.source_format_hint,
            Some(ImportSourceFormat::Md)
        ));
    }

    #[test]
    fn import_status_contract_round_trips() {
        let payload = ImportStatusOutput {
            session: ImportSessionSummary {
                session_id: "import_session:alpha".to_string(),
                project_id: Some("project:demo".to_string()),
                target_branch_id: Some("bible_branch:main".to_string()),
                status: ImportSessionStatus::ReviewNeeded,
                active_pass: ImportPassName::NarrativeAnalysis,
                source_format: Some(ImportSourceFormat::Docx),
                hydrate_mode: ImportHydrationMode::NewProject,
                progress: ImportSessionProgress {
                    total_documents: 1,
                    processed_documents: 1,
                    total_segments: 12,
                    processed_segments: 12,
                    total_review_items: 3,
                    open_review_items: 1,
                },
                started_at: Some("2026-04-04T12:00:00Z".to_string()),
                updated_at: Some("2026-04-04T12:05:00Z".to_string()),
            },
            structure: Some(ImportStructuralAnalysisSummary {
                source_documents: vec![ImportSourceDocumentSummary {
                    document_id: "import_source_document:one".to_string(),
                    display_name: "chapter-01.docx".to_string(),
                    source_path: "/tmp/chapter-01.docx".to_string(),
                    copied_path: "/data/imports/chapter-01.docx".to_string(),
                    source_format: ImportSourceFormat::Docx,
                    original_sha256: "orig".to_string(),
                    normalized_sha256: "norm".to_string(),
                    word_count: 1200,
                    chapter_hint: Some("Chapter One".to_string()),
                    source_order: 0,
                }],
                chapters: vec![ImportChapterSlice {
                    segment_id: "import_segment:ch1".to_string(),
                    book_number: 1,
                    chapter_number: 1,
                    title: Some("Arrival".to_string()),
                    start_offset: 0,
                    end_offset: 1200,
                    word_count: 1200,
                    confidence: 0.92,
                    confidence_level: ImportConfidenceLevel::High,
                    scenes: vec![ImportSceneSlice {
                        segment_id: "import_segment:sc1".to_string(),
                        chapter_segment_id: Some("import_segment:ch1".to_string()),
                        scene_index: 1,
                        label: Some("Scene 1".to_string()),
                        start_offset: 0,
                        end_offset: 400,
                        word_count: 400,
                        character_count: 2300,
                        pov_guess: Some(ImportPovGuess {
                            character_name: Some("Mara".to_string()),
                            cluster_id: Some("import_entity_cluster:mara".to_string()),
                            confidence: 0.88,
                            confidence_level: ImportConfidenceLevel::High,
                            source: ImportPovGuessSource::Heuristic,
                            rationale: Some(
                                "First-person pronouns cluster around Mara".to_string(),
                            ),
                        }),
                        confidence: 0.9,
                        confidence_level: ImportConfidenceLevel::High,
                    }],
                }],
                review_items_created: 1,
            }),
            entity_extraction: None,
            entity_consolidation: None,
            characters: Vec::new(),
            world: None,
            narrative: None,
            final_state: None,
            review_items: vec![ImportReviewItemSummary {
                review_item_id: "import_review_item:one".to_string(),
                session_id: "import_session:alpha".to_string(),
                pass_name: ImportPassName::StructuralAnalysis,
                kind: ImportReviewItemKind::Structure,
                severity: ImportReviewSeverity::RequiresReview,
                status: ImportReviewStatus::Open,
                title: "Ambiguous POV".to_string(),
                description: "Two viewpoint candidates were equally likely.".to_string(),
                related_segment_ids: vec!["import_segment:sc1".to_string()],
                related_entity_ids: vec![],
                confidence: Some(0.51),
                proposed_correction: Some(ImportCorrectionPayload::Structural {
                    chapter_number: Some(1),
                    scene_order: Some(1),
                    pov_character_name: Some("Mara".to_string()),
                    segment_ids: vec!["import_segment:sc1".to_string()],
                }),
                resolver_notes: None,
            }],
            hydration_report: None,
        };

        let json = serde_json::to_string(&payload).expect("serialize import status output");
        let decoded: ImportStatusOutput =
            serde_json::from_str(&json).expect("deserialize import status output");
        assert_eq!(decoded.session.session_id, payload.session.session_id);
        assert_eq!(decoded.review_items.len(), 1);
        assert!(decoded.structure.is_some());
    }

    #[test]
    fn record_knowledge_contract_round_trips() {
        let payload = RecordKnowledgeOutput {
            fact: KnowledgeFactSummary {
                knowledge_fact_id: "knowledge_fact:one".to_string(),
                branch_id: "bible_branch:main".to_string(),
                character_id: "character:mara".to_string(),
                fact: "Mara knows the archive key is hidden in the bell tower.".to_string(),
                source_summary: "Imported from chapter 12 confrontation.".to_string(),
                learned_at: Some(StoryPlacement {
                    book_number: 1,
                    chapter_number: 12,
                    scene_order: Some(2),
                    note: Some("bell tower reveal".to_string()),
                }),
                confidence: Some(0.93),
                tags: vec!["imported".to_string(), "plot-critical".to_string()],
                reader_visible: true,
            },
            knows_edge_id: Some("knows:edge1".to_string()),
        };

        let json = serde_json::to_string(&payload).expect("serialize record knowledge output");
        let decoded: RecordKnowledgeOutput =
            serde_json::from_str(&json).expect("deserialize record knowledge output");
        assert_eq!(decoded.fact.fact, payload.fact.fact);
        assert_eq!(decoded.fact.tags, payload.fact.tags);
        assert_eq!(decoded.knows_edge_id, payload.knows_edge_id);
    }

    #[test]
    fn record_note_contract_round_trips() {
        let input = RecordNoteInput {
            project_id: "project:demo".to_string(),
            branch_id: Some("bible_branch:main".to_string()),
            note: "Remember to keep Tarin's injury visible in scene 3.".to_string(),
        };
        let input_json = serde_json::to_string(&input).expect("serialize record note input");
        let decoded_input: RecordNoteInput =
            serde_json::from_str(&input_json).expect("deserialize record note input");
        assert_eq!(decoded_input.project_id, input.project_id);
        assert_eq!(decoded_input.branch_id, input.branch_id);
        assert_eq!(decoded_input.note, input.note);

        let output = RecordNoteOutput {
            activity_id: "session_activity:abc123".to_string(),
            branch_id: "bible_branch:main".to_string(),
            kind: "note".to_string(),
            summary: input.note,
            created_at: "2026-04-10T01:00:00Z".to_string(),
        };
        let output_json = serde_json::to_string(&output).expect("serialize record note output");
        let decoded_output: RecordNoteOutput =
            serde_json::from_str(&output_json).expect("deserialize record note output");
        assert_eq!(decoded_output.activity_id, output.activity_id);
        assert_eq!(decoded_output.branch_id, output.branch_id);
        assert_eq!(decoded_output.kind, "note");
        assert_eq!(decoded_output.summary, output.summary);
    }

    #[test]
    fn research_query_contract_round_trips() {
        let input = ResearchQueryInput {
            project_id: "project:demo".to_string(),
            query: "What does decompression sickness feel like?".to_string(),
            context_hint: Some("chapter 5 dive scene".to_string()),
            branch_id: None,
            rating: None,
            tags: None,
            scene_id: None,
            store: true,
            source_policy: None,
        };
        let json = serde_json::to_string(&input).expect("serialize research query input");
        let decoded: ResearchQueryInput =
            serde_json::from_str(&json).expect("deserialize research query input");
        assert_eq!(decoded.project_id, input.project_id);
        assert_eq!(decoded.query, input.query);
        assert_eq!(decoded.context_hint, input.context_hint);

        let output = ResearchQueryOutput {
            summary: "Decompression sickness causes joint pain and fatigue.".to_string(),
            sources: vec![],
            notes: vec![],
            claims: vec![],
            tags: vec!["diving".to_string()],
            warnings: vec![],
            uncertainty_level: Some("low".to_string()),
        };
        let json = serde_json::to_string(&output).expect("serialize research query output");
        let decoded: ResearchQueryOutput =
            serde_json::from_str(&json).expect("deserialize research query output");
        assert_eq!(decoded.summary, output.summary);
        assert_eq!(decoded.tags, output.tags);
        assert_eq!(decoded.uncertainty_level, output.uncertainty_level);
    }

    #[test]
    fn character_voice_profile_data_legacy_shape_round_trips() {
        let payload = serde_json::json!({
            "vocabulary": ["oath"],
            "sentence_structure": ["direct"],
            "tics": [],
            "forbidden_words": [],
            "example_lines": ["I know the cost."]
        });

        let decoded: CharacterVoiceProfileData =
            serde_json::from_value(payload).expect("deserialize legacy voice profile");
        assert_eq!(decoded.tone, None);
        assert_eq!(decoded.established_in_scene_id, None);
        assert_eq!(decoded.updated_at, None);
        assert_eq!(decoded.vocabulary, vec!["oath".to_string()]);
    }

    #[test]
    fn character_voice_profile_data_enriched_shape_round_trips() {
        let payload = CharacterVoiceProfileData {
            tone: Some("grim".to_string()),
            vocabulary: vec!["oath".to_string()],
            sentence_structure: vec!["direct".to_string()],
            tics: vec!["touches the pommel".to_string()],
            forbidden_words: vec!["maybe".to_string()],
            example_lines: vec!["I know the cost.".to_string()],
            established_in_scene_id: Some("scene:gate-breach".to_string()),
            updated_at: Some("2026-04-09T04:00:00Z".to_string()),
        };

        let json = serde_json::to_string(&payload).expect("serialize enriched voice profile");
        let decoded: CharacterVoiceProfileData =
            serde_json::from_str(&json).expect("deserialize enriched voice profile");
        assert_eq!(decoded.tone, payload.tone);
        assert_eq!(
            decoded.established_in_scene_id,
            payload.established_in_scene_id
        );
        assert_eq!(decoded.updated_at, payload.updated_at);
        assert_eq!(decoded.example_lines, payload.example_lines);
    }

    #[test]
    fn commit_scene_changes_structured_canonical_fact_entries_deserialize() {
        let payload = serde_json::json!({
            "project_id": "project:test",
            "scene_id": "scene:test",
            "character_states": [
                {
                    "character_id": "character:cole",
                    "state": "Training at Iron Circle twice weekly."
                }
            ],
            "canonical_facts": [
                {
                    "fact_type": "ability_gained",
                    "key": "pull-6-snap-reflex-c-rank-uncommon",
                    "value": "Pull #6: Snap Reflex (C-Rank, Uncommon) — 18% reaction time improvement."
                }
            ],
            "relationship_updates": [
                {
                    "character_id_1": "character:cole",
                    "character_id_2": "character:sam",
                    "summary": "Mentor-student. Cole trusts Sam's process."
                }
            ]
        });

        let decoded: CommitSceneChangesInput =
            serde_json::from_value(payload).expect("deserialize scene commit payload");

        assert_eq!(
            decoded.character_states[0].changes.notes,
            Some(vec!["Training at Iron Circle twice weekly.".to_string()])
        );
        assert_eq!(
            decoded.character_states[0].changes.source_summary,
            Some("Training at Iron Circle twice weekly.".to_string())
        );
        assert_eq!(
            decoded.canonical_facts[0].fact_type,
            Some("ability_gained".to_string())
        );
        assert_eq!(
            decoded.canonical_facts[0].key,
            Some("pull-6-snap-reflex-c-rank-uncommon".to_string())
        );
        assert_eq!(decoded.relationship_updates[0].trust_delta, 0);
        assert_eq!(decoded.relationship_updates[0].tension_delta, 0);
        assert_eq!(
            decoded.relationship_updates[0].reason,
            "Mentor-student. Cole trusts Sam's process."
        );
    }
}
