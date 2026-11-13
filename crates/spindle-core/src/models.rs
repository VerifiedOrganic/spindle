pub use crate::serial::*;
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

/// Deserialize an explicit JSON `null` (or a missing field, combined with
/// `#[serde(default)]`) into the type's default. Miners and permissive clients
/// send `null` for "unknown" struct fields; rejecting that with
/// "invalid type: null, expected struct …" makes optional-by-intent fields
/// hard-required in practice.
fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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

/// One sampled point on a book's expected-intensity curve. `position` is a
/// `0.0..=1.0` fraction of the book; `intensity` is the `0.0..=1.0` expected
/// intensity there. Stored on `pacing_curve.intensity_points`; the realized-
/// intensity trend directive interpolates against a curve's points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IntensityPoint {
    pub position: f64,
    pub intensity: f64,
}

/// In-world placement of a scene or event on the project's story clock. Every
/// field is optional: a project that never declares story-time leaves them unset
/// and behaves exactly as before. Distinct from [`StoryPlacement`], which is the
/// structural (manuscript) position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct StoryClock {
    /// In-world day index from the project epoch (monotonic, spans books).
    /// Negative values are valid: the epoch is the story's opening, and
    /// births, backstory, and timeline events routinely precede it.
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

/// A human-facing date in a project calendar. Year 1 starts at day index zero;
/// `month` is one-based in [`CalendarDef::months`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CalendarDate {
    pub year: i64,
    pub month: i32,
    pub day: i32,
}

/// How a calendar's years gain days. Existing calendars deserialize as `None`
/// (fixed-length years); a project opts into leap years by setting a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LeapYearRule {
    /// Fixed-length years: current behavior for every pre-existing calendar.
    #[default]
    None,
    /// Proleptic Gregorian: every 4 years, except every 100, except every 400.
    /// The calendar must declare a `February` month with 28 common-year days.
    Gregorian,
    /// Add days to one named month every N years. `anchor_year` controls the
    /// phase; year 0 is the default anchor, so year 4 is the first leap year in
    /// a four-year cycle.
    Custom {
        month: String,
        every_years: i32,
        #[serde(default = "default_leap_extra_days")]
        extra_days: i32,
        #[serde(default)]
        anchor_year: i64,
    },
}

fn default_leap_extra_days() -> i32 {
    1
}

impl LeapYearRule {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// Per-project calendar definition mapping the abstract day index to/from a
/// human-facing date. Supports non-24h days, leap years, and fully invented
/// month/week schemes, so an invented fantasy calendar is handled identically to
/// Gregorian.
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
    /// Days in a common (non-leap) year. A leap rule adds its extra days on
    /// top of this; legacy calendars with `leap_rule = None` keep this exact
    /// year length.
    pub days_per_year: i32,
    #[serde(default)]
    pub epoch_label: Option<String>,
    #[serde(default, skip_serializing_if = "LeapYearRule::is_none")]
    pub leap_rule: LeapYearRule,
}

impl CalendarDef {
    /// The standard twelve-month Gregorian calendar with leap years enabled.
    /// Year 1 starts at day index zero. Use this as the starting point for a
    /// Gregorian-style project, then replace any month names/lengths the
    /// project actually needs.
    pub fn gregorian() -> Self {
        let months = [
            ("January", 31),
            ("February", 28),
            ("March", 31),
            ("April", 30),
            ("May", 31),
            ("June", 30),
            ("July", 31),
            ("August", 31),
            ("September", 30),
            ("October", 31),
            ("November", 30),
            ("December", 31),
        ]
        .into_iter()
        .map(|(name, days)| CalendarMonth {
            name: name.to_string(),
            days,
        })
        .collect();
        Self {
            days_per_week: 7,
            hours_per_day: 24,
            week_day_names: Vec::new(),
            months,
            days_per_year: 365,
            epoch_label: None,
            leap_rule: LeapYearRule::Gregorian,
        }
    }

    /// Minutes in one in-world day. Folds `(day_index, time_of_day)` into a single
    /// total-order index.
    pub fn minutes_per_day(&self) -> i64 {
        self.hours_per_day.max(0) as i64 * 60
    }

    /// Whether `year` is a leap year under this calendar's rule.
    pub fn is_leap_year(&self, year: i64) -> bool {
        match &self.leap_rule {
            LeapYearRule::None => false,
            LeapYearRule::Gregorian => {
                year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
            }
            LeapYearRule::Custom {
                every_years,
                anchor_year,
                ..
            } => {
                year >= 1
                    && *every_years > 0
                    && (year - *anchor_year).rem_euclid(*every_years as i64) == 0
            }
        }
    }

    /// Extra days added by the leap rule (0 for a fixed calendar).
    pub fn leap_extra_days(&self) -> i32 {
        match &self.leap_rule {
            LeapYearRule::None => 0,
            LeapYearRule::Gregorian => 1,
            LeapYearRule::Custom { extra_days, .. } => *extra_days,
        }
    }

    /// Days in `year`, including the leap-rule extra when applicable.
    pub fn days_in_year(&self, year: i64) -> i32 {
        self.days_per_year
            + if self.is_leap_year(year) {
                self.leap_extra_days()
            } else {
                0
            }
    }

    fn leap_month_index(&self) -> Option<usize> {
        match &self.leap_rule {
            LeapYearRule::None => None,
            LeapYearRule::Gregorian => self
                .months
                .iter()
                .position(|month| month.name.eq_ignore_ascii_case("february")),
            LeapYearRule::Custom { month, .. } => self
                .months
                .iter()
                .position(|candidate| candidate.name.eq_ignore_ascii_case(month)),
        }
    }

    /// Days in a one-based calendar month for `year`.
    pub fn days_in_month(&self, year: i64, month: i32) -> Option<i32> {
        if month < 1 {
            return None;
        }
        let index = (month - 1) as usize;
        let base = self.months.get(index)?.days;
        let leap = self.is_leap_year(year) && self.leap_month_index() == Some(index);
        Some(base + if leap { self.leap_extra_days() } else { 0 })
    }

    /// Number of leap years before `year` (negative for years before year 1).
    fn leap_years_before(&self, year: i64) -> i64 {
        match &self.leap_rule {
            LeapYearRule::None => 0,
            LeapYearRule::Gregorian => {
                let n = year - 1;
                n.div_euclid(4) - n.div_euclid(100) + n.div_euclid(400)
            }
            LeapYearRule::Custom {
                every_years,
                anchor_year,
                ..
            } => {
                if *every_years <= 0 {
                    return 0;
                }
                let every = *every_years as i64;
                if year <= 1 {
                    0
                } else {
                    let n = year - 1;
                    let first = 1 + (anchor_year - 1).rem_euclid(every);
                    if first > n {
                        0
                    } else {
                        (n - first) / every + 1
                    }
                }
            }
        }
    }

    /// Days elapsed before January 1 of `year` (year 1 starts at day zero).
    fn days_before_year(&self, year: i64) -> i64 {
        (year - 1) * self.days_per_year as i64
            + self.leap_years_before(year) * self.leap_extra_days() as i64
    }

    /// Convert a calendar date to its absolute `day_index`.
    pub fn day_index_for_date(&self, date: &CalendarDate) -> Result<i64, String> {
        if self.months.is_empty() {
            return Err("calendar has no months; date conversion is unavailable".to_string());
        }
        if date.month < 1 || date.month as usize > self.months.len() {
            return Err(format!(
                "month {} is outside 1..={}",
                date.month,
                self.months.len()
            ));
        }
        let days_in_month = self
            .days_in_month(date.year, date.month)
            .ok_or_else(|| "invalid calendar month".to_string())?;
        if date.day < 1 || date.day > days_in_month {
            return Err(format!(
                "day {} is outside 1..={} for year {} month {}",
                date.day, days_in_month, date.year, date.month
            ));
        }
        let mut day_of_year = 0i64;
        for (index, _month) in self.months.iter().enumerate() {
            let month_number = index as i32 + 1;
            if month_number == date.month {
                break;
            }
            day_of_year +=
                self.days_in_month(date.year, month_number)
                    .ok_or_else(|| "invalid calendar month".to_string())? as i64;
        }
        Ok(self.days_before_year(date.year) + day_of_year + date.day as i64 - 1)
    }

    /// Alias for [`Self::day_index_for_date`].
    pub fn date_to_day_index(&self, date: &CalendarDate) -> Result<i64, String> {
        self.day_index_for_date(date)
    }

    /// Alias for [`Self::date_for_day_index`].
    pub fn day_index_to_date(&self, day_index: i64) -> Result<CalendarDate, String> {
        self.date_for_day_index(day_index)
    }

    /// Convert an absolute `day_index` to a calendar date.
    pub fn date_for_day_index(&self, day_index: i64) -> Result<CalendarDate, String> {
        if self.months.is_empty() {
            return Err("calendar has no months; date conversion is unavailable".to_string());
        }
        // The base-year estimate is never below the true year because leap days
        // only make years longer. Expand around it, then binary-search.
        let estimate = 1 + day_index.div_euclid(self.days_per_year as i64);
        let mut low = estimate.saturating_sub(4);
        let mut high = estimate.saturating_add(4);
        while self.days_before_year(low) > day_index {
            low = low.saturating_sub((low.abs() / 4).max(4));
        }
        while self.days_before_year(high) <= day_index {
            high = high.saturating_add((high.abs() / 4).max(4));
        }
        while low < high {
            let mid = low + (high - low + 1) / 2;
            if self.days_before_year(mid) <= day_index {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        let year = low;
        let mut day_of_year = day_index - self.days_before_year(year);
        for (index, _) in self.months.iter().enumerate() {
            let month = index as i32 + 1;
            let days = self
                .days_in_month(year, month)
                .ok_or_else(|| "invalid calendar month".to_string())?;
            if day_of_year < days as i64 {
                return Ok(CalendarDate {
                    year,
                    month,
                    day: day_of_year as i32 + 1,
                });
            }
            day_of_year -= days as i64;
        }
        Err("day_index is outside the calendar year".to_string())
    }

    /// Whole-year age at `at` for someone born on `birth`.
    pub fn age_years(&self, birth: &CalendarDate, at: &CalendarDate) -> Result<i64, String> {
        let birth_index = self.day_index_for_date(birth)?;
        let at_index = self.day_index_for_date(at)?;
        if at_index < birth_index {
            return Err("at date precedes birth date".to_string());
        }
        let mut age = at.year - birth.year;
        if (at.month, at.day) < (birth.month, birth.day) {
            age -= 1;
        }
        Ok(age)
    }

    /// Whole-year age from absolute day indices.
    pub fn age_years_at_day(&self, birth_day_index: i64, at_day_index: i64) -> Result<i64, String> {
        let birth = self.date_for_day_index(birth_day_index)?;
        let at = self.date_for_day_index(at_day_index)?;
        self.age_years(&birth, &at)
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
                    return Err(format!(
                        "calendar month '{}' must have >= 1 day",
                        month.name
                    ));
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
        if !self.week_day_names.is_empty() && self.week_day_names.len() as i32 != self.days_per_week
        {
            return Err(format!(
                "week_day_names has {} entries but days_per_week is {}",
                self.week_day_names.len(),
                self.days_per_week
            ));
        }
        match &self.leap_rule {
            LeapYearRule::None => {}
            LeapYearRule::Gregorian => {
                let Some(month) = self
                    .months
                    .iter()
                    .find(|month| month.name.eq_ignore_ascii_case("february"))
                else {
                    return Err(
                        "Gregorian leap rule requires a February month in `months`".to_string()
                    );
                };
                if month.days != 28 {
                    return Err(format!(
                        "Gregorian February must have 28 common-year days (got {})",
                        month.days
                    ));
                }
            }
            LeapYearRule::Custom {
                month,
                every_years,
                extra_days,
                ..
            } => {
                if *every_years < 1 {
                    return Err(format!(
                        "custom leap rule every_years must be >= 1 (got {every_years})"
                    ));
                }
                if *extra_days < 1 {
                    return Err(format!(
                        "custom leap rule extra_days must be >= 1 (got {extra_days})"
                    ));
                }
                if !self
                    .months
                    .iter()
                    .any(|candidate| candidate.name.eq_ignore_ascii_case(month))
                {
                    return Err(format!(
                        "custom leap month '{month}' is not present in `months`"
                    ));
                }
            }
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
    ///
    /// `day_index` is a SIGNED offset from the project epoch: births and
    /// backstory precede the story's opening day, so negative values are
    /// valid (and the total-order index keeps comparing correctly). Only
    /// magnitudes — duration — and the intra-day clock are bounded.
    pub fn validate(&self, calendar: &CalendarDef) -> Result<(), String> {
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
    /// Internal byte range of `matched` for attribution; not part of the
    /// public `scan_scene_prices` contract.
    #[serde(default, skip)]
    #[schemars(skip)]
    pub start: usize,
    #[serde(default, skip)]
    #[schemars(skip)]
    pub end: usize,
}

/// Scan `text` for "<number> <unit>" price mentions, where `unit` is one of
/// `units` (case-insensitive, a trailing plural 's' tolerated). Numbers may be
/// digits or English words, including scale words and hyphenated tens. Pure and
/// dependency-free: a token-window scan, deliberately high-precision/low-recall.
pub fn extract_price_mentions(text: &str, units: &[String]) -> Vec<PriceMention> {
    let unit_set: std::collections::BTreeSet<String> =
        units.iter().map(|u| u.to_ascii_lowercase()).collect();
    if unit_set.is_empty() {
        return Vec::new();
    }
    let tokens: Vec<(&str, usize, usize)> = token_spans(text);
    let mut out: Vec<PriceMention> = Vec::new();
    for index in 0..tokens.len() {
        if out
            .iter()
            .any(|existing| tokens[index].1 >= existing.start && tokens[index].1 < existing.end)
        {
            continue;
        }
        for span_len in 1..=4 {
            let Some(unit_token) = tokens.get(index + span_len) else {
                break;
            };
            let Some(unit) = normalize_price_unit(unit_token.0, &unit_set) else {
                continue;
            };
            let Some(amount) = parse_amount_phrase(&tokens[index..index + span_len]) else {
                continue;
            };
            let start = tokens[index].1;
            let end = unit_token.2;
            if out
                .iter()
                .any(|existing| existing.start == start && existing.end == end)
            {
                continue;
            }
            out.push(PriceMention {
                amount,
                unit,
                matched: text[start..end].to_string(),
                start,
                end,
            });
        }
    }
    out.sort_by_key(|mention| mention.start);
    out
}

fn token_spans(text: &str) -> Vec<(&str, usize, usize)> {
    let mut tokens = Vec::new();
    let mut search_from = 0;
    for token in text.split_whitespace() {
        let Some(relative) = text[search_from..].find(token) else {
            continue;
        };
        let start = search_from + relative;
        let end = start + token.len();
        tokens.push((token, start, end));
        search_from = end;
    }
    tokens
}

fn normalize_price_unit(
    token: &str,
    unit_set: &std::collections::BTreeSet<String>,
) -> Option<String> {
    let raw = token
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }
    let singular = raw.strip_suffix('s').unwrap_or(raw.as_str());
    if unit_set.contains(&raw) {
        Some(raw)
    } else if unit_set.contains(singular) {
        Some(singular.to_string())
    } else {
        None
    }
}

fn parse_amount_phrase(tokens: &[(&str, usize, usize)]) -> Option<f64> {
    let mut total = 0.0;
    let mut current = 0.0;
    let mut saw_value = false;
    for (token, _, _) in tokens {
        for part in token
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
            .split('-')
        {
            let word = part.to_ascii_lowercase();
            if word.is_empty() || word == "and" {
                continue;
            }
            let numeric = word.replace(',', "");
            if let Ok(value) = numeric.parse::<f64>() {
                current += value;
                saw_value = true;
                continue;
            }
            let value = match word.as_str() {
                "a" | "an" => 1.0,
                "zero" => 0.0,
                "one" => 1.0,
                "two" => 2.0,
                "three" => 3.0,
                "four" => 4.0,
                "five" => 5.0,
                "six" => 6.0,
                "seven" => 7.0,
                "eight" => 8.0,
                "nine" => 9.0,
                "ten" => 10.0,
                "eleven" => 11.0,
                "twelve" => 12.0,
                "thirteen" => 13.0,
                "fourteen" => 14.0,
                "fifteen" => 15.0,
                "sixteen" => 16.0,
                "seventeen" => 17.0,
                "eighteen" => 18.0,
                "nineteen" => 19.0,
                "twenty" => 20.0,
                "thirty" => 30.0,
                "forty" => 40.0,
                "fifty" => 50.0,
                "sixty" => 60.0,
                "seventy" => 70.0,
                "eighty" => 80.0,
                "ninety" => 90.0,
                "hundred" => {
                    current = if current == 0.0 { 1.0 } else { current } * 100.0;
                    saw_value = true;
                    continue;
                }
                "thousand" => {
                    total += if current == 0.0 { 1.0 } else { current } * 1_000.0;
                    current = 0.0;
                    saw_value = true;
                    continue;
                }
                "million" => {
                    total += if current == 0.0 { 1.0 } else { current } * 1_000_000.0;
                    current = 0.0;
                    saw_value = true;
                    continue;
                }
                _ => return None,
            };
            current += value;
            saw_value = true;
        }
    }
    saw_value.then_some(total + current)
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
        assert_eq!(
            (plural[0].amount, plural[0].unit.as_str()),
            (12.0, "silver")
        );
        // unknown unit and empty vocabulary yield nothing
        assert!(extract_price_mentions("10 apples", &units).is_empty());
        assert!(extract_price_mentions("5 silver", &[]).is_empty());
    }

    #[test]
    fn extract_price_mentions_parses_number_words_and_offsets() {
        let units = vec![
            "silver".to_string(),
            "gold".to_string(),
            "marks".to_string(),
        ];
        let text = "A loaf costs five silver and a sword twenty-five gold.";
        let m = extract_price_mentions(text, &units);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].amount, m[0].unit.as_str()), (5.0, "silver"));
        assert_eq!((m[1].amount, m[1].unit.as_str()), (25.0, "gold"));
        assert_eq!(&text[m[0].start..m[0].end], m[0].matched);
        assert_eq!(&text[m[1].start..m[1].end], m[1].matched);

        let text = "She paid a hundred marks for the compass and twelve marks for the chart.";
        let m = extract_price_mentions(text, &units);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].amount, m[0].unit.as_str()), (100.0, "marks"));
        assert_eq!((m[1].amount, m[1].unit.as_str()), (12.0, "marks"));

        let m = extract_price_mentions("The fee is one hundred and twenty marks.", &units);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].amount, 120.0);
        assert_eq!(
            &"The fee is one hundred and twenty marks."[m[0].start..m[0].end],
            m[0].matched
        );
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
            leap_rule: Default::default(),
        }
    }

    fn gregorian_months() -> Vec<CalendarMonth> {
        [
            ("January", 31),
            ("February", 28),
            ("March", 31),
            ("April", 30),
            ("May", 31),
            ("June", 30),
            ("July", 31),
            ("August", 31),
            ("September", 30),
            ("October", 31),
            ("November", 30),
            ("December", 31),
        ]
        .into_iter()
        .map(|(name, days)| CalendarMonth {
            name: name.to_string(),
            days,
        })
        .collect()
    }

    fn gregorian_leap_calendar() -> CalendarDef {
        CalendarDef {
            days_per_week: 7,
            hours_per_day: 24,
            week_day_names: Vec::new(),
            months: gregorian_months(),
            days_per_year: 365,
            epoch_label: None,
            leap_rule: LeapYearRule::Gregorian,
        }
    }

    fn date(year: i64, month: i32, day: i32) -> CalendarDate {
        CalendarDate { year, month, day }
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
    fn clock_accepts_negative_day_index_and_rejects_unknown_precision() {
        let cal = gregorian();
        // Births and backstory precede the story's opening day (the epoch),
        // so day_index is a signed offset — negative values must validate.
        assert!(
            StoryClock {
                day_index: Some(-8913),
                precision: Some("day".into()),
                ..Default::default()
            }
            .validate(&cal)
            .is_ok()
        );
        // ...and order before the epoch in the total-order index.
        let birth = StoryClock {
            day_index: Some(-8913),
            ..Default::default()
        };
        let opening = StoryClock {
            day_index: Some(1),
            ..Default::default()
        };
        assert!(birth.total_index(&cal).unwrap() < opening.total_index(&cal).unwrap());
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

    #[test]
    fn clock_still_rejects_negative_duration() {
        let cal = gregorian();
        assert!(
            StoryClock {
                duration_days: Some(-1.0),
                ..Default::default()
            }
            .validate(&cal)
            .is_err()
        );
    }

    #[test]
    fn gregorian_leap_years_exclude_1900_and_include_2000() {
        let cal = gregorian_leap_calendar();
        assert!(!cal.is_leap_year(1900));
        assert!(cal.is_leap_year(2000));
        assert!(cal.is_leap_year(2024));
        assert!(!cal.is_leap_year(2023));
        assert_eq!(cal.days_in_year(1900), 365);
        assert_eq!(cal.days_in_year(2000), 366);
        assert_eq!(cal.days_in_month(2000, 2), Some(29));
        assert_eq!(cal.days_in_month(1900, 2), Some(28));
    }

    #[test]
    fn february_29_round_trips_only_on_leap_years() {
        let cal = gregorian_leap_calendar();
        let leap_day = date(2000, 2, 29);
        let index = cal.day_index_for_date(&leap_day).expect("Feb 29 2000");
        assert_eq!(cal.date_for_day_index(index).unwrap(), leap_day);
        assert!(cal.day_index_for_date(&date(1900, 2, 29)).is_err());
        assert!(cal.day_index_for_date(&date(2023, 2, 29)).is_err());
        assert_eq!(
            cal.day_index_for_date(&date(2024, 2, 29)).unwrap(),
            cal.day_index_for_date(&date(2024, 3, 1)).unwrap() - 1
        );
    }

    #[test]
    fn multi_year_date_spans_account_for_leap_days() {
        let cal = gregorian_leap_calendar();
        let start = cal.day_index_for_date(&date(1896, 1, 1)).unwrap();
        let end = cal.day_index_for_date(&date(1904, 1, 1)).unwrap();
        // 1896 and 1904 are leap years, but 1904's Feb 29 has not happened by
        // Jan 1; 1900 is not a leap year.
        assert_eq!(end - start, 8 * 365 + 1);
        for ordinal in [0, 1, 365, 366, 10_000, -1, -366, -10_000] {
            let date = cal.date_for_day_index(ordinal).unwrap();
            assert_eq!(cal.day_index_for_date(&date).unwrap(), ordinal);
        }
    }

    #[test]
    fn age_across_a_leap_day_uses_the_actual_birthday() {
        let cal = gregorian_leap_calendar();
        assert_eq!(
            cal.age_years(&date(2000, 2, 29), &date(2001, 2, 28))
                .unwrap(),
            0
        );
        assert_eq!(
            cal.age_years(&date(2000, 2, 29), &date(2001, 3, 1))
                .unwrap(),
            1
        );
        assert_eq!(
            cal.age_years(&date(2000, 2, 29), &date(2004, 2, 29))
                .unwrap(),
            4
        );
        // Year 0 is a Gregorian leap year; a pre-epoch Feb 29 birthday still
        // keeps its actual anniversary in year 1.
        assert_eq!(cal.age_years(&date(0, 2, 29), &date(1, 2, 28)).unwrap(), 0);
        assert_eq!(cal.age_years(&date(0, 2, 29), &date(1, 3, 1)).unwrap(), 1);
        assert_eq!(
            cal.age_years(&date(1899, 3, 1), &date(1900, 2, 28))
                .unwrap(),
            0
        );
        assert_eq!(
            cal.age_years(&date(1899, 3, 1), &date(1900, 3, 1)).unwrap(),
            1
        );
    }

    #[test]
    fn custom_leap_rule_adds_days_to_the_named_month() {
        let mut cal = gregorian_leap_calendar();
        cal.months = vec![
            CalendarMonth {
                name: "First".into(),
                days: 10,
            },
            CalendarMonth {
                name: "Second".into(),
                days: 10,
            },
            CalendarMonth {
                name: "Third".into(),
                days: 10,
            },
            CalendarMonth {
                name: "Fourth".into(),
                days: 10,
            },
        ];
        cal.days_per_year = 40;
        cal.leap_rule = LeapYearRule::Custom {
            month: "Second".into(),
            every_years: 2,
            extra_days: 2,
            anchor_year: 0,
        };
        assert!(cal.validate().is_ok());
        assert_eq!(cal.days_in_month(2, 2), Some(12));
        assert_eq!(cal.days_in_month(4, 2), Some(12));
        assert_eq!(cal.days_in_month(3, 2), Some(10));
        let index = cal.day_index_for_date(&date(2, 2, 12)).unwrap();
        assert_eq!(cal.date_for_day_index(index).unwrap(), date(2, 2, 12));

        // Year 2 has 42 days; year 3 starts after both extra days.
        assert_eq!(cal.day_index_for_date(&date(3, 1, 1)).unwrap(), 82);
        assert_eq!(cal.date_for_day_index(81).unwrap(), date(2, 4, 10));
        assert_eq!(cal.date_for_day_index(82).unwrap(), date(3, 1, 1));
        for ordinal in [0, 39, 40, 41, 81, 82, 123, 164, 205, 246] {
            let date = cal.date_for_day_index(ordinal).unwrap();
            assert_eq!(cal.day_index_for_date(&date).unwrap(), ordinal);
        }
        // Leap years 2 and 4 have occurred; year 7 follows three leap years
        // (2, 4, 6), each contributing two days.
        assert_eq!(cal.day_index_for_date(&date(7, 1, 1)).unwrap(), 246);
    }

    #[test]
    fn legacy_calendar_json_defaults_to_fixed_years() {
        let raw = serde_json::json!({
            "days_per_week": 7,
            "hours_per_day": 24,
            "week_day_names": [],
            "months": [],
            "days_per_year": 365,
            "epoch_label": null
        });
        let cal: CalendarDef = serde_json::from_value(raw).unwrap();
        assert_eq!(cal.leap_rule, LeapYearRule::None);
        assert!(!cal.is_leap_year(2000));
        assert!(cal.validate().is_ok());
    }

    #[test]
    fn gregorian_preset_is_valid_and_leap_aware() {
        let cal = CalendarDef::gregorian();
        assert!(cal.validate().is_ok());
        assert!(cal.is_leap_year(2000));
        assert_eq!(cal.date_for_day_index(0).unwrap(), date(1, 1, 1));
        assert_eq!(
            cal.date_for_day_index(cal.day_index_for_date(&date(2000, 2, 29)).unwrap())
                .unwrap(),
            date(2000, 2, 29)
        );
    }

    #[test]
    fn legacy_calendar_without_rule_keeps_fixed_years() {
        let mut cal = gregorian_leap_calendar();
        cal.leap_rule = LeapYearRule::None;
        assert!(!cal.is_leap_year(2000));
        assert!(cal.day_index_for_date(&date(2000, 2, 29)).is_err());
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CharacterArcMilestone {
    pub label: String,
    pub placement: Option<StoryPlacement>,
    pub description: String,
    #[serde(default)]
    pub unlocks: Vec<String>,
    /// Where the milestone was actually reached in the manuscript, once
    /// demonstrated. `None` means the milestone is still pending. Additive and
    /// serde-defaulted so pre-existing milestone JSON (which lacks the field)
    /// keeps deserializing. Drives `arc_milestone_audit`.
    #[serde(default)]
    pub reached_at: Option<StoryPlacement>,
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
    /// Alternate names the character answers to (nicknames, titles, an
    /// in-world name decided after the record exists). Searchable via
    /// find_entity exactly like the primary name; rename preserves the old
    /// name here automatically.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Optional (defect item 5): minor characters rarely arrive with a voice
    /// profile. Missing AND explicit null both default to an empty profile.
    #[serde(default, deserialize_with = "null_to_default")]
    pub voice_profile: CharacterVoiceProfileData,
    /// Optional (defect item 5), same null/missing tolerance as voice_profile.
    #[serde(default, deserialize_with = "null_to_default")]
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
    /// "semantic_references", "previous_scene_tail". Supported scene sections:
    /// "location",
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
    /// Realized-intensity trend fed forward from recent annotated chapters
    /// (T-109): the mean intensity of the last up-to-3 annotated chapters
    /// strictly before the current one, with its direction (rising / falling /
    /// flat), so the drafting agent sees the pacing trend it is about to extend.
    /// `None` when no prior chapter carries an intensity annotation. Rides the
    /// `pacing_directives` render/budget/trim path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realized_intensity_trend: Option<String>,
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
    /// Themes, conflicts, plot lines, and theme-connected motifs the current
    /// chapter plan explicitly targets. Hydrated so the drafting agent can
    /// advance the threads the plan asked it to advance; empty when the chapter
    /// has no plan or targets nothing.
    #[serde(default)]
    pub active_threads: Vec<ActiveThreadSummary>,
    /// Closing excerpt of the immediately preceding scene, so the drafting
    /// agent can hand off cleanly from its exit beat (emotional register,
    /// physical continuity). `None` at the very start of a book or when the
    /// preceding scene has no prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_scene_tail: Option<PreviousSceneTail>,
}

/// Closing excerpt of the scene immediately preceding the one being drafted,
/// surfaced in scene context so the scene-to-scene prose hand-off (exit beat,
/// emotional register, physical continuity) is visible to the drafting agent.
///
/// # Cross-rating elision
///
/// A mixed-rating chapter can place an Explicit scene next to a General one.
/// Because this context is assembled into a draft prompt that is then dispatched
/// to the route serving the *target* scene's rating, handing over an explicit
/// neighbour's prose verbatim would transmit explicit material to an agent that
/// is not cleared for it — the exact leak the rating-gated dispatch chokepoint
/// exists to prevent. So when the neighbour is explicit and the target scene is
/// not (or its rating cannot be established), `excerpt` carries the neighbour's
/// stored summary instead of its prose and [`Self::elided_reason`] explains the
/// substitution. See `docs/spindle-agent-config.md` ("Mixed-rating chapters").
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviousSceneTail {
    pub scene_id: String,
    pub chapter_number: i32,
    pub scene_order: i32,
    /// The neighbour's closing prose — or, when `elided_reason` is set, its
    /// non-explicit summary standing in for that prose.
    pub excerpt: String,
    /// Set when `excerpt` is a rating-elided substitute rather than the
    /// neighbour's actual closing prose. `None` on an ordinary hand-off, so a
    /// caller can tell an elided tail from a faithful one instead of silently
    /// reading a summary as prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elided_reason: Option<String>,
}

/// A narrative thread (theme, conflict, plot line, or theme-connected motif)
/// the current chapter plan explicitly targets, projected for briefing so the
/// drafting agent can advance it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActiveThreadSummary {
    pub id: String,
    /// "theme" | "conflict" | "plot_line" | "motif".
    pub kind: String,
    pub name: String,
    /// One-line statement/stakes/summary, truncated.
    pub statement: String,
    /// Entity status or placement summary; "" when none applies.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_expectation: Option<String>,
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
    /// Budget-capped fiction shelf digest for the active genre/style profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_shelf_digest: Option<crate::style::antislop::CompactShelfDigest>,
    /// Project-local on-voice excerpts from style-profile guidance (not a corpus dump).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voice_samples: Vec<crate::style::antislop::VoiceSample>,
    /// Compact do-not-repeat notes from style-profile avoid rules / shelf hooks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_negatives: Vec<crate::style::antislop::SceneNegative>,
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
    pub compact_shelf_digest: Option<crate::style::antislop::CompactShelfDigest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voice_samples: Vec<crate::style::antislop::VoiceSample>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_negatives: Vec<crate::style::antislop::SceneNegative>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_markdown: Option<String>,
}

/// Supported intent modes for `prepare_scene`.
pub const PREPARE_SCENE_INTENTS: [&str; 3] = ["draft", "revise", "continue"];

/// Scene preparation mode (`draft`, `revise`, or `continue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrepareSceneIntent {
    #[default]
    Draft,
    Revise,
    Continue,
}

impl PrepareSceneIntent {
    /// Canonical lowercase string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Revise => "revise",
            Self::Continue => "continue",
        }
    }
}

impl std::str::FromStr for PrepareSceneIntent {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "draft" => Ok(Self::Draft),
            "revise" => Ok(Self::Revise),
            "continue" => Ok(Self::Continue),
            other => Err(format!(
                "unsupported prepare_scene intent '{other}'; expected one of: {}",
                PREPARE_SCENE_INTENTS.join(", ")
            )),
        }
    }
}

impl<'de> Deserialize<'de> for PrepareSceneIntent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// Input parameters for `prepare_scene`, assembling a bounded scene preparation packet.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PrepareSceneInput {
    /// Project identifier.
    pub project_id: String,
    /// Branch identifier; defaults to the session or project active branch when omitted.
    #[serde(default)]
    pub branch_id: Option<String>,
    /// Existing scene identifier to prepare; resolves placement, cast, and setting when present.
    #[serde(default)]
    pub scene_id: Option<String>,
    /// Chapter identifier used to resolve book and chapter numbers when `scene_id` is omitted.
    #[serde(default)]
    pub chapter_id: Option<String>,
    /// Book number (1-based) when targeting by placement or falling back to the writer cursor.
    #[serde(default)]
    pub book_number: Option<i32>,
    /// Chapter number (1-based) when targeting by placement or falling back to the writer cursor.
    #[serde(default)]
    pub chapter_number: Option<i32>,
    /// Scene order within the chapter (1-based).
    #[serde(default)]
    pub scene_order: Option<i32>,
    /// Character identifiers present in the scene; resolved from the scene or chapter plan when omitted.
    #[serde(default)]
    pub character_ids: Vec<String>,
    /// Location identifier for the scene setting; resolved from the scene or chapter plan when omitted.
    #[serde(default)]
    pub location_id: Option<String>,
    /// Preparation intent (`draft`, `revise`, or `continue`); defaults to `draft`.
    #[serde(default)]
    pub intent: Option<PrepareSceneIntent>,
    /// Token budget cap for assembled scene context; hard constraints must fit within this budget.
    #[serde(default)]
    pub budget_tokens: Option<usize>,
    /// Optional research query to attach relevant project research claims and artifacts.
    #[serde(default)]
    pub research_query: Option<String>,
}

/// Dependency hashes captured when a scene preparation packet is assembled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScenePreparationDependencies {
    /// Content hash of the persisted scene prose at preparation time, if a scene exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_text_hash: Option<String>,
    /// Content hash of the branch-visible canon state for this placement.
    pub canon_snapshot_hash: String,
    /// Content hash of the project reader contract, narrator voice, style rules, and active style profile.
    pub style_revision_hash: String,
    /// Content hash of the chapter plan and outline covering this placement.
    pub plan_revision_hash: String,
}

/// Freshness evaluation comparing a prepared scene packet against current project state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScenePreparationFreshness {
    /// True when all dependency hashes match current project state.
    pub fresh: bool,
    /// Recomputed composite context revision token for the current state.
    pub current_context_revision_token: String,
    /// Current dependency hashes for scene text, canon, style, and plan.
    pub current_dependencies: ScenePreparationDependencies,
    /// Names of dependency components that changed (`scene_text`, `canon`, `style`, `plan`).
    #[serde(default)]
    pub stale_components: Vec<String>,
}

/// Read-only snapshot of the writer cursor and next-scene recommendation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PrepareSceneCursor {
    /// Scene identifier currently recorded on the writer cursor, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_scene_id: Option<String>,
    /// Story placement currently recorded on the writer cursor, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_placement: Option<StoryPlacement>,
    /// Next recommended scene identifier from writer state, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_scene_id: Option<String>,
    /// Current writer intent stored on the project cursor, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writer_intent: Option<WriterIntent>,
    /// Writer notes attached to the current cursor, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Resolved scene purpose, cast, setting, beats, and active narrative threads.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PrepareScenePurpose {
    /// Synopsis of the enclosing chapter from the chapter record or chapter plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_synopsis: Option<String>,
    /// Title of the target scene when persisted or planned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_title: Option<String>,
    /// Summary of the target scene when persisted or planned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_summary: Option<String>,
    /// Point-of-view character identifier for the scene, if assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pov_character_id: Option<String>,
    /// Resolved location identifier where the scene takes place.
    pub location_id: String,
    /// Resolved character identifiers participating in the scene.
    #[serde(default)]
    pub character_ids: Vec<String>,
    /// Effective content rating for the scene.
    pub content_rating: ContentRating,
    /// Expected chapter or scene beats to advance in this scene.
    #[serde(default)]
    pub expected_beats: Vec<String>,
    /// Active themes, conflicts, plot lines, and motifs targeted by this chapter.
    #[serde(default)]
    pub active_threads: Vec<ActiveThreadSummary>,
}

/// Compact reference to a canon, plan, or research record included in the preparation packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PrepareSceneEvidenceRef {
    /// Record or section identifier.
    pub id: String,
    /// Evidence category (`character`, `location`, `world_rule`, `canonical_fact`, `chapter_plan`, `scene`, `research`).
    pub kind: String,
    /// Human-readable title or summary label.
    pub label: String,
}

/// Report of an optional or degraded retrieval pass during scene preparation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PrepareSceneUnavailablePass {
    /// Name of the retrieval pass (`semantic_references`, `research_recall`, `drafting_packet_v1`).
    pub pass: String,
    /// Explanation of why the pass was degraded or unavailable.
    pub reason: String,
}

/// Relevant research claim or artifact surfaced for the scene.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PrepareSceneResearchItem {
    /// Research claim or artifact identifier.
    pub id: String,
    /// Item kind (`claim` or `artifact`).
    pub kind: String,
    /// Statement or title/excerpt summary.
    pub summary: String,
    /// Associated research source identifier, if linked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

/// Budget, section inclusion/omission, evidence, and degraded-pass report for `prepare_scene`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PrepareSceneCoverage {
    /// Token budget applied to the scene preparation packet.
    pub budget_tokens: usize,
    /// Estimated token count of the assembled scene context after trimming.
    pub estimated_tokens: usize,
    /// Estimated token cost of non-truncatable hard constraints.
    pub hard_constraint_tokens: usize,
    /// Context sections retained within the token budget.
    #[serde(default)]
    pub included_sections: Vec<String>,
    /// Context sections omitted or truncated to respect the token budget.
    #[serde(default)]
    pub truncated_sections: Vec<String>,
    /// Compact evidence references backing the prepared packet.
    #[serde(default)]
    pub evidence_references: Vec<PrepareSceneEvidenceRef>,
    /// Retrieval passes that were unavailable or degraded during assembly.
    #[serde(default)]
    pub unavailable_passes: Vec<PrepareSceneUnavailablePass>,
}

/// Assembled scene preparation packet returned by `prepare_scene`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PrepareSceneOutput {
    /// Unique identifier for this preparation packet (`prep:<hash>`).
    pub preparation_id: String,
    /// Project identifier.
    pub project_id: String,
    /// Resolved branch identifier.
    pub branch_id: String,
    /// Resolved branch name.
    pub branch_name: String,
    /// Effective preparation intent (`draft`, `revise`, or `continue`).
    pub intent: PrepareSceneIntent,
    /// Resolved story placement (`book_number`, `chapter_number`, `scene_order`).
    pub placement: StoryPlacement,
    /// Enclosing chapter identifier, if a chapter record exists for this placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    /// Persisted scene identifier, if a scene record exists at this placement on the branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    /// Current persisted scene version (`revision_count`), if a scene record exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_version: Option<i32>,
    /// Composite freshness token over scene text, canon, style, and plan dependencies.
    pub context_revision_token: String,
    /// Individual dependency hashes for staleness detection at commit time.
    pub dependency_tokens: ScenePreparationDependencies,
    /// Read-only snapshot of the writer cursor and next-scene recommendation.
    pub writer_cursor: PrepareSceneCursor,
    /// Scene purpose, cast, setting, beats, and active narrative threads.
    pub scene_purpose: PrepareScenePurpose,
    /// Non-truncatable hard constraints (story so far, temporal anchor, quantities, world rules, canonical facts, secret envelopes).
    #[serde(default)]
    pub hard_constraints: Vec<HardConstraint>,
    /// Project reader contract.
    pub reader_contract: ReaderContract,
    /// Active genre/style directive, when configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_directive: Option<crate::style::StyleDirective>,
    /// Budget-capped fiction shelf digest for the active genre/style profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_shelf_digest: Option<crate::style::antislop::CompactShelfDigest>,
    /// Project-local on-voice excerpts from style-profile guidance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voice_samples: Vec<crate::style::antislop::VoiceSample>,
    /// Compact do-not-repeat notes from style-profile avoid rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_negatives: Vec<crate::style::antislop::SceneNegative>,
    /// Assembled novel-layer context (world rules, subjects, timeline, knowledge, promises, pacing, threads, previous scene tail).
    pub novel: SceneContextNovelLayer,
    /// Assembled scene-layer context (location, world state, character states, relationships, agency check).
    pub scene: SceneContextSceneLayer,
    /// Existing scene prose when `intent` is `revise` or `continue` and a persisted scene exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_prose: Option<String>,
    /// Relevant research claims and artifacts when `research_query` is provided.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub research: Vec<PrepareSceneResearchItem>,
    /// Optional `DraftingPacketV1` contract when a persisted scene and chapter record exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drafting_packet_v1: Option<crate::drafting::DraftingPacketV1>,
    /// Coverage, budget, omission, evidence, and degraded-pass report.
    pub coverage: PrepareSceneCoverage,
}

/// Declared authorship of committed scene prose, independent of provider
/// rating-clearance receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SceneAuthorship {
    /// The author wrote or edited the prose.
    #[default]
    Human,
    /// A connected assistant or configured generation authored the prose.
    Assistant,
}

/// Whether a committed scene operation can be undone, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommitSceneUndoSupport {
    /// The prose and its approved derived state can be reversed coherently.
    Reversible,
    /// Only a branch checkpoint can restore the prior state.
    CheckpointOnly,
    /// The operation has effects that cannot be reversed.
    Irreversible,
}

/// Kind of follow-up work a successful commit still requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommitSceneFollowUpKind {
    /// Persist a chapter-completion summary after the final scene.
    ChapterSummary,
    /// Run a checkpoint audit for the completed chapter window.
    CheckpointAudit,
    /// Stage or ratify mined canon proposals explicitly.
    CanonMining,
    /// Advance or resume the enclosing authoring run.
    AuthoringRun,
}

/// One required follow-up after a scene commit.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneFollowUp {
    /// Follow-up category.
    pub kind: CommitSceneFollowUpKind,
    /// Human-readable action the caller still needs to take.
    pub message: String,
}

/// Story placement used inside the `commit_scene` continuity package.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneStoryPlacement {
    /// Book number (1-based).
    pub book_number: i32,
    /// Chapter number (1-based).
    pub chapter_number: i32,
    /// Scene order within the chapter (1-based), when known.
    #[serde(default)]
    pub scene_order: Option<i32>,
    /// Optional note about the placement.
    #[serde(default)]
    pub note: Option<String>,
}

impl From<&CommitSceneStoryPlacement> for StoryPlacement {
    fn from(value: &CommitSceneStoryPlacement) -> Self {
        Self {
            book_number: value.book_number,
            chapter_number: value.chapter_number,
            scene_order: value.scene_order,
            note: value.note.clone(),
        }
    }
}

/// Character state fields committed with a scene.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneCharacterStatePatch {
    /// Emotional-state keys and their new values.
    #[serde(default)]
    pub emotional_state: BTreeMap<String, serde_json::Value>,
    /// Current goals after the scene; omitted entries inherit the prior state.
    #[serde(default)]
    pub goals: Option<Vec<String>>,
    /// Current status descriptors after the scene.
    #[serde(default)]
    pub status: Option<Vec<String>>,
    /// Author notes about the state change.
    #[serde(default)]
    pub notes: Option<Vec<String>>,
    /// Short summary of what caused the change.
    #[serde(default)]
    pub source_summary: Option<String>,
}

impl From<CommitSceneCharacterStatePatch> for CharacterStatePatch {
    fn from(value: CommitSceneCharacterStatePatch) -> Self {
        Self {
            emotional_state: value.emotional_state,
            goals: value.goals,
            status: value.status,
            notes: value.notes,
            source_summary: value.source_summary,
        }
    }
}

/// One character state change committed with a scene.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneCharacterStateEntry {
    /// Character record id whose state changes.
    pub character_id: String,
    /// New state fields; omitted fields inherit the prior state.
    pub changes: CommitSceneCharacterStatePatch,
}

/// One canonical fact established by the committed scene.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneCanonicalFactEntry {
    /// Optional fact category label; `predicate` is used when omitted.
    #[serde(default)]
    pub fact_type: Option<String>,
    /// Optional legacy short key; used as the predicate when `predicate` is absent.
    #[serde(default)]
    pub key: Option<String>,
    /// Optional legacy value string; `value_text` takes precedence.
    #[serde(default)]
    pub value: Option<String>,
    /// Subject table the fact belongs to (for example `character` or `project`).
    #[serde(default)]
    pub subject_table: Option<String>,
    /// Subject record id when the fact is about a specific entity.
    #[serde(default)]
    pub subject_id: Option<String>,
    /// Fact predicate or attribute name.
    #[serde(default)]
    pub predicate: Option<String>,
    /// Value shape (`string`, `number`, or `json`); inferred when omitted.
    #[serde(default)]
    pub value_kind: Option<String>,
    /// Text value for string facts.
    #[serde(default)]
    pub value_text: Option<String>,
    /// Numeric value for numeric facts.
    #[serde(default)]
    pub value_number: Option<f64>,
    /// Unit for a numeric value.
    #[serde(default)]
    pub value_unit: Option<String>,
    /// Structured JSON value for composite facts.
    #[serde(default)]
    pub value_json: Option<serde_json::Value>,
    /// Alternate names that should resolve to this fact.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Whether the fact is invariant, evolving, or conditional.
    #[serde(default)]
    pub scope: Option<CanonicalFactScope>,
    /// Story placement from which the fact is valid.
    #[serde(default)]
    pub valid_from: Option<CommitSceneStoryPlacement>,
    /// Story placement after which the fact is no longer valid.
    #[serde(default)]
    pub valid_until: Option<CommitSceneStoryPlacement>,
    /// Optional context explaining the fact.
    #[serde(default)]
    pub context: Option<String>,
    /// Existing fact id this entry supersedes.
    #[serde(default)]
    pub supersedes_fact_id: Option<String>,
}

impl From<CommitSceneCanonicalFactEntry> for CanonicalFactEntry {
    fn from(value: CommitSceneCanonicalFactEntry) -> Self {
        Self {
            fact_type: value.fact_type,
            key: value.key,
            value: value.value,
            subject_table: value.subject_table,
            subject_id: value.subject_id,
            predicate: value.predicate,
            value_kind: value.value_kind,
            value_text: value.value_text,
            value_number: value.value_number,
            value_unit: value.value_unit,
            value_json: value.value_json,
            aliases: value.aliases,
            scope: value.scope,
            valid_from: value.valid_from.as_ref().map(StoryPlacement::from),
            valid_until: value.valid_until.as_ref().map(StoryPlacement::from),
            context: value.context,
            supersedes_fact_id: value.supersedes_fact_id,
        }
    }
}

/// One relationship trust/tension change committed with a scene.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneRelationshipEntry {
    /// First character record id in the relationship.
    pub character_a_id: String,
    /// Second character record id in the relationship.
    pub character_b_id: String,
    /// Trust change applied by this scene.
    pub trust_delta: i32,
    /// Tension change applied by this scene.
    pub tension_delta: i32,
    /// Why the relationship changed.
    pub reason: String,
}

impl From<CommitSceneRelationshipEntry> for RelationshipUpdateEntry {
    fn from(value: CommitSceneRelationshipEntry) -> Self {
        Self {
            character_a_id: value.character_a_id,
            character_b_id: value.character_b_id,
            trust_delta: value.trust_delta,
            tension_delta: value.tension_delta,
            reason: value.reason,
        }
    }
}

/// One on-page knowledge acquisition committed with a scene.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneKnowledgeEntry {
    /// Character record id who learns the fact.
    pub character_id: String,
    /// Short statement of the learned fact.
    pub fact: String,
    /// Optional note on how the fact was learned.
    #[serde(default)]
    pub source_summary: Option<String>,
    /// Secret canonical fact id revealed to this character, when applicable.
    #[serde(default)]
    pub secret_of_fact_id: Option<String>,
    /// Whether the reader also learns the fact; defaults to true.
    #[serde(default)]
    pub reader_visible: Option<bool>,
}

impl From<CommitSceneKnowledgeEntry> for KnowledgeLearnedEntry {
    fn from(value: CommitSceneKnowledgeEntry) -> Self {
        Self {
            character_id: value.character_id,
            fact: value.fact,
            source_summary: value.source_summary,
            secret_of_fact_id: value.secret_of_fact_id,
            reader_visible: value.reader_visible,
        }
    }
}

/// A quantity reading committed with a scene.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneQuantityState {
    /// Numeric amount; must be non-negative.
    #[serde(default)]
    pub amount: Option<f64>,
    /// Unit of the amount (for example `gold`).
    #[serde(default)]
    pub unit: Option<String>,
    /// Declared progression band, validated against the project scheme.
    #[serde(default)]
    pub band: Option<String>,
    /// Why the reading changed, when it reflects a deliberate jump.
    #[serde(default)]
    pub change_reason: Option<String>,
}

impl From<CommitSceneQuantityState> for QuantityState {
    fn from(value: CommitSceneQuantityState) -> Self {
        Self {
            amount: value.amount,
            unit: value.unit,
            band: value.band,
            change_reason: value.change_reason,
        }
    }
}

/// One author-declared quantity state committed with a scene.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneQuantityStateEntry {
    /// Subject table the quantity belongs to (for example `character`).
    pub subject_table: String,
    /// Record id of the subject that owns the quantity.
    pub subject_id: String,
    /// Declared measure name, validated against the project scheme when one exists.
    pub measure: String,
    /// New quantity reading.
    pub state: CommitSceneQuantityState,
}

/// One author-declared narrative promise planted in the committed scene.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitScenePromiseEntry {
    /// Promise category (for example `mystery` or `threat`).
    pub promise_type: String,
    /// What the prose promises the reader.
    pub description: String,
    /// Story placement where the promise is planted.
    pub planted_at: CommitSceneStoryPlacement,
    /// Planned payoff placement, when one is known.
    #[serde(default)]
    pub planned_payoff: Option<CommitSceneStoryPlacement>,
    /// Optional author notes about the promise.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// One author-declared timeline event established in the committed scene.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneTimelineEventEntry {
    /// Short event title.
    pub title: String,
    /// Event category (for example `battle` or `discovery`).
    pub event_type: String,
    /// Story placement where the event occurs.
    pub placement: CommitSceneStoryPlacement,
    /// Summary of what happened.
    pub summary: String,
    /// Related entity record ids.
    #[serde(default)]
    pub related_entity_ids: Vec<String>,
}

/// Severity of a scene-commit validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommitSceneIssueSeverity {
    /// A blocking finding; the commit wrote nothing.
    Error,
    /// An advisory finding that accompanied a successful commit.
    Warning,
    /// Purely informational finding.
    Info,
}

/// One validation finding returned by `commit_scene`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneIssue {
    /// Finding severity.
    pub severity: CommitSceneIssueSeverity,
    /// Deterministic check that produced the finding.
    pub check_type: String,
    /// Human-readable finding.
    pub message: String,
    /// Related entity record ids.
    #[serde(default)]
    pub entity_ids: Vec<String>,
    /// Suggested remediation, when one is known.
    #[serde(default)]
    pub suggested_action: Option<String>,
}

/// Typed author-declared continuity package applied with a scene commit.
///
/// These are explicit author decisions. Mined canon proposals are never
/// applied here and still require `decide_canon_deltas` ratification.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneContinuityPackage {
    /// Character state changes established in this scene.
    #[serde(default)]
    pub character_states: Vec<CommitSceneCharacterStateEntry>,
    /// Canonical facts established in this scene.
    #[serde(default)]
    pub canonical_facts: Vec<CommitSceneCanonicalFactEntry>,
    /// Relationship trust/tension deltas established in this scene.
    #[serde(default)]
    pub relationship_updates: Vec<CommitSceneRelationshipEntry>,
    /// Character knowledge acquired on the page, including secret reveals.
    #[serde(default)]
    pub knowledge_learned: Vec<CommitSceneKnowledgeEntry>,
    /// Quantity readings established in this scene.
    #[serde(default)]
    pub quantity_states: Vec<CommitSceneQuantityStateEntry>,
    /// Narrative promises planted in this scene.
    #[serde(default)]
    pub narrative_promises: Vec<CommitScenePromiseEntry>,
    /// Timeline events established in this scene.
    #[serde(default)]
    pub timeline_events: Vec<CommitSceneTimelineEventEntry>,
}

impl CommitSceneContinuityPackage {
    /// Total number of declared continuity entries, used to detect empty packages.
    pub fn len(&self) -> usize {
        self.character_states.len()
            + self.canonical_facts.len()
            + self.relationship_updates.len()
            + self.knowledge_learned.len()
            + self.quantity_states.len()
            + self.narrative_promises.len()
            + self.timeline_events.len()
    }

    /// True when no continuity entry is declared.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One realized beat annotation committed with a scene.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SceneBeatAnnotationInput {
    /// Beat category (for example `action` or `revelation`).
    pub beat_type: String,
    /// What happens in the realized beat.
    pub summary: String,
}

/// Optional chapter-completion summary committed with the final scene of a chapter.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneChapterSummary {
    /// Chapter synopsis after this scene.
    pub summary: String,
    /// Key events in the chapter.
    #[serde(default)]
    pub key_events: Vec<String>,
    /// Character changes across the chapter.
    #[serde(default)]
    pub character_changes: Vec<String>,
    /// Relationship shifts across the chapter.
    #[serde(default)]
    pub relationship_shifts: Vec<String>,
    /// Character-arc advances across the chapter.
    #[serde(default)]
    pub arc_advances: Vec<String>,
    /// Narrative promise events across the chapter.
    #[serde(default)]
    pub promise_events: Vec<String>,
}

/// Authoring-run scene phase a caller expects before a commit advances it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommitSceneRunPhase {
    /// The scene is planned but no draft exists yet.
    Pending,
    /// A draft has been saved for the scene.
    DraftSaved,
    /// The scene's continuity changes have been committed.
    ChangesCommitted,
    /// Beats have been annotated; the fully committed scene state.
    BeatsAnnotated,
}

/// Input for `commit_scene`, the single atomic write for ordinary scene work.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneInput {
    /// Project identifier.
    pub project_id: String,
    /// Branch to write on; defaults to the project's active branch.
    #[serde(default)]
    pub branch_id: Option<String>,
    /// Preparation packet identifier returned by `prepare_scene`.
    #[serde(default)]
    pub preparation_id: Option<String>,
    /// Composite context revision token from `prepare_scene`, revalidated before writing.
    #[serde(default)]
    pub context_revision_token: Option<String>,
    /// Optional individual dependency hashes for detailed staleness reporting.
    #[serde(default)]
    pub dependency_tokens: Option<ScenePreparationDependencies>,
    /// Existing scene identifier; when omitted, provide the placement fields.
    #[serde(default)]
    pub scene_id: Option<String>,
    /// Book number (1-based) for a new scene.
    #[serde(default)]
    pub book_number: Option<i32>,
    /// Chapter number (1-based) for a new scene.
    #[serde(default)]
    pub chapter_number: Option<i32>,
    /// Scene order within the chapter (1-based) for a new scene.
    #[serde(default)]
    pub scene_order: Option<i32>,
    /// Expected current scene version; a mismatch rejects the commit as stale.
    #[serde(default)]
    pub expected_scene_version: Option<i32>,
    /// Full scene prose. Exactly one of `full_text` or `prose_path` is required.
    /// This text is persisted and never echoed back in the receipt.
    #[serde(default)]
    pub full_text: String,
    /// Workspace-relative path to a UTF-8 scene prose file. Exactly one of
    /// `full_text` or `prose_path` is required. The path is resolved by the
    /// primary process against the folder containing `.spindle/`; dotfiles and
    /// `.spindle/` internals are rejected.
    #[serde(default)]
    pub prose_path: Option<String>,
    /// Optional SHA-256 of the source bytes (file bytes, or inline UTF-8) for
    /// integrity checking before the commit.
    #[serde(default)]
    pub expected_sha256: Option<String>,
    /// Concise scene summary.
    pub summary: String,
    /// Scene content rating.
    pub content_rating: ContentRating,
    /// Optional location record id where the scene takes place.
    #[serde(default)]
    pub location_id: Option<String>,
    /// Optional scene tone label.
    #[serde(default)]
    pub tone: Option<String>,
    /// Declared authorship of the prose.
    #[serde(default)]
    pub authorship: SceneAuthorship,
    /// Optional provider generation receipt id required for explicit prose.
    #[serde(default)]
    pub generation_receipt: Option<String>,
    /// Typed author-declared continuity changes applied atomically with the prose.
    #[serde(default)]
    pub continuity: CommitSceneContinuityPackage,
    /// Realized beat annotations for the scene.
    #[serde(default)]
    pub beat_annotations: Vec<SceneBeatAnnotationInput>,
    /// Optional chapter-completion summary, normally supplied for the final scene.
    #[serde(default)]
    pub chapter_summary: Option<CommitSceneChapterSummary>,
    /// Optional enclosing authoring run whose phase advances on a live success.
    #[serde(default)]
    pub run_id: Option<String>,
    /// Expected authoring scene phase before the run phase may advance.
    #[serde(default)]
    pub expected_phase: Option<CommitSceneRunPhase>,
    /// When true, run the full preflight and report proposed effects without writing.
    #[serde(default)]
    pub dry_run: bool,
    /// Client-generated key that makes retries safe; reuse with different content fails.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// Coverage and skipped-check report for a scene commit.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneCoverage {
    /// Deterministic checks that ran during preflight.
    #[serde(default)]
    pub checks_run: Vec<String>,
    /// Checks skipped and why, when a deliberate scope choice was made.
    #[serde(default)]
    pub checks_skipped: Vec<String>,
    /// Total validation findings (blocking and advisory).
    #[serde(default)]
    pub findings_total: usize,
    /// Advisory findings that did not block the commit.
    #[serde(default)]
    pub advisory_total: usize,
}

/// Identifiers produced by applying a scene's continuity package.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneAppliedDeltas {
    /// Total number of applied continuity entries.
    #[serde(default)]
    pub total_applied: usize,
    /// Created character-state record ids.
    #[serde(default)]
    pub character_state_ids: Vec<String>,
    /// Created canonical-fact record ids.
    #[serde(default)]
    pub canonical_fact_ids: Vec<String>,
    /// Updated relationship record ids.
    #[serde(default)]
    pub relationship_ids: Vec<String>,
    /// Created knowledge-fact record ids.
    #[serde(default)]
    pub knowledge_fact_ids: Vec<String>,
    /// Created quantity-state record ids.
    #[serde(default)]
    pub quantity_state_ids: Vec<String>,
    /// Created narrative-promise record ids.
    #[serde(default)]
    pub narrative_promise_ids: Vec<String>,
    /// Created timeline-event record ids.
    #[serde(default)]
    pub timeline_event_ids: Vec<String>,
}

/// Writer cursor position after a live scene commit.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneCursor {
    /// Scene identifier recorded on the cursor.
    pub scene_id: String,
    /// Book number of the committed scene.
    pub book_number: i32,
    /// Chapter number of the committed scene.
    pub chapter_number: i32,
    /// Scene order of the committed scene.
    pub scene_order: i32,
}

/// Where `commit_scene` read the prose and its integrity receipt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CommitSceneProseSource {
    Inline,
    File,
}

/// Prose source, relative path (file mode), raw byte count, and SHA-256.
/// File hashes cover the raw bytes before line-ending normalization.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneProseReceipt {
    pub source: CommitSceneProseSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub byte_count: usize,
    pub sha256: String,
}

/// Compact receipt returned by `commit_scene`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitSceneOutput {
    /// Stable operation identifier for audit and undo lookup.
    pub operation_id: String,
    /// Scene identifier written or previewed.
    pub scene_id: String,
    /// Current scene version after the commit.
    pub version_number: i32,
    /// Branch the commit targeted.
    pub branch_id: String,
    /// True when this was a dry run with no persisted effects.
    pub dry_run: bool,
    /// True when a retried idempotency key returned the original receipt.
    pub idempotent_replay: bool,
    /// Blocking and advisory validation findings.
    #[serde(default)]
    pub issues: Vec<CommitSceneIssue>,
    /// Advisory messages that accompanied a successful commit.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Number of blocking findings; nonzero means no writes occurred.
    #[serde(default)]
    pub blocking_count: usize,
    /// Deterministic checks run or skipped during preflight.
    pub coverage: CommitSceneCoverage,
    /// Applied continuity identifiers and counts.
    pub applied: CommitSceneAppliedDeltas,
    /// Number of beat annotations written.
    #[serde(default)]
    pub beat_count: usize,
    /// True when a chapter-completion summary was written.
    #[serde(default)]
    pub chapter_summary_saved: bool,
    /// Writer cursor after a live commit, if it moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writer_cursor: Option<CommitSceneCursor>,
    /// Work that still requires a later explicit call.
    #[serde(default)]
    pub follow_ups: Vec<CommitSceneFollowUp>,
    /// Whether and how this operation can be undone.
    pub undo_support: CommitSceneUndoSupport,
    /// Explanation when undo is limited or unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_reason: Option<String>,
    /// Prose source and integrity receipt; absent on pre-0.4 stored receipts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prose: Option<CommitSceneProseReceipt>,
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
    /// Themes, conflicts, plot lines, and theme-connected motifs the chapter
    /// plan explicitly targets. Mirrors the scene-context novel layer's
    /// `active_threads`; empty when the chapter has no plan or targets nothing.
    #[serde(default)]
    pub active_threads: Vec<ActiveThreadSummary>,
    pub scene_seed: ChapterBriefingSceneSeed,
    pub scene_context: Option<SceneContextOutput>,
    /// Same digest as the bundled scene context, repeated so a briefing-only
    /// caller does not have to unpack `scene_context` to see the shelves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_shelf_digest: Option<crate::style::antislop::CompactShelfDigest>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub omissions: Vec<WriterStateOmission>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub omissions: Vec<WriterStateOmission>,
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

/// A writer-state section or field that was reduced to fit `budget_tokens`.
/// `omitted` counts whole records for dropped/capped sections and truncated
/// records for text caps. Callers can re-run with a larger budget or use
/// `fetch_with` for a focused read.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct WriterStateOmission {
    pub section: String,
    /// `budget` (optional section dropped), `section_cap` (required list
    /// capped), or `text_cap` (required text truncated).
    pub reason: String,
    pub omitted: usize,
    pub fetch_with: String,
    pub hint: String,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MoveSceneInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    /// Optional id of the scene to move. When provided it identifies the
    /// source row unambiguously (safe even if duplicates or orphans share a
    /// position); the `from_*` fields become optional and are only validated
    /// against the scene's actual placement when supplied.
    #[serde(default)]
    pub scene_id: Option<String>,
    /// Source book. Required unless `scene_id` is provided.
    #[serde(default)]
    pub from_book_number: i32,
    /// Source chapter. Required unless `scene_id` is provided.
    #[serde(default)]
    pub from_chapter_number: i32,
    /// Source scene order. Required unless `scene_id` is provided.
    #[serde(default)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DeleteSceneInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    /// Optional id of the scene to delete. Preferred whenever the exact row
    /// is known: it identifies the row unambiguously and can never act on a
    /// different scene that happens to share a position. The scene must be
    /// on the project's active branch. When provided, the position fields
    /// are optional and only validated against the scene's actual placement
    /// if supplied.
    #[serde(default)]
    pub scene_id: Option<String>,
    /// Book of the scene. Required unless `scene_id` is provided.
    #[serde(default)]
    pub book_number: i32,
    /// Chapter of the scene. Required unless `scene_id` is provided.
    #[serde(default)]
    pub chapter_number: i32,
    /// Scene order within the chapter. Required unless `scene_id` is
    /// provided. Position addressing resolves a single active-branch row; it
    /// cannot disambiguate rows sharing a position.
    #[serde(default)]
    pub scene_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteSceneOutput {
    pub scene_id: String,
    pub branch_id: String,
    pub status: String,
    pub left_scene_order_gap: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct OperatorDeleteSceneInput {
    /// Record id returned by create_project (e.g. "project:abc123def")
    pub project_id: String,
    /// Optional id of the scene to delete (see `DeleteSceneInput::scene_id`).
    #[serde(default)]
    pub scene_id: Option<String>,
    /// Book of the scene. Required unless `scene_id` is provided.
    #[serde(default)]
    pub book_number: i32,
    /// Chapter of the scene. Required unless `scene_id` is provided.
    #[serde(default)]
    pub chapter_number: i32,
    /// Scene order within the chapter. Required unless `scene_id` is
    /// provided.
    #[serde(default)]
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

/// Declared authorship is independent of provider rating-clearance receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DraftAuthorship {
    #[default]
    Human,
    Assistant,
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
    /// Scene order within the chapter (1-based).
    pub scene_order: i32,
    /// Full scene prose text.
    #[serde(alias = "content", alias = "text")]
    pub full_text: String,
    /// Use assistant for prose composed or revised by the host AI. Human by
    /// default. Enables opt-in learning from a later human edit, not AI edits.
    #[serde(default)]
    pub authorship: DraftAuthorship,
    /// Concise scene summary.
    pub summary: String,
    /// Scene content rating.
    pub content_rating: ContentRating,
    /// Optional scene tone label.
    pub tone: Option<String>,
    // Optional server-side receipt id returned by `continue_generation`.
    // Required when saving explicit sexual prose with `content_rating:
    // "explicit"`. The receipt is provenance only: it proves the save was
    // authorized by a cleared, explicit-capable draft-route generation and
    // stamps `draft_origin: agent:<id>`. The caller-supplied `full_text` is
    // authoritative and is what Spindle persists — the receipt output is never
    // substituted for the prose.
    /// Optional receipt ID from `continue_generation` (required for `explicit` prose).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    /// Optional path to the local source file this scene was written from.
    #[serde(default)]
    pub source_path: Option<String>,
    // Optional location (record id returned by create_location) this scene is
    // set in. Persisted on the scene so the pre-draft temporal anchor can name
    // where the previous scene ended. Re-saving without it preserves the
    // existing value (it is never cleared by an omitted field).
    /// Optional location record ID where this scene takes place.
    #[serde(default)]
    pub location_id: Option<String>,
    /// Research source IDs cited by this scene.
    #[serde(default)]
    pub research_source_ids: Vec<String>,
    /// Research note IDs cited by this scene.
    #[serde(default)]
    pub research_note_ids: Vec<String>,
    /// Research claim IDs cited by this scene.
    #[serde(default)]
    pub research_claim_ids: Vec<String>,
    /// Optional research query pack input used during drafting.
    #[serde(default)]
    pub research_query_pack_input: Option<String>,
    /// Optional research context hash.
    #[serde(default)]
    pub research_context_hash: Option<String>,
    // On-page knowledge acquisitions to record with this scene (design §2.3
    // path 2). Each entry becomes a knowledge_fact row stamped at this scene's
    // placement; a `secret_of_fact_id` link expands that secret's circle of
    // trust. Additive — an empty vector behaves exactly as before.
    /// On-page character knowledge acquisitions recorded with this scene.
    #[serde(default)]
    pub knowledge_learned: Vec<KnowledgeLearnedEntry>,
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
    // Genre-voice mismatches against the project style contract (reader
    // contract style_notes, `style` world rules, narrator voice). A non-empty
    // list is a failed style gate — revise before committing. Coarse by
    // design; an empty list is not proof the scene is on-genre.
    /// Genre-voice mismatches against the project style contract.
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
    // Intra-scene temporal-coherence advisories (deterministic prose scan):
    // unsignaled time-of-day teleports, internal time drift, and unrendered
    // declared spans. Advisory-only — every entry is `severity: "warning"` and
    // never blocks a save.
    /// Advisory intra-scene temporal-coherence findings.
    #[serde(default)]
    pub temporal_findings: Vec<ConsistencyIssue>,
    // Fiction anti-slop scan of the saved prose. Soft and hard hits are
    // advisory on save — this field never fails the write. Hard over-limit
    // fail-closes only on the verify/revise path when that loop is on.
    /// Fiction anti-slop scan report for the saved prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anti_slop: Option<crate::style::antislop::AntiSlopReport>,
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

/// A scene stored on an `alternative`-type branch that is NOT the active
/// branch, reported beside (never inside) the spine listing. These are the
/// rows `generate_alternatives` produced; until one is promoted via
/// `select_alternative` it is not part of the active-branch spine and is
/// never rendered by `compile_manuscript` or addressable by position.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UnresolvedAlternativeEntry {
    pub scene_id: String,
    pub branch_id: String,
    pub branch_name: String,
    pub scene_order: i32,
    pub word_count: usize,
    pub summary_first_line: String,
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
    /// The active-branch spine of this chapter, exactly what
    /// `compile_manuscript` renders and the position resolver addresses.
    /// Never contains rows from other branches and never contains duplicate
    /// `scene_order` values.
    #[serde(default)]
    pub scenes: Vec<SceneSpineEntry>,
    /// Alternative-branch scenes in this chapter that were never promoted to
    /// the active branch. Deliberately separate from `scenes` so they can
    /// never be mistaken for the spine or deleted by position.
    #[serde(default)]
    pub unresolved_alternatives: Vec<UnresolvedAlternativeEntry>,
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

/// Where a character-state snapshot is anchored when it is not tied to a
/// saved scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CharacterStateAnchor {
    /// The state is true before the first scene: backstory, a death before
    /// page one, or another story-start condition. Stored at placement
    /// `(book 1, chapter 1, scene_order 0)` with no scene id.
    #[serde(alias = "backstory", alias = "before_first_scene")]
    StoryStart,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitCharacterStateInput {
    /// Record id returned by create_character (e.g. "character:abc123")
    pub character_id: String,
    /// Record id returned by save_scene_draft (e.g. "scene:abc123").
    /// Omit when `anchor` is supplied.
    #[serde(default)]
    pub scene_id: Option<String>,
    /// Story-start/backstory anchor for state that predates the first scene.
    /// Exactly one of `scene_id` or `anchor` must be supplied.
    #[serde(default)]
    pub anchor: Option<CharacterStateAnchor>,
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

// One on-page knowledge acquisition flagged in a scene's continuity package
// (design §2.3 path 2, the draft-time reveal). Each entry becomes a
// `knowledge_fact` row for `character_id`, learned_at = the saving scene's
// placement. When `secret_of_fact_id` names a secret canonical fact, the row
// links to it and expands that secret's circle of trust from this scene
// forward — the reveal mechanism. `None` there records ordinary knowledge.
/// On-page character knowledge acquisition recorded with a scene.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeLearnedEntry {
    /// Character who learns the fact.
    pub character_id: String,
    /// Short natural-language statement of the learned fact.
    pub fact: String,
    /// Optional note on how or where the fact was learned.
    #[serde(default)]
    pub source_summary: Option<String>,
    // When set, links this knowledge to a secret canonical fact (its circle of
    // trust expands to include `character_id`). The id must reference an
    // existing canonical fact marked `secret = 1`, or the save is rejected.
    /// Optional secret canonical fact ID revealed to this character.
    #[serde(default)]
    pub secret_of_fact_id: Option<String>,
    /// Whether the reader knows this fact (defaults to true).
    #[serde(default)]
    pub reader_visible: Option<bool>,
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
    /// Intra-scene temporal-coherence advisories (deterministic prose scan).
    /// Always advisory: every entry is `severity: "warning"`, kept in its own
    /// field (never in `blocking_continuity_findings`), so a temporal finding
    /// cannot block a commit under any `continuity_gate`.
    #[serde(default)]
    pub temporal_findings: Vec<ConsistencyIssue>,
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
    /// The chapter's effective title after the call: the supplied title when
    /// it was applied, otherwise the title the chapter already had (the
    /// ensure path can return a pre-existing row). `None` when untitled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    /// Non-blocking advisories, e.g. a chapter word-count target in the new
    /// narrator voice that contradicts reader_contract.style_notes or a style
    /// world rule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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

/// How far a recorded change can be backed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UndoSupport {
    /// A compensating operation can restore the prior state.
    Reversible,
    /// Restore the whole branch checkpoint explicitly; not a single-record undo.
    CheckpointOnly,
    /// No safe automatic compensation; manual recovery is required.
    Irreversible,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListChangesInput {
    pub project_id: String,
    /// Branch to inspect; defaults to the active branch.
    #[serde(default)]
    pub branch_id: Option<String>,
    /// Restrict to changes touching this scene.
    #[serde(default)]
    pub scene_id: Option<String>,
    /// `scene_save`, `branch_checkpoint`, `style_apply`, `style_patch`, or `undo`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Page size, 1-100; defaults to 20.
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangeReceipt {
    /// Prefix-qualified operation id accepted by `undo_change`.
    pub operation_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub kind: String,
    pub subject_id: String,
    pub summary: String,
    pub created_at: String,
    pub undo_support: UndoSupport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_reason: Option<String>,
    #[serde(default)]
    pub undone: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListChangesOutput {
    pub changes: Vec<ChangeReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UndoChangeInput {
    pub project_id: String,
    /// Operation id from `list_changes` (for example `scene_version:...`).
    pub operation_id: String,
    /// Current head the caller observed; guards against intervening edits.
    pub expected_head: String,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UndoAffected {
    pub table: String,
    pub id: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UndoChangeOutput {
    pub operation_id: String,
    pub kind: String,
    pub project_id: String,
    pub branch_id: String,
    pub affected: Vec<UndoAffected>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compensating_operation_id: Option<String>,
    pub dry_run: bool,
    pub message: String,
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
    /// Intra-scene temporal-coherence advisories (deterministic prose scan),
    /// recomputed on the revised prose. Advisory-only — every entry is
    /// `severity: "warning"` and never blocks a revision.
    #[serde(default)]
    pub temporal_findings: Vec<ConsistencyIssue>,
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
    /// Validate and report the batch without persisting.
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateTermsOutput {
    #[serde(default)]
    pub term_ids: Vec<String>,
    pub created: usize,
    /// Non-fatal post-commit derived-work warnings (for example a search
    /// index refresh that must be retried). The rows are committed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateEntityInput {
    pub entity_type: String,
    pub entity_id: String,
    #[schemars(schema_with = "any_object_schema")]
    pub changes: serde_json::Value,
    /// Renaming a character is a controlled operation, not a plain field
    /// write: it must move the normalized-name uniqueness key, preserve the
    /// old name as an alias, refresh the search index, and report records
    /// still referencing the old name. Set this to true to opt in when
    /// `changes` includes `"name"` on a character; without it the call is
    /// rejected. Ignored for every other entity type.
    #[serde(default)]
    pub allow_rename: Option<bool>,
}

fn any_object_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    serde_json::from_value(serde_json::json!({
        "type": "object",
        "description": "Key-value map of fields to update on the entity"
    }))
    .expect("valid schema")
}

/// Typed replacement for the open `update_entity` path on characters.
/// Only the fields the row allowlist supports are exposed; omitted fields are
/// unchanged, and a `name` change still requires `allow_rename` so the
/// uniqueness key, aliases, search index, and stale-reference report move
/// together.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCharacterInput {
    /// Record id of the character (for example "character:abc123").
    pub character_id: String,
    /// New primary name; requires `allow_rename`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<String>,
    /// Alternate names; replaces the stored list when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    /// Opt in to a controlled character rename when `name` is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_rename: Option<bool>,
}

impl UpdateCharacterInput {
    pub fn into_update_entity(self) -> UpdateEntityInput {
        let mut changes = serde_json::Map::new();
        if let Some(value) = self.name {
            changes.insert("name".to_string(), value.into());
        }
        if let Some(value) = self.summary {
            changes.insert("summary".to_string(), value.into());
        }
        if let Some(value) = self.role {
            changes.insert("role".to_string(), value.into());
        }
        if let Some(value) = self.realm {
            changes.insert("realm".to_string(), value.into());
        }
        if let Some(value) = self.appearance {
            changes.insert("appearance".to_string(), value.into());
        }
        if let Some(value) = self.aliases {
            changes.insert(
                "aliases".to_string(),
                serde_json::to_value(value).expect("aliases serialize"),
            );
        }
        UpdateEntityInput {
            entity_type: "character".to_string(),
            entity_id: self.character_id,
            changes: serde_json::Value::Object(changes),
            allow_rename: self.allow_rename,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateLocationInput {
    /// Record id of the location (for example "location:abc123").
    pub location_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl UpdateLocationInput {
    pub fn into_update_entity(self) -> UpdateEntityInput {
        let mut changes = serde_json::Map::new();
        if let Some(value) = self.summary {
            changes.insert("summary".to_string(), value.into());
        }
        if let Some(value) = self.realm {
            changes.insert("realm".to_string(), value.into());
        }
        if let Some(value) = self.kind {
            changes.insert("kind".to_string(), value.into());
        }
        UpdateEntityInput {
            entity_type: "location".to_string(),
            entity_id: self.location_id,
            changes: serde_json::Value::Object(changes),
            allow_rename: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateConflictInput {
    /// Record id of the conflict (for example "conflict:abc123").
    pub conflict_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stakes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_total_cycles: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub try_fail_cycles: Option<Vec<TryFailCycleStep>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation_stages: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stated_consequences: Option<Vec<StatedConsequence>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Index-aligned with `escalation_stages`; each entry is the placement
    /// where that stage was demonstrated (or null while pending).
    pub escalation_demonstrated: Option<Vec<Option<StoryPlacement>>>,
}

impl UpdateConflictInput {
    pub fn into_update_entity(self) -> UpdateEntityInput {
        let mut changes = serde_json::Map::new();
        if let Some(value) = self.stakes {
            changes.insert("stakes".to_string(), value.into());
        }
        if let Some(value) = self.resolution_summary {
            changes.insert("resolution_summary".to_string(), value.into());
        }
        if let Some(value) = self.conflict_type {
            changes.insert("conflict_type".to_string(), value.into());
        }
        if let Some(value) = self.expected_total_cycles {
            changes.insert("expected_total_cycles".to_string(), value.into());
        }
        for (key, value) in [
            (
                "try_fail_cycles",
                self.try_fail_cycles
                    .map(|v| serde_json::to_value(v).expect("cycles serialize")),
            ),
            (
                "escalation_stages",
                self.escalation_stages
                    .map(|v| serde_json::to_value(v).expect("stages serialize")),
            ),
            (
                "stated_consequences",
                self.stated_consequences
                    .map(|v| serde_json::to_value(v).expect("consequences serialize")),
            ),
            (
                "escalation_demonstrated",
                self.escalation_demonstrated
                    .map(|v| serde_json::to_value(v).expect("demonstrated serialize")),
            ),
        ] {
            if let Some(value) = value {
                changes.insert(key.to_string(), value);
            }
        }
        UpdateEntityInput {
            entity_type: "conflict".to_string(),
            entity_id: self.conflict_id,
            changes: serde_json::Value::Object(changes),
            allow_rename: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCharacterArcInput {
    /// Record id of the character arc (for example "character_arc:abc123").
    pub character_arc_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arc_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ending_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thematic_purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_theme_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Replaces the stored milestone list when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestones: Option<Vec<CharacterArcMilestone>>,
}

impl UpdateCharacterArcInput {
    pub fn into_update_entity(self) -> UpdateEntityInput {
        let mut changes = serde_json::Map::new();
        for (key, value) in [
            ("arc_type", self.arc_type.map(|v| v.into())),
            ("starting_state", self.starting_state.map(|v| v.into())),
            ("ending_state", self.ending_state.map(|v| v.into())),
            ("thematic_purpose", self.thematic_purpose.map(|v| v.into())),
            ("status", self.status.map(|v| v.into())),
            (
                "connected_theme_ids",
                self.connected_theme_ids
                    .map(|v| serde_json::to_value(v).expect("themes serialize")),
            ),
            (
                "milestones",
                self.milestones
                    .map(|v| serde_json::to_value(v).expect("milestones serialize")),
            ),
        ] {
            if let Some(value) = value {
                changes.insert(key.to_string(), value);
            }
        }
        UpdateEntityInput {
            entity_type: "character_arc".to_string(),
            entity_id: self.character_arc_id,
            changes: serde_json::Value::Object(changes),
            allow_rename: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateEntityOutput {
    pub entity_type: String,
    pub entity_id: String,
    /// Present only when the call renamed a character (`changes` included
    /// `"name"` with `allow_rename: true`). Lists the records that still
    /// reference the old name so the author can fix them — prose mentions in
    /// scenes are never rewritten automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rename_report: Option<RenameReport>,
    /// Non-blocking advisories about the post-update state. Currently used to
    /// surface reader-contract / style-surface contradictions (e.g. chapter
    /// word-count targets that disagree between reader_contract.style_notes,
    /// the narrator voice, and style world rules) so a stale figure is caught
    /// at update time instead of after chapters are drafted against it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Outcome of a character rename: what changed and what still references the
/// old name. The search index (FTS + embeddings) and the character's own FTS
/// entry are refreshed as part of the rename; the records listed here are the
/// ones the rename could NOT update safely.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenameReport {
    pub character_id: String,
    pub old_name: String,
    pub new_name: String,
    /// True when the old name was appended to the character's aliases (it is
    /// not re-added when already present, or when only the casing/spacing of
    /// the name changed).
    pub old_name_kept_as_alias: bool,
    /// Scenes whose prose or summary mentions the old name (FTS match on the
    /// active branch). Rename never edits scene text — the author decides
    /// which mentions are historical and which must follow the rename.
    pub scenes_referencing_old_name: Vec<String>,
    /// Active (not superseded) canonical facts whose value text, JSON value,
    /// or aliases mention the old name. Facts that reference the character by
    /// id are unaffected and not listed.
    pub canonical_facts_referencing_old_name: Vec<String>,
    /// Knowledge facts whose text mentions the old name.
    pub knowledge_facts_referencing_old_name: Vec<String>,
    /// Character arcs (of this character) whose notes or milestones mention
    /// the old name.
    pub arcs_referencing_old_name: Vec<String>,
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
    /// Non-blocking advisories, e.g. a style world rule whose chapter
    /// word-count target contradicts reader_contract.style_notes or the
    /// narrator voice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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
    /// Validate and report the batch without persisting.
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchSetCharacterVoiceProfilesOutput {
    #[serde(default)]
    pub profiles: Vec<SetCharacterVoiceProfileOutput>,
    pub updated: usize,
    /// Non-fatal post-commit derived-work warnings (for example a search
    /// index refresh that must be retried). The rows are committed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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
    #[serde(default)]
    pub connected_conflict_ids: Vec<String>,
    #[serde(default)]
    pub connected_theme_ids: Vec<String>,
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
    /// Validate and report the batch without persisting.
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateMotifsOutput {
    #[serde(default)]
    pub motif_ids: Vec<String>,
    pub created: usize,
    /// Non-fatal post-commit derived-work warnings (for example a search
    /// index refresh that must be retried). The rows are committed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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
    /// Validate and report the batch without persisting.
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchCreateNarrativePromisesOutput {
    #[serde(default)]
    pub narrative_promise_ids: Vec<String>,
    pub created: usize,
    /// Non-fatal post-commit derived-work warnings (for example a search
    /// index refresh that must be retried). The rows are committed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Canonical batch creation for glossary terms. Items are validated together
/// and committed in one branch transaction; `dry_run` validates without writes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTermsInput {
    pub project_id: String,
    /// 1-50 terms to create atomically.
    pub items: Vec<BatchCreateTermItem>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateTermsOutput {
    #[serde(default)]
    pub term_ids: Vec<String>,
    pub created: usize,
    #[serde(default)]
    pub dry_run: bool,
    /// Non-fatal post-commit derived-work warnings (for example a search
    /// index refresh that must be retried). The rows are committed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateMotifsInput {
    pub project_id: String,
    /// 1-50 motifs to create atomically.
    pub items: Vec<BatchCreateMotifItem>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateMotifsOutput {
    #[serde(default)]
    pub motif_ids: Vec<String>,
    pub created: usize,
    #[serde(default)]
    pub dry_run: bool,
    /// Non-fatal post-commit derived-work warnings (for example a search
    /// index refresh that must be retried). The rows are committed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateNarrativePromisesInput {
    pub project_id: String,
    /// 1-50 promises to create atomically.
    pub items: Vec<BatchCreateNarrativePromiseItem>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateNarrativePromisesOutput {
    #[serde(default)]
    pub narrative_promise_ids: Vec<String>,
    pub created: usize,
    #[serde(default)]
    pub dry_run: bool,
    /// Non-fatal post-commit derived-work warnings (for example a search
    /// index refresh that must be retried). The rows are committed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCharacterVoiceProfilesInput {
    pub project_id: String,
    pub branch_id: String,
    /// 1-50 character voice profiles to set atomically.
    pub items: Vec<BatchSetCharacterVoiceProfileItem>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetCharacterVoiceProfilesOutput {
    #[serde(default)]
    pub profiles: Vec<SetCharacterVoiceProfileOutput>,
    pub updated: usize,
    #[serde(default)]
    pub dry_run: bool,
    /// Non-fatal post-commit derived-work warnings (for example a search
    /// index refresh that must be retried). The rows are committed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// An actual story event, separate from an author's planned payoff.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PromiseStatusEvent {
    pub id: String,
    pub previous_status: String,
    pub status: String,
    pub at: Option<StoryPlacement>,
    pub source_scene_id: Option<String>,
    pub note: Option<String>,
    pub replaces_event_id: Option<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct UpdatePromiseStatusInput {
    #[serde(alias = "promise_id")]
    pub narrative_promise_id: String,
    #[serde(alias = "new_status")]
    pub status: String,
    pub note: Option<String>,
    /// Actual placement. Omit when unknown; planned_payoff is never used here.
    pub at: Option<StoryPlacement>,
    /// Evidence scene; supplies the placement when `at` is omitted.
    pub source_scene_id: Option<String>,
    /// Correct an existing event without deleting its audit history.
    pub replaces_event_id: Option<String>,
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
pub struct UpdateArcMilestoneInput {
    /// Record id of the character arc (e.g. "character_arc:abc123").
    pub arc_id: String,
    /// Exact label of the milestone to update.
    pub label: String,
    /// New planned placement for the milestone. Omit to keep the current
    /// placement. (Clearing a placement is not supported here; resend the
    /// full milestones array via update_entity for that.)
    #[serde(default)]
    pub placement: Option<StoryPlacement>,
    /// Manuscript position where the milestone was actually reached. Omit to
    /// keep the current value.
    #[serde(default)]
    pub reached_at: Option<StoryPlacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateArcMilestoneOutput {
    pub arc_id: String,
    /// The milestone after the update.
    pub milestone: CharacterArcMilestone,
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
    #[serde(default)]
    pub intensity_points: Vec<IntensityPoint>,
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
    /// A single scene inside one chapter (P2.1 scene-scoped verification).
    /// Reachable only when the flat input pins a single chapter AND carries a
    /// `scene_order`; the scoped scene set collapses to exactly this scene.
    Scene {
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
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
    /// Optional single-scene narrowing (P2.1). Meaningful only when the scope
    /// already pins one chapter — a `chapter_range` whose start book+chapter
    /// equal its end book+chapter. Any other shape (missing bounds, or
    /// `start != end`) with a `scene_order` present is an input validation
    /// error. Absent (the default) it deserializes as `None`, so scope
    /// payloads authored before this field existed are unchanged.
    #[serde(default)]
    pub scene_order: Option<i32>,
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
            scene_order: None,
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
            scene_order: None,
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
            scene_order: None,
        }
    }

    pub fn to_scope(&self) -> Result<ConsistencyScope, String> {
        match self.scope_type.as_str() {
            "full" => {
                self.reject_scene_order("full")?;
                Ok(ConsistencyScope::Full)
            }
            "book" => {
                self.reject_scene_order("book")?;
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
                // Scene narrowing is meaningful only over a single pinned
                // chapter: start book+chapter must equal end book+chapter.
                if let Some(scene_order) = self.scene_order {
                    if (start_book_number, start_chapter_number)
                        != (end_book_number, end_chapter_number)
                    {
                        return Err("scene_order narrowing requires a single-chapter scope: \
                             start_book_number/start_chapter_number must equal \
                             end_book_number/end_chapter_number"
                            .to_string());
                    }
                    return Ok(ConsistencyScope::Scene {
                        book_number: start_book_number,
                        chapter_number: start_chapter_number,
                        scene_order,
                    });
                }
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

    /// A `scene_order` is only ever meaningful with a single-chapter
    /// `chapter_range`; pairing it with `full`/`book` is a caller mistake.
    fn reject_scene_order(&self, scope_type: &str) -> Result<(), String> {
        if self.scene_order.is_some() {
            return Err(format!(
                "scene_order narrowing requires a single-chapter 'chapter_range' scope, \
                 not scope_type '{scope_type}'"
            ));
        }
        Ok(())
    }
}

/// The deterministic, fast, scene-relevant check subset that in-run
/// scene-scoped verification runs immediately after a draft is saved
/// (evolution design §3.2). Every entry is a check that (a) needs no model
/// router when `deep_check = false`, and (b) attributes findings to a scene or
/// runs cleanly over a single-chapter window. Deep-only and cross-chapter-trend
/// checks are deliberately excluded — they stay checkpoint-scoped for cost.
///
/// The trailing four names are Phase-4 validator **request** names (identical to
/// their finding `check_type` strings): they trigger the deterministic
/// `run_phase_four_validator_checks_for_scenes` fan-out under the non-deep path.
pub const SCENE_VERIFY_CHECKS: &[&str] = &[
    // ── Deterministic, scene-attributed continuity checks ──
    "retcon_reachability",  // shared as-of character-state / retcon scanner
    "knowledge_timing",     // premature-knowledge scan, per scene
    "chronology",           // story-time ordering, over the scoped window
    "temporal_coherence",   // intra-scene time-jump lexical scan (Tier 1)
    "quantity_drift",       // unexplained multi-band jumps (branch-wide feed)
    "currency_consistency", // price facts disagreeing once converted
    "affordability",        // priced purchase vs a present character's wealth
    "tone_consistency",     // scene tone vs declared reader-contract boundary
    "content_boundary_compliance", // explicit-scene review flag, per scene
    "secret_leak",          // out-of-circle speaker leaks a secret (Tier 1)
    "canonical_fact_consistency", // conflicting active canonical-fact values
    // ── Phase-4 validator request names (deterministic, prose-drift) ──
    "canonical_fact_prose_drift", // prose contradicts a registered canonical fact
    "world_rule_semantic_drift",  // prose trips a world-rule scan pattern
    "voice_drift",                // dialogue drifts from a character's voice profile
    "style_compliance",           // prose drifts from an applied style profile
    "anti_slop",                  // fiction hard shelves over limit (Phase 3 verify)
                                  // ── Deliberately EXCLUDED (deep-only or cross-chapter trend) ──
                                  // "promise_payoff_detection"   — deep-only (model tier)
                                  // "scene_purpose_fulfillment"  — deep-only (model tier)
                                  // "pacing_drift"             — multi-chapter intensity trend
                                  // "research_accuracy"        — corpus-wide provenance audit, not scene-local
                                  // "scene_divergence"         — source-vs-draft, separate bridge
];

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckConsistencyInput {
    /// Page through capped deep audits in stable story/urgency order. Keep the
    /// scope and manuscript unchanged between pages; defaults to zero.
    #[serde(default)]
    pub deep_scan_offset: Option<usize>,
    pub project_id: String,
    pub scope: ConsistencyScopeInput,
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub severity_filter: Vec<String>,
    #[serde(default)]
    pub deep_check: Option<bool>,
    /// Additive semantic lane mode. Omitted means `off`; P2 builds candidate
    /// requests/coverage only and never emits model findings.
    #[serde(default)]
    pub semantic_mode: Option<crate::semantic_continuity::SemanticMode>,
    /// Optional host reports answering `prepare_semantic_continuity` requests.
    /// Reports are validated and cached; no model call is made for them.
    #[serde(default)]
    pub semantic_host_reports: Vec<crate::semantic_continuity::SemanticHostReportV1>,
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
    /// Exact scene evidence when a finding was produced by the semantic lane
    /// or another evidence-anchored detector. Omitted for legacy findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<crate::semantic_continuity::SemanticEvidenceSpanV1>,
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
    /// Empty for shallow checks. A clean finding list does not imply complete coverage.
    #[serde(default)]
    pub audit_coverage: Vec<AuditCoverage>,
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
    /// Shadow/on semantic counters. Present only when the lane was invoked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_summary: Option<crate::semantic_continuity::SemanticRunSummaryV1>,
    /// Per-scene semantic counters for evaluator percentiles and coverage.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_scene_summaries: Vec<crate::semantic_continuity::SemanticSceneSummaryV1>,
}

/// Read-only request for the deterministic semantic candidate sets of every
/// scene in scope. Makes no model call and changes no state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PrepareSemanticContinuityInput {
    /// Record id returned by `create_project`.
    pub project_id: String,
    /// Story scope to prepare candidate sets for.
    pub scope: ConsistencyScopeInput,
    /// Optional subject narrowing, matching `check_consistency`.
    #[serde(default)]
    pub subjects: Vec<String>,
}

/// Candidate coverage summary for one prepare call. Empty candidate sets are
/// honest coverage, not a clean semantic review.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SemanticCandidateAudit {
    /// Scenes considered in scope.
    pub scene_count: usize,
    /// Scenes with at least one candidate.
    pub candidate_scene_count: usize,
    /// Total candidates after ranking and caps.
    pub candidate_count: usize,
    /// Counts of family candidates omitted by a cap.
    #[serde(default)]
    pub omitted_by_family: std::collections::BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PrepareSemanticContinuityOutput {
    /// One read-only request per scene in scope.
    pub requests: Vec<crate::semantic_continuity::SemanticHostRequestV1>,
    /// Candidate coverage summary.
    pub coverage: SemanticCandidateAudit,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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
    /// Secret-knowledge gating (design §2.3.1, the reveal path): when set, this
    /// knowledge row grants the character membership in the circle of trust for
    /// the referenced secret canonical fact. The id must reference an existing
    /// canonical fact with `secret = 1`. `None` for ordinary knowledge.
    #[serde(default)]
    pub secret_of_fact_id: Option<String>,
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
pub struct SkillSummary {
    pub name: String,
    /// "skill" or "reference".
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListSkillsOutput {
    pub skills: Vec<SkillSummary>,
    pub references: Vec<SkillSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UseProfileInput {
    /// Generated profile name to activate for this MCP session (for example "workflow-write", "research", or "full").
    pub profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UseProfileOutput {
    /// Active profile name after the switch.
    pub profile: String,
    /// Number of tools now visible in this session.
    pub tool_count: usize,
    /// Tool names now visible in this session, in discovery order.
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSkillInput {
    /// Skill name as returned by `list_skills` (e.g. "scene-writer").
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSkillOutput {
    /// Skill name as returned by `list_skills`.
    pub name: String,
    /// Actionable workflow instructions when covered, or a profile-selection message when uncovered.
    pub markdown: String,
    /// Whether the active tool profile covers all tools required by this skill.
    #[serde(default = "default_true")]
    pub covered: bool,
    /// Smallest tool profile that covers this skill.
    #[serde(default)]
    pub required_profile: Option<String>,
    /// Skill and reference units covered by the active session profile.
    #[serde(default)]
    pub available_units: Vec<String>,
    /// Required tool capabilities missing from the active session profile.
    #[serde(default)]
    pub missing_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetReferenceInput {
    /// Craft reference name as returned by `list_skills` (e.g. "anti-slop").
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetReferenceOutput {
    /// Craft reference or response contract name.
    pub name: String,
    /// Reference markdown when covered, or a profile-selection message when uncovered.
    pub markdown: String,
    /// Whether the active tool profile covers all tools required by this reference.
    #[serde(default = "default_true")]
    pub covered: bool,
    /// Smallest tool profile that covers this reference.
    #[serde(default)]
    pub required_profile: Option<String>,
    /// Skill and reference units covered by the active session profile.
    #[serde(default)]
    pub available_units: Vec<String>,
    /// Required tool capabilities missing from the active session profile.
    #[serde(default)]
    pub missing_capabilities: Vec<String>,
}

/// Optional index of full response schemas, independent of profile access.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResponseContractIndex {
    /// Major contract version. Version 1 describes the current result envelopes.
    pub version: u32,
    /// Full-schema loading routes for explicitly selected large result types.
    pub contracts: Vec<ResponseContractEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResponseContractEntry {
    /// Existing registered tool whose structured response the schema describes.
    pub tool_name: String,
    /// Resource URI returning the full JSON Schema without project data.
    pub uri: String,
    /// Equivalent get_reference name for hosts without MCP resource support.
    pub reference_name: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_error: Option<String>,
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
    /// Effective reasoning effort for the route: the routing rule's override
    /// when set, otherwise the agent-level default. `None` means no
    /// `reasoning_effort` field is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_packet_hash: Option<String>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
    /// run the drafting model in their own MCP session so the spawned agent
    /// can call `set_active_project` before pulling canon — its session does
    /// not inherit the caller's active project.
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
    /// Optional authoring run id for telemetry attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Optional prompt template identifier override for telemetry attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_template_id: Option<String>,
    /// Optional prompt template version override for telemetry attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_version: Option<String>,
    /// Optional SHA-256 hex digest of the assembled context packet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_packet_hash: Option<String>,
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

/// Input for `compile_manuscript`: assemble the committed prose of a book (or a
/// chapter range within it) on the active branch into a single Markdown
/// document. Read-only over scene/plan data; the workspace write is opt-in.
/// Branch resolution is implicit — the service compiles the project's active
/// branch, matching the other read tools (`preflight_book_export`,
/// `list_book_chapters`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompileManuscriptInput {
    pub project_id: String,
    pub book_number: i32,
    /// Optional inclusive start chapter. When omitted, compilation begins at the
    /// book's first chapter.
    #[serde(default)]
    pub start_chapter: Option<i32>,
    /// Optional inclusive end chapter. When omitted, compilation runs to the
    /// book's last chapter.
    #[serde(default)]
    pub end_chapter: Option<i32>,
    /// When true, the compiled Markdown is also written to the project's
    /// workspace artifacts directory under a deterministic filename, and the
    /// path is returned in `artifact_path`.
    #[serde(default)]
    pub write_to_workspace: bool,
}

/// Output of `compile_manuscript`: the assembled Markdown plus counts and the
/// list of planned-but-undrafted scenes rendered as placeholders. A zero-chapter
/// range yields a structured empty result, not an error.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompileManuscriptOutput {
    pub markdown: String,
    pub scene_count: usize,
    pub word_count: usize,
    /// `chapter.scene` refs (e.g. "2.2") for scenes present in a chapter plan
    /// spine but not yet drafted. Never silently omitted from the Markdown.
    #[serde(default)]
    pub missing_scenes: Vec<String>,
    /// `chapter.scene` refs (e.g. "2.2") for drafted scenes that are stubs:
    /// their body matches obvious placeholder text, or their word count is
    /// below the project's minimum scene length
    /// (`project.min_scene_word_count`, default 20). They render into the
    /// Markdown but are not shippable prose.
    #[serde(default)]
    pub stub_scenes: Vec<String>,
    /// Absolute path to the written artifact when `write_to_workspace` is set;
    /// `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
}

// Input for `export_recap` (evolution §3.8): a spoiler-bounded, reader-facing
// "previously on" recap of a book up to (and including) `through_chapter`. Pure
// read model over chapter summaries and narrative promises on the active
// branch; the cursor is the end of `through_chapter` in `book_number`, and the
// reader-secret rule (see the resolver's doc comment) governs what a fact/thread
// may name. The workspace write is opt-in and mirrors `compile_manuscript`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportRecapInput {
    /// Project identifier.
    pub project_id: String,
    /// Book number to summarize (1-based).
    pub book_number: i32,
    /// Inclusive last chapter covered by the recap; later chapters are withheld.
    pub through_chapter: i32,
    /// Also write the recap Markdown to the workspace artifacts directory.
    #[serde(default)]
    pub write_to_workspace: bool,
}

/// Output of `export_recap`: the recap Markdown plus counts and the opt-in
/// artifact path. A book with no summarized chapters at/under the cursor yields
/// a structured empty recap, not an error.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportRecapOutput {
    /// Rendered recap Markdown.
    pub markdown: String,
    /// Number of chapter summaries at or under the cursor folded into the recap.
    pub chapter_count: usize,
    /// Word count of the rendered recap Markdown.
    pub word_count: usize,
    /// Absolute path to the written artifact when `write_to_workspace` is set;
    /// `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
}

// Input for `export_series_bible` (evolution §3.8): a spoiler-bounded,
// reader-facing series bible (characters, locations, glossary, factions,
// religions) as of an optional cursor. Pure read model over branch entities and
// placement-stamped character state; the reader-secret rule governs what may be
// named. The workspace write is opt-in and mirrors `compile_manuscript`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportSeriesBibleInput {
    /// Project identifier.
    pub project_id: String,
    /// Optional spoiler cursor; state after this placement is withheld.
    #[serde(default)]
    pub through: Option<StoryPlacement>,
    /// Also write the series bible Markdown to the workspace artifacts directory.
    #[serde(default)]
    pub write_to_workspace: bool,
}

/// Output of `export_series_bible`: the bible Markdown plus counts and the opt-in
/// artifact path. An empty project yields a structured empty bible, not an error.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportSeriesBibleOutput {
    pub markdown: String,
    /// Number of characters rendered (those established at or under the cursor).
    pub chapter_count: usize,
    /// Word count of the rendered bible Markdown.
    pub word_count: usize,
    /// Absolute path to the written artifact when `write_to_workspace` is set;
    /// `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
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

/// Secret-knowledge gating (design §2.1): the audience boundary for a fact
/// registered in confidence. `holder_ids` is the initial circle of trust —
/// the characters who know the fact at declaration time. `concealment_note` is
/// optional drafting guidance rendered into the `[SECRETS IN PLAY]` envelope
/// (Part B), e.g. "she deflects questions about her past with dry humor".
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SecrecyScope {
    /// Characters who know the fact at declaration time (the initial circle).
    pub holder_ids: Vec<String>,
    /// Optional guidance rendered into the envelope when the fact ships to the
    /// model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concealment_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterCanonicalFactInput {
    pub project_id: String,
    /// The scene that dramatises the fact. Optional: omit it to register a
    /// fact decided during PLANNING but not yet dramatised (e.g. a name
    /// locked by author decision before its scene exists). Such a fact is
    /// planned-and-pending — placed by book_number/chapter_number only — and
    /// can be attached to its scene later with bind_canonical_fact_to_scene.
    #[serde(default)]
    pub scene_id: Option<String>,
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
    /// Secret-knowledge gating (design §2.1): when present, the fact is marked
    /// `secret = 1`, its `concealment_note` is stored, and one knowledge_fact
    /// row is written per `holder_id` linking that character into the fact's
    /// circle of trust with `learned_at = None` (known from the start).
    #[serde(default)]
    pub secrecy: Option<SecrecyScope>,
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
pub struct BindCanonicalFactToSceneInput {
    /// Record id of the canonical fact (e.g. "canonical_fact:abc123").
    pub canonical_fact_id: String,
    /// The scene that dramatises the fact. Must belong to the same project
    /// and branch as the fact.
    pub scene_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BindCanonicalFactToSceneOutput {
    pub canonical_fact_id: String,
    pub scene_id: String,
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
    /// Project identifier.
    pub project_id: String,
    /// Book number (1-based).
    pub book_number: i32,
    /// Inclusive starting chapter number (1-based).
    pub start_chapter: i32,
    /// Optional inclusive ending chapter number.
    #[serde(default)]
    pub end_chapter: Option<i32>,
    /// Optional number of chapters to include starting at `start_chapter`.
    #[serde(default)]
    pub chapter_count: Option<i32>,
    /// Number of chapters per checkpoint review window (minimum 1).
    pub checkpoint_interval: usize,
    /// Optional editorial directives carried into the run.
    #[serde(default)]
    pub editorial_directives: Option<Vec<String>>,
    /// Execution mode (`hybrid` for host drafting or `agent` for routed offload).
    #[serde(default)]
    pub mode: Option<AuthoringMode>,
    /// Canon-mining policy (`disabled` or `propose_all` to stage canon deltas per scene).
    // Omitted, `None`, or `"disabled"` = disabled (persists NULL, behaves
    // exactly as before; evolution §3.1).
    #[serde(default)]
    pub mining_policy: Option<AuthoringMiningPolicy>,
    /// Bounded verify/revise attempts per scene (`0..=2`; `0` or omitted disables the loop).
    // Bounded in-run verify/revise attempts per scene (evolution §3.2). Omitted,
    // `None`, or `0` = disabled: a saved draft goes straight to commit exactly
    // as before. `1` or `2` opts the run into scene-scoped verification after
    // each draft, feeding any warning-or-worse findings back as a bounded
    // revision. Validated `0..=2`; anything else is an input error.
    #[serde(default)]
    pub max_revise_attempts: Option<i32>,
    /// Checkpoint review policy (`manual`, `auto_advisory`, or `auto_strict`).
    // Omitted, `None`, or `"manual"` = the classic 4-step operator checkpoint
    // flow, byte-identical to today (evolution §3.3, I1). `"auto_advisory"`
    // self-clears when no finding is `warning`-or-worse; `"auto_strict"`
    // self-clears only on zero findings. Auto policies require review-route
    // coverage across the run's ratings at start.
    #[serde(default)]
    pub checkpoint_policy: Option<AuthoringCheckpointPolicy>,
    /// Living-outline replan policy (`disabled` or `propose_all` to stage amendments after chapter summaries).
    // Omitted, `None`, or `"disabled"` = the loop never replans (ADR 0003,
    // evolution §3.5). `"propose_all"` stages amendment proposals against
    // undrafted future chapters for operator ratification (never auto-applied).
    #[serde(default)]
    pub replan_policy: Option<AuthoringReplanPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStartRunOutput {
    pub run_id: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStatusInput {
    /// Project identifier.
    pub project_id: String,
    /// Optional authoring run ID (defaults to the project's latest run).
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
    /// Post-summary replan outcome (`staged`, `skipped`, `no_targets`, `no_summary`, or `error`).
    // Living-outline replan outcome for this chapter's post-summary pass (ADR
    // 0003, evolution §3.5). `None` = replan not attempted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replan_status: Option<String>,
    /// Human-readable replan outcome summary (counts or skip reason; never prose).
    // Never carries prose (evolution I8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replan_detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStatusScene {
    pub scene_order: i32,
    pub phase: String,
    pub scene_id: Option<String>,
    pub scene_artifact_path: Option<String>,
    pub blocked_reason: Option<String>,
    /// Canon-mining outcome (`staged`, `skipped`, `model_output_rejected`, or `error`).
    // Canon-mining outcome for this scene (evolution §3.1). `None` = mining
    // not attempted (disabled run or scene not yet past commit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine_status: Option<String>,
    /// Human-readable canon-mining summary (staged delta count or skip/error reason; never prose).
    // Never carries prose (evolution I8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine_detail: Option<String>,
    /// Scene verification outcome (`clean`, `findings`, `parked_findings`, or `error`).
    // In-run verification outcome for this scene (evolution §3.2). `None` =
    // verify not attempted (disabled run, or scene not yet past draft).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_status: Option<String>,
    /// Human-readable verification summary (finding counts, parked reason, or error; never prose).
    // Never carries prose (evolution I8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_detail: Option<String>,
    /// Number of bounded revision passes consumed by this scene.
    // Evolution §3.2; `0` for scenes that never entered the revise loop.
    #[serde(default)]
    pub revise_attempts: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringStatusCheckpoint {
    pub start_chapter: i32,
    pub end_chapter: i32,
    pub save_point_id: String,
    pub status: String,
    pub report_artifact_path: Option<String>,
    /// Checkpoint policy in force (`auto_advisory` or `auto_strict`; `None` when manual).
    // Evolution §3.3; additive, serde-default so pre-policy clients ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_policy: Option<AuthoringCheckpointPolicy>,
    /// Auto-checkpoint outcome (`approved`, `blocked`, or `manual`; `None` when manual or pending).
    // Outcome of the in-process auto-checkpoint automation (evolution §3.3).
    // Ids/enums only, never prose (I8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_outcome: Option<String>,
    /// Scene IDs whose sampled review fell back to manual due to uncleared review routing.
    // Evolution §3.3 explicit-manual-fallback (I3); ids only, never prose.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_manual_scene_ids: Vec<String>,
    /// Per-chapter cumulative reader-simulation engagement summaries for this checkpoint.
    // Evolution §3.6; read from the checkpoint report's `reader_sim` section.
    // Enums/ids only, never prose (I8).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reader_sim_engagement: Vec<ReaderSimEngagementSummary>,
    /// Sampled dual-persona review status (`complete`, `pending_manual_review`, or `pending_dual_persona_review`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampled_review_status: Option<String>,
    /// Number of sampled dual-persona review outputs persisted in the checkpoint report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampled_review_count: Option<usize>,
}

/// Compact per-chapter reader-simulation engagement summary (`high`, `steady`, `dipping`, `unparsed`, or `skipped`).
// Surfaced on `authoring_status` (evolution §3.6, R3). Enums/ids only — the
// reader's notes and concern text stay in the report artifact.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ReaderSimEngagementSummary {
    /// Chapter number (1-based).
    pub chapter: i32,
    /// Engagement level (`high`, `steady`, `dipping`, `unparsed`, or `skipped`).
    pub engagement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringExecuteNextInput {
    /// Project identifier.
    pub project_id: String,
    /// Optional authoring run ID (defaults to the project's latest run).
    #[serde(default)]
    pub run_id: Option<String>,
    /// Execution mode (`hybrid` for host drafting or `agent` for routed offload).
    #[serde(default)]
    pub mode: Option<AuthoringMode>,
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
    /// Project identifier.
    pub project_id: String,
    /// Optional authoring run ID (defaults to the project's latest run).
    #[serde(default)]
    pub run_id: Option<String>,
    /// Book number (1-based).
    #[serde(default)]
    pub book_number: i32,
    /// Chapter number (1-based).
    #[serde(default)]
    pub chapter_number: i32,
    /// Optional chapter record ID.
    #[serde(default)]
    pub chapter_id: Option<String>,
    /// Scene order within the chapter (1-based).
    pub scene_order: i32,
    /// Full scene prose text.
    #[serde(alias = "content", alias = "text")]
    pub full_text: String,
    /// Declare assistant for host-AI prose; omission preserves human authorship.
    #[serde(default)]
    pub authorship: DraftAuthorship,
    /// Concise scene summary.
    pub summary: String,
    /// Scene content rating.
    pub content_rating: ContentRating,
    /// Optional scene tone label.
    #[serde(default)]
    pub tone: Option<String>,
    /// Optional server-side generation receipt ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    /// Optional manuscript source path.
    #[serde(default)]
    pub source_path: Option<String>,
    /// Optional location record ID where the scene takes place.
    #[serde(default)]
    pub location_id: Option<String>,
    /// Research source IDs cited by this scene.
    #[serde(default)]
    pub research_source_ids: Vec<String>,
    /// Research note IDs cited by this scene.
    #[serde(default)]
    pub research_note_ids: Vec<String>,
    /// Research claim IDs cited by this scene.
    #[serde(default)]
    pub research_claim_ids: Vec<String>,
    /// Optional research query pack input used during drafting.
    #[serde(default)]
    pub research_query_pack_input: Option<String>,
    /// Optional research context hash.
    #[serde(default)]
    pub research_context_hash: Option<String>,
    /// Character state updates established in this scene.
    #[serde(default)]
    pub character_states: Vec<CharacterStatePatchEntry>,
    /// Canonical facts established in this scene.
    #[serde(default)]
    pub canonical_facts: Vec<CanonicalFactEntry>,
    /// Relationship updates established in this scene.
    #[serde(default)]
    pub relationship_updates: Vec<RelationshipUpdateEntry>,
    /// Outline beats realized in this scene.
    #[serde(default)]
    pub beats: Vec<AnnotatedBeatInput>,
    /// Continuity notes recorded with this scene.
    #[serde(default)]
    pub continuity_notes: Vec<String>,
    /// On-page character knowledge acquisitions recorded with this scene.
    #[serde(default)]
    pub knowledge_learned: Vec<KnowledgeLearnedEntry>,
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
    /// Project identifier.
    pub project_id: String,
    /// Optional authoring run ID (defaults to the project's latest run).
    #[serde(default)]
    pub run_id: Option<String>,
    /// Inclusive first chapter of the checkpoint window.
    pub start_chapter: i32,
    /// Inclusive last chapter of the checkpoint window.
    pub end_chapter: i32,
    /// Editorial directives recorded from this checkpoint review.
    pub directives: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthoringRecordCheckpointAuditInput {
    /// Project identifier.
    pub project_id: String,
    /// Optional authoring run ID (defaults to the project's latest run).
    #[serde(default)]
    pub run_id: Option<String>,
    /// Inclusive first chapter of the checkpoint window.
    pub start_chapter: i32,
    /// Inclusive last chapter of the checkpoint window.
    pub end_chapter: i32,
    /// Structured output returned by `check_consistency` with `deep_check=true` for this range.
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
    /// Project identifier.
    pub project_id: String,
    /// Optional authoring run ID (defaults to the project's latest run).
    #[serde(default)]
    pub run_id: Option<String>,
    /// Chapter number of the blocked scene (1-based).
    pub chapter_number: i32,
    /// Scene order of the blocked scene (1-based).
    pub scene_order: i32,
    /// Target scene phase (`pending`, `draft_saved`, `changes_committed`, `beats_annotated`, or `redraft`).
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
    /// Project identifier.
    pub project_id: String,
    /// Optional authoring run ID (defaults to the project's latest run).
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
    /// Project identifier.
    pub project_id: String,
    /// Book number (1-based).
    pub book_number: i32,
    /// Inclusive starting chapter number (1-based).
    pub start_chapter: i32,
    /// Optional inclusive ending chapter number.
    #[serde(default)]
    pub end_chapter: Option<i32>,
    /// Optional number of chapters to inspect starting at `start_chapter`.
    #[serde(default)]
    pub chapter_count: Option<i32>,
    /// Optional canon-mining policy (`disabled` or `propose_all` to preflight mine/review routes).
    // Prepare skips the extra mine-route preflight when `None` or `Disabled`;
    // `ProposeAll` verifies mine-or-review route coverage for every planned rating (evolution §3.1).
    #[serde(default)]
    pub mining_policy: Option<AuthoringMiningPolicy>,
    /// Optional bounded verify/revise attempts per scene (`0..=2`).
    // Threaded from `authoring_start_run` for validation; scene-scoped
    // verification is deterministic and revision reuses the `draft` route (evolution §3.2).
    #[serde(default)]
    pub max_revise_attempts: Option<i32>,
    /// Optional checkpoint policy (`manual`, `auto_advisory`, or `auto_strict` to preflight review routes).
    // Auto policies verify the `review` route resolves rating-cleared for every
    // planned rating (evolution §3.3).
    #[serde(default)]
    pub checkpoint_policy: Option<AuthoringCheckpointPolicy>,
    /// Optional living-outline replan policy (`disabled` or `propose_all`).
    // Threaded from `authoring_start_run` for validation; replanning reads
    // summaries and metadata only, so no extra rating preflight is added (ADR 0003, evolution §3.5).
    #[serde(default)]
    pub replan_policy: Option<AuthoringReplanPolicy>,
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

/// Normalize a freeform entity lookup for case- and diacritic-insensitive
/// comparison. Collapses whitespace but preserves punctuation inside a name.
pub fn normalize_entity_lookup(input: &str) -> String {
    let mut folded = String::new();
    for ch in input.chars().flat_map(char::to_lowercase) {
        // Combining marks left behind by lowercasing/decomposition are
        // dropped; the precomposed Latin characters are folded below.
        if matches!(
            ch,
            '\u{0300}'..='\u{036f}'
                | '\u{1ab0}'..='\u{1aff}'
                | '\u{1dc0}'..='\u{1dff}'
                | '\u{20d0}'..='\u{20ff}'
                | '\u{fe20}'..='\u{fe2f}'
        ) {
            continue;
        }
        match ch {
            'a' | 'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => folded.push('a'),
            'c' | 'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => folded.push('c'),
            'd' | 'ď' | 'đ' | 'ð' => folded.push('d'),
            'e' | 'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => folded.push('e'),
            'g' | 'ĝ' | 'ğ' | 'ġ' | 'ģ' => folded.push('g'),
            'h' | 'ĥ' | 'ħ' => folded.push('h'),
            'i' | 'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => folded.push('i'),
            'j' | 'ĵ' => folded.push('j'),
            'k' | 'ķ' => folded.push('k'),
            'l' | 'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => folded.push('l'),
            'n' | 'ñ' | 'ń' | 'ņ' | 'ň' => folded.push('n'),
            'o' | 'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => folded.push('o'),
            'r' | 'ŕ' | 'ŗ' | 'ř' => folded.push('r'),
            's' | 'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => folded.push('s'),
            't' | 'ţ' | 'ť' | 'ŧ' | 'ț' => folded.push('t'),
            'u' | 'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => {
                folded.push('u')
            }
            'w' | 'ŵ' => folded.push('w'),
            'y' | 'ý' | 'ÿ' | 'ŷ' => folded.push('y'),
            'z' | 'ź' | 'ż' | 'ž' => folded.push('z'),
            'æ' => folded.push_str("ae"),
            'œ' => folded.push_str("oe"),
            'ß' => folded.push_str("ss"),
            'þ' => folded.push_str("th"),
            _ => folded.push(ch),
        }
    }
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Word tokens of an entity lookup/name, ignoring punctuation and whitespace.
pub fn entity_lookup_tokens(input: &str) -> Vec<String> {
    normalize_entity_lookup(input)
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

/// Canonical `mode` values for an authoring run.
pub const AUTHORING_MODES: [&str; 2] = ["hybrid", "agent"];

/// Execution mode for an authoring run (`hybrid` or `agent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringMode {
    #[default]
    Hybrid,
    Agent,
}

impl AuthoringMode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::Agent => "agent",
        }
    }

    pub const fn uses_agent_drafting(&self) -> bool {
        matches!(self, Self::Agent)
    }
}

impl std::str::FromStr for AuthoringMode {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "hybrid" => Ok(Self::Hybrid),
            "agent" | "agents" | "auto" | "automated" | "offload" | "full_agent"
            | "fully_automated" | "agent_draft" | "agent-draft" | "agent_drafting"
            | "agent-drafting" | "internal" | "internal_draft" | "internal-draft" => {
                Ok(Self::Agent)
            }
            _ => Err(raw.to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for AuthoringMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse::<Self>().map_err(|rejected| {
            serde::de::Error::custom(format!(
                "mode must be one of {:?}, got {:?}",
                AUTHORING_MODES, rejected
            ))
        })
    }
}

/// Valid `mining_policy` values for an authoring run.
// Evolution §3.1: `"disabled"` leaves the loop unchanged; `"propose_all"`
// inserts the per-scene MineScene step.
pub const AUTHORING_MINING_POLICIES: [&str; 2] = ["disabled", "propose_all"];

/// Canon-mining policy for an authoring run (`disabled` or `propose_all`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringMiningPolicy {
    #[default]
    Disabled,
    ProposeAll,
}

impl AuthoringMiningPolicy {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::ProposeAll => "propose_all",
        }
    }

    pub fn into_persisted_option(self) -> Option<String> {
        match self {
            Self::Disabled => None,
            Self::ProposeAll => Some("propose_all".to_string()),
        }
    }
}

impl std::str::FromStr for AuthoringMiningPolicy {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "disabled" => Ok(Self::Disabled),
            "propose_all" => Ok(Self::ProposeAll),
            _ => Err(raw.to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for AuthoringMiningPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse::<Self>().map_err(|rejected| {
            serde::de::Error::custom(format!(
                "mining_policy must be one of {:?}, got {:?}",
                AUTHORING_MINING_POLICIES, rejected
            ))
        })
    }
}

/// Validate and canonicalize an optional `mining_policy` from run input.
// - `None` or `"disabled"` → `Ok(None)` (persists NULL, matching pre-upgrade runs).
// - `"propose_all"` → `Ok(Some("propose_all"))`.
// - anything else → `Err(rejected value)`.
pub fn validate_mining_policy(policy: Option<&str>) -> Result<Option<String>, String> {
    match policy {
        None => Ok(None),
        Some(raw) => raw
            .parse::<AuthoringMiningPolicy>()
            .map(AuthoringMiningPolicy::into_persisted_option),
    }
}

/// Valid `replan_policy` values for an authoring run.
// ADR 0003, evolution §3.5: `"disabled"` never replans; `"propose_all"` stages
// amendment proposals after each chapter summary.
pub const AUTHORING_REPLAN_POLICIES: [&str; 2] = ["disabled", "propose_all"];

/// Living-outline replan policy for an authoring run (`disabled` or `propose_all`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringReplanPolicy {
    #[default]
    Disabled,
    ProposeAll,
}

impl AuthoringReplanPolicy {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::ProposeAll => "propose_all",
        }
    }

    pub fn into_persisted_option(self) -> Option<String> {
        match self {
            Self::Disabled => None,
            Self::ProposeAll => Some("propose_all".to_string()),
        }
    }
}

impl std::str::FromStr for AuthoringReplanPolicy {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "disabled" => Ok(Self::Disabled),
            "propose_all" => Ok(Self::ProposeAll),
            _ => Err(raw.to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for AuthoringReplanPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse::<Self>().map_err(|rejected| {
            serde::de::Error::custom(format!(
                "replan_policy must be one of {:?}, got {:?}",
                AUTHORING_REPLAN_POLICIES, rejected
            ))
        })
    }
}

/// Validate and canonicalize an optional `replan_policy` from run input.
// Mirrors [`validate_mining_policy`] (ADR 0003, evolution §3.5).
pub fn validate_replan_policy(policy: Option<&str>) -> Result<Option<String>, String> {
    match policy {
        None => Ok(None),
        Some(raw) => raw
            .parse::<AuthoringReplanPolicy>()
            .map(AuthoringReplanPolicy::into_persisted_option),
    }
}

/// Valid `checkpoint_policy` values for an authoring run.
// Evolution §3.3: `"manual"` runs the classic 4-step operator checkpoint flow;
// `"auto_advisory"` and `"auto_strict"` run in-process checkpoint automation.
pub const AUTHORING_CHECKPOINT_POLICIES: [&str; 3] = ["manual", "auto_advisory", "auto_strict"];

/// Checkpoint review policy for an authoring run (`manual`, `auto_advisory`, or `auto_strict`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringCheckpointPolicy {
    #[default]
    Manual,
    AutoAdvisory,
    AutoStrict,
}

impl AuthoringCheckpointPolicy {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::AutoAdvisory => "auto_advisory",
            Self::AutoStrict => "auto_strict",
        }
    }

    pub fn into_persisted_option(self) -> Option<String> {
        match self {
            Self::Manual => None,
            Self::AutoAdvisory => Some("auto_advisory".to_string()),
            Self::AutoStrict => Some("auto_strict".to_string()),
        }
    }
}

impl std::str::FromStr for AuthoringCheckpointPolicy {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "manual" => Ok(Self::Manual),
            "auto_advisory" => Ok(Self::AutoAdvisory),
            "auto_strict" => Ok(Self::AutoStrict),
            _ => Err(raw.to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for AuthoringCheckpointPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse::<Self>().map_err(|rejected| {
            serde::de::Error::custom(format!(
                "checkpoint_policy must be one of {:?}, got {:?}",
                AUTHORING_CHECKPOINT_POLICIES, rejected
            ))
        })
    }
}

/// Validate and canonicalize an optional `checkpoint_policy` from run input.
// - `None` or `"manual"` → `Ok(None)` (persists NULL, matching pre-policy runs; evolution §3.3, I1).
// - `"auto_advisory"` / `"auto_strict"` → `Ok(Some(canonical))`.
// - anything else → `Err(rejected value)`.
pub fn validate_checkpoint_policy(policy: Option<&str>) -> Result<Option<String>, String> {
    match policy {
        None => Ok(None),
        Some(raw) => raw
            .parse::<AuthoringCheckpointPolicy>()
            .map(AuthoringCheckpointPolicy::into_persisted_option),
    }
}

/// The inclusive upper bound on `max_revise_attempts` (evolution §3.2). The
/// in-run revise loop is a bound, not a tunable knob — one or two attempts keep
/// the loop convergent while capping model-call cost. `>2` or negative is an
/// input error.
pub const MAX_REVISE_ATTEMPTS_UPPER_BOUND: i32 = 2;

/// Validate and canonicalize an optional `max_revise_attempts` from run input
/// (evolution §3.2).
///
/// - `None` → `Ok(None)` (revise disabled = default, byte-identical to the
///   pre-revise loop).
/// - `Some(0)` → `Ok(None)`. Zero is the explicit "disabled" spelling; it
///   canonicalizes to `None` so the run persists NULL, matching a pre-upgrade
///   row exactly.
/// - `Some(n)` for `1..=2` → `Ok(Some(n))`.
/// - anything else (`n > 2`, negative) → `Err(rejected value)` for an
///   input-error message.
pub fn validate_max_revise_attempts(attempts: Option<i32>) -> Result<Option<i32>, i32> {
    match attempts {
        None | Some(0) => Ok(None),
        Some(n) if (1..=MAX_REVISE_ATTEMPTS_UPPER_BOUND).contains(&n) => Ok(Some(n)),
        Some(other) => Err(other),
    }
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

// =============================================================================
// Canon deltas (ADR 0001 — canon mining & ratification)
// =============================================================================

/// The v1 canon-delta class vocabulary (ADR 0001 D1). Order and spelling are a
/// **one-way door**: staged rows persist in operator databases keyed on these
/// strings, so a rename strands rows and breaks decided-audit replay.
///
/// Classes are validated strings, not an enum — additions are additive by
/// construction (ADR reversal-cost note), but [`is_canon_delta_class`] rejects
/// any class not in this table at the staging boundary.
pub const CANON_DELTA_CLASSES: [&str; 14] = [
    "canonical_fact",
    "promise_planted",
    "promise_payoff_candidate",
    "promise_reinforced",
    "relationship_shift",
    "character_state",
    "knowledge_learned",
    "beat_annotation",
    "try_fail_cycle",
    "consequence_delivered",
    "escalation_demonstrated",
    "arc_milestone_reached",
    "quantity_change",
    "entity_candidate",
];

/// Whether `class` is one of the recognised [`CANON_DELTA_CLASSES`]. Exact
/// (case-sensitive) match — the class vocabulary is a public contract, not a
/// free-form label. Staging rejects anything this returns `false` for.
pub fn is_canon_delta_class(class: &str) -> bool {
    CANON_DELTA_CLASSES.contains(&class)
}

// A proposed canon delta mined from a committed scene and awaiting operator
// ratification (ADR 0001 D2). Read model: timestamps are ISO-8601 strings,
// mapped from the stored microsecond representation at the adapter boundary
// (mirrors [`SessionActivity`] / [`ProgressionEvent`]). `payload` is the typed
// per-class JSON — validated by the write tool the class maps to on apply.
/// A proposed canon delta mined from a committed scene and awaiting operator ratification.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CanonDelta {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    /// Provenance: the scene this delta was mined from.
    pub scene_id: String,
    /// The authoring run that mined it, or `None` when mined outside a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authoring_run_id: Option<String>,
    /// One of [`CANON_DELTA_CLASSES`].
    pub delta_class: String,
    /// The existing entity this modifies; `None` proposes a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// Typed per-class payload (the class's write-tool input, minus the fields
    /// injected at apply).
    pub payload: serde_json::Value,
    /// Sanitized prose excerpt (≤300 chars) grounding the proposal; mandatory.
    pub evidence: String,
    /// `high` | `medium` | `low`.
    pub confidence: String,
    /// `staged` | `applied` | `rejected` | `superseded`.
    pub status: String,
    /// Ratification audit — when the decision was recorded (ISO-8601).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
    /// Ratification audit — who recorded the decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// Input for `mine_scene_canon` (evolution §3.1): mine one committed scene's
// prose into proposed canon deltas awaiting operator ratification. Branch
// resolution is implicit — the service mines the project's active branch,
// matching the other scene-scoped read passes.
/// Input for mining one committed scene's prose into proposed canon deltas awaiting operator ratification.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MineSceneCanonInput {
    /// Project identifier.
    pub project_id: String,
    /// Scene identifier.
    pub scene_id: String,
}

// Output of `mine_scene_canon`. `status` is one of `staged` (deltas persisted),
// `skipped` (no cleared route / empty prose — `skip_reason` names why), or
// `model_output_rejected` (the model's JSON was malformed; nothing staged). A
// skip or rejection never reads as a clean mine (evolution I8).
/// Output of `mine_scene_canon`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MineSceneCanonOutput {
    /// The surviving deltas staged this pass, in prompt order.
    #[serde(default)]
    pub staged: Vec<CanonDelta>,
    /// Deltas the model proposed but that failed validation (unknown class,
    /// evidence not verbatim in prose, missing entity kind).
    pub discarded_count: usize,
    /// Prior `staged` deltas for this scene flipped to `superseded` on remine.
    pub superseded_count: usize,
    /// `staged` | `skipped` | `model_output_rejected`.
    pub status: String,
    /// Present only when `status == "skipped"`; names the route+rating that was
    /// uncleared, or "empty scene". Never carries prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
}

// =============================================================================
// Plan amendments (ADR 0003 — living-outline replanning & ratification)
// =============================================================================

/// The v1 plan-amendment class vocabulary (ADR 0003 D1). Order and spelling are
/// a **one-way door**: staged rows persist in operator databases keyed on these
/// strings, so a rename strands rows and breaks decided-audit replay (identical
/// contract to [`CANON_DELTA_CLASSES`]).
///
/// Classes are validated strings, not an enum — additions are additive by
/// construction (ADR reversal-cost note), but [`is_plan_amendment_class`]
/// rejects any class not in this table at the staging boundary. Each applies
/// through an existing plan write path only (`plan_chapter` /
/// `create_narrative_promise`) — the replanner never gains a novel write path.
pub const PLAN_AMENDMENT_CLASSES: [&str; 8] = [
    "synopsis_update",
    "scene_add",
    "scene_drop",
    "scene_replace",
    "scene_reorder",
    "thread_promote",
    "thread_retire",
    "promise_followup",
];

/// Whether `class` is one of the recognised [`PLAN_AMENDMENT_CLASSES`]. Exact
/// (case-sensitive) match — the class vocabulary is a public contract, not a
/// free-form label. Staging rejects anything this returns `false` for.
pub fn is_plan_amendment_class(class: &str) -> bool {
    PLAN_AMENDMENT_CLASSES.contains(&class)
}

// A proposed plan amendment staged by the replan differ and awaiting operator
// ratification (ADR 0003 D2). Read model: timestamps are ISO-8601 strings,
// mapped from the stored microsecond representation at the adapter boundary
// (mirrors [`CanonDelta`]). `payload` is the typed per-class JSON — the minimal
// delta against the plan-write input the class applies through (Part B).
/// A proposed plan amendment staged by the replan differ and awaiting operator ratification.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanAmendment {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    /// Provenance: the summarized chapter whose save triggered the replan pass.
    pub source_chapter: i32,
    /// The book the source/target chapter numbers belong to.
    pub book_number: i32,
    /// The authoring run that staged it, or `None` when replanned outside a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authoring_run_id: Option<String>,
    /// One of [`PLAN_AMENDMENT_CLASSES`].
    pub amendment_class: String,
    /// The future chapter this amends. `None` only for `promise_followup`
    /// (which targets a future placement, not a chapter row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_chapter: Option<i32>,
    // Typed per-class payload (the minimal delta against the class's plan-write
    // input, replayed on apply by Part B).
    /// Typed per-class payload for the plan change.
    pub payload: serde_json::Value,
    /// The replanner's stated reasoning (ids/summaries only, no prose quotes);
    /// mandatory, non-empty, ≤500 chars.
    pub rationale: String,
    /// `high` | `medium` | `low`.
    pub confidence: String,
    /// `staged` | `applied` | `rejected` | `superseded`.
    pub status: String,
    /// Ratification audit — when the decision was recorded (ISO-8601).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
    /// Ratification audit — who recorded the decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    // ADR D4 history: the affected plan slice snapshotted at apply time, before
    // the write. `None` until an apply captures it (Part B).
    /// The affected plan slice snapshotted at apply time, before the write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_state: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// Input for the replan differ (`replan_chapter`, ADR 0003). The pass audits the
// realized reality of `source_chapter` against every not-yet-drafted chapter's
// plan strictly after it in the same book, staging amendment proposals the
// operator ratifies. Branch resolution is implicit — the differ reads the
// project's active branch, matching the other summary-scoped passes.
/// Input for auditing a summarized chapter against future undrafted chapter plans to stage amendment proposals.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReplanChapterInput {
    /// Project identifier.
    pub project_id: String,
    /// 1-based book number.
    pub book_number: i32,
    /// The just-summarized chapter whose realized reality drives the audit.
    pub source_chapter: i32,
}

// Output of `replan_chapter` (ADR 0003). `status` is one of `staged`
// (amendments persisted), `no_summary` (the source chapter has no saved
// summary — nothing to compare against), `no_targets` (no eligible undrafted
// future chapter), `skipped` (no cleared route / transport error —
// `skip_reason` names the error class, never prose), or `model_output_rejected`
// (the model's JSON was malformed; nothing staged). A skip, no-summary,
// no-targets, or rejection never reads as a clean pass (evolution I8).
/// Output of `replan_chapter`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReplanChapterOutput {
    /// The surviving amendments staged this pass, in prompt order (cap 8).
    #[serde(default)]
    pub staged: Vec<PlanAmendment>,
    /// Amendments the model proposed but that failed validation (unknown class,
    /// empty rationale, missing/forbidden target chapter, bad payload, target
    /// outside the eligible set) or that overflowed the per-pass cap.
    pub discarded_count: usize,
    /// Prior `staged` amendments for this source chapter flipped to `superseded`
    /// on rerun.
    pub superseded_count: usize,
    /// `staged` | `no_summary` | `no_targets` | `skipped` | `model_output_rejected`.
    pub status: String,
    /// Present only when `status == "skipped"`; names the route+rating that was
    /// uncleared or the transport failure. Never carries prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
}

// Input for `list_plan_amendments` (ADR 0003): read the living-outline ratify
// queue on a project's active branch. Filters compose (AND): `status` narrows
// to `staged`/`applied`/`rejected`/`superseded`; `book_number` + `source_chapter`
// scope to one replan pass's provenance (both required together — a lone
// `book_number` or lone `source_chapter` is an input error). Mirrors
// [`ListCanonDeltasInput`].
/// Input for listing the living-outline ratification queue on a project's active branch.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListPlanAmendmentsInput {
    /// Project identifier.
    pub project_id: String,
    /// `staged` | `applied` | `rejected` | `superseded`; `None` returns all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Scope to the amendments a specific replan pass staged: the book the
    /// `source_chapter` belongs to. Must be paired with `source_chapter`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_number: Option<i32>,
    /// Scope to the amendments triggered by a specific summarized chapter. Must
    /// be paired with `book_number`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_chapter: Option<i32>,
}

/// Output of `list_plan_amendments`: the matching amendments in the repository's
/// deterministic `(created_at, id)` order.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListPlanAmendmentsOutput {
    #[serde(default)]
    pub amendments: Vec<PlanAmendment>,
}

// One operator ratification decision on a staged plan amendment (ADR 0003 D5).
// `action` is `apply` or `reject`. `edit` optionally replaces the staged
// payload (ratify-with-correction) — the edited payload is what is applied AND
// recorded on the row. `note` is an operator annotation surfaced in the output
// (the `plan_amendment` row has no note column, so it is never persisted).
// Mirrors [`CanonDeltaDecisionInput`].
/// One operator ratification decision on a staged plan amendment.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanAmendmentDecisionInput {
    /// Plan amendment identifier.
    pub amendment_id: String,
    /// `apply` | `reject`.
    pub action: String,
    /// Corrected payload replacing the staged one on apply. On reject it is
    /// recorded but not applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "any_object_schema")]
    pub edit: Option<serde_json::Value>,
    /// Free-form operator note; echoed in the output, not persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// Input for `decide_plan_amendments` (ADR 0003 D5): the apply dispatcher. All
// decisions are pre-flighted before any write; a single pre-flight failure
// aborts the whole call with zero writes (decisions on already-decided rows are
// input errors — finality). The ADR D3 immutability guard is checked at apply
// time for every chapter-targeting class. `decided_by` defaults to `operator`.
// Mirrors [`DecideCanonDeltasInput`].
/// Input for applying or rejecting staged plan amendments.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DecidePlanAmendmentsInput {
    /// Project identifier.
    pub project_id: String,
    /// Ordered ratification decisions to apply or reject.
    #[serde(default)]
    pub decisions: Vec<PlanAmendmentDecisionInput>,
    /// Actor recording the decision; defaults to `operator`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
}

// The per-decision outcome in a `decide_plan_amendments` response.
//
// * `applied` — the amendment replayed through its plan write path and the
//   decision (with the ADR D4 prior-state snapshot) was recorded.
// * `rejected` — the amendment was marked rejected; no plan write.
// * `failed` — a real write error occurred after pre-flight; earlier applies in
//   the batch stay applied (they are real outline changes), this row stays
//   staged.
// * `not_reached` — a later decision the dispatcher never got to because an
//   earlier one failed (this row stays staged).
//
// Mirrors [`CanonDeltaDecisionResult`].
/// Per-decision outcome in a `decide_plan_amendments` response.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanAmendmentDecisionResult {
    pub amendment_id: String,
    /// `applied` | `rejected` | `failed` | `not_reached`.
    pub outcome: String,
    /// Present on `failed`: the write error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Present on `applied` for `promise_followup` (the created narrative-promise
    /// id). `None` for the chapter-plan classes, which rewrite a plan in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_record_id: Option<String>,
    /// The operator note carried on the decision input, echoed back (the row has
    /// no note column, so this is the only place it survives).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Output of `decide_plan_amendments`: one result per decision in input order,
/// plus counts. Mirrors [`DecideCanonDeltasOutput`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DecidePlanAmendmentsOutput {
    #[serde(default)]
    pub results: Vec<PlanAmendmentDecisionResult>,
    pub applied_count: usize,
    pub rejected_count: usize,
    pub failed_count: usize,
}

// A single reader-simulation concern (evolution §3.6 cumulative reader-sim).
// `severity` is `info` | `warning` — a craft signal only, never a hard gate:
// reader-sim concerns are report-only, matching the sampled-review outcomes
// which never fold into the auto-checkpoint verdict counts. `description` is
// the model's own concern text (a craft observation, not committed prose).
/// A single reader-simulation craft concern.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ReaderSimConcern {
    /// `info` | `warning`. A `warning` is a stronger craft signal but does NOT
    /// block the checkpoint (report-only, per the studied verdict semantics).
    pub severity: String,
    /// The model's concern text (a craft observation).
    pub description: String,
}

// Input to the per-chapter cumulative reader-simulation pass (evolution §3.6).
// The service resolves the ordered scenes by id, concatenates their prose in
// spine order, builds the persona + prior-notes prompt, dispatches through the
// `reader_sim` → `review` fallback ladder gated at `rating`, and parses strict
// JSON out. Prior notes flow in so the reader accumulates memory chapter to
// chapter across a checkpoint range.
/// Input to the per-chapter cumulative reader-simulation pass.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReaderSimChapterInput {
    /// Project identifier.
    pub project_id: String,
    /// Active-branch scene ids for this chapter, already ordered by scene spine
    /// (the caller resolves the order from run state). Empty ⇒ nothing to read.
    #[serde(default)]
    pub scene_ids: Vec<String>,
    /// The batch content rating for this chapter (the strictest across its
    /// scenes — `max_scene_rating`). Gates the prose-bearing dispatch. `None`
    /// leaves the request unrated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
    /// The reader's cumulative notes carried in from the prior chapter (already
    /// char-capped by the caller). Empty on the first chapter of a range.
    #[serde(default)]
    pub prior_notes: String,
}

// Outcome of one chapter's reader-simulation pass (evolution §3.6). `status` is
// one of `read` (the model returned parseable JSON — `engagement`/`notes`/
// `concerns` are populated), `unparsed` (the model output was malformed;
// `engagement` is `"unparsed"`, `notes` echoes the caller's prior notes so the
// reader loses no memory, and `concerns` is empty), or `skipped` (no cleared
// route / transport error / no prose — `skip_reason` names why, never prose).
// A skip or an unparsed read never reads as a clean pass (evolution I8).
/// Outcome of one chapter's reader-simulation pass.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReaderSimChapterOutcome {
    /// Updated reader questions. Omitted by older models; an explicit empty
    /// array means all previous questions were answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_questions: Option<Vec<String>>,
    /// `read` | `unparsed` | `skipped`.
    pub status: String,
    /// The reader's engagement verdict: `high` | `steady` | `dipping` on a
    /// parsed read; `"unparsed"` when the output was malformed; `"skipped"` when
    /// the pass was skipped.
    pub engagement: String,
    /// The cumulative, self-contained replacement notes the reader emitted (on
    /// `read`), or the preserved prior notes (on `unparsed`), or empty (on
    /// `skipped`). This is the reader's own craft memory, not committed prose.
    #[serde(default)]
    pub notes: String,
    /// Concerns the reader raised this chapter (empty unless `status == "read"`).
    #[serde(default)]
    pub concerns: Vec<ReaderSimConcern>,
    /// Present only when `status == "skipped"`; names the route+rating that was
    /// uncleared, the transport failure, or "no prose". Never carries prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
}

/// An inclusive chapter span within a single book, used to scope a
/// `list_canon_deltas` read to the scenes of those chapters (mirrors the way
/// `compile_manuscript` resolves a chapter range to active-branch scenes).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChapterRange {
    /// The book the chapter numbers belong to.
    pub book_number: i32,
    /// Inclusive first chapter.
    pub start: i32,
    /// Inclusive last chapter.
    pub end: i32,
}

// Input for `list_canon_deltas` (ADR 0001): read the ratify queue on a
// project's active branch. Filters compose (AND): `status` narrows to
// `staged`/`applied`/`rejected`/`superseded`; `scene_id` narrows to one
// scene's provenance; `chapter_range` resolves to the set of active-branch
// scenes in those chapters. `scene_id` and `chapter_range` are mutually
// exclusive — supplying both is an input error.
/// Input for listing the canon-delta ratification queue on a project's active branch.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListCanonDeltasInput {
    /// Project identifier.
    pub project_id: String,
    /// `staged` | `applied` | `rejected` | `superseded`; `None` returns all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Restrict to deltas mined from this one scene.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    /// Restrict to deltas mined from any scene in this chapter span.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_range: Option<ChapterRange>,
}

/// Output of `list_canon_deltas`: the matching deltas in the repository's
/// deterministic `(created_at, id)` order.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListCanonDeltasOutput {
    #[serde(default)]
    pub deltas: Vec<CanonDelta>,
}

// One operator ratification decision on a staged canon delta (ADR 0001 D3).
// `action` is `apply` or `reject`. `edit` optionally replaces the staged
// payload (ratify-with-correction) — the edited payload is what is applied AND
// recorded on the row. `note` is an operator annotation surfaced in the output
// (the `canon_delta` row has no note column, so it is never persisted).
/// One operator ratification decision on a staged canon delta.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CanonDeltaDecisionInput {
    /// Canon delta identifier.
    pub delta_id: String,
    /// `apply` | `reject`.
    pub action: String,
    /// Corrected payload replacing the staged one on apply. On reject it is
    /// recorded but not applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "any_object_schema")]
    pub edit: Option<serde_json::Value>,
    /// Free-form operator note; echoed in the output, not persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// Input for `decide_canon_deltas` (ADR 0001 D3): the apply dispatcher. All
// decisions are pre-flighted before any write; a single pre-flight failure
// aborts the whole call with zero writes (decisions on already-decided rows are
// input errors — finality). `decided_by` defaults to `operator`.
/// Input for applying or rejecting staged canon deltas.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DecideCanonDeltasInput {
    /// Project identifier.
    pub project_id: String,
    /// Ordered ratification decisions to apply or reject.
    #[serde(default)]
    pub decisions: Vec<CanonDeltaDecisionInput>,
    /// Actor recording the decision; defaults to `operator`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
}

/// The per-decision outcome in a `decide_canon_deltas` response.
///
/// * `applied` — the delta's write tool ran and the decision was recorded.
/// * `rejected` — the delta was marked rejected; no canon write.
/// * `failed` — a real write error occurred after pre-flight; earlier applies in
///   the batch stay applied (they are recorded canon), this row stays staged.
/// * `not_reached` — a later decision the dispatcher never got to because an
///   earlier one failed (this row stays staged).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CanonDeltaDecisionResult {
    pub delta_id: String,
    /// `applied` | `rejected` | `failed` | `not_reached`.
    pub outcome: String,
    /// Present on `failed`: the write error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Present on `applied` when the write tool returns a fresh record id (e.g.
    /// a created fact/promise/entity). `None` for updates that mutate in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_record_id: Option<String>,
    /// The operator note carried on the decision input, echoed back (the row has
    /// no note column, so this is the only place it survives).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Output of `decide_canon_deltas`: one result per decision in input order,
/// plus counts.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DecideCanonDeltasOutput {
    #[serde(default)]
    pub results: Vec<CanonDeltaDecisionResult>,
    pub applied_count: usize,
    pub rejected_count: usize,
    pub failed_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_character_accepts_null_and_missing_profiles() {
        // Defect item 5: voice_profile and emotional_profile must be optional —
        // explicit null (what the canon miner emits for a minor character) and
        // omission both deserialize to empty defaults.
        let with_nulls: CreateCharacterInput = serde_json::from_value(serde_json::json!({
            "project_id": "project:p1",
            "name": "The stranger",
            "summary": "A stranger.",
            "role": "minor",
            "voice_profile": null,
            "emotional_profile": null
        }))
        .expect("null profiles must deserialize");
        assert!(with_nulls.voice_profile.tone.is_none());
        assert!(with_nulls.emotional_profile.base_emotions.is_empty());

        let with_missing: CreateCharacterInput = serde_json::from_value(serde_json::json!({
            "project_id": "project:p1",
            "name": "The stranger",
            "summary": "A stranger.",
            "role": "minor"
        }))
        .expect("missing profiles must deserialize");
        assert!(with_missing.voice_profile.vocabulary.is_empty());
        assert!(with_missing.emotional_profile.suppressed.is_empty());
    }

    #[test]
    fn scene_verify_checks_is_deduped_and_excludes_deep_only_checks() {
        // No duplicates.
        let mut seen = std::collections::BTreeSet::new();
        for check in SCENE_VERIFY_CHECKS {
            assert!(seen.insert(*check), "duplicate scene-verify check: {check}");
        }
        assert!(!SCENE_VERIFY_CHECKS.is_empty());
        // The deliberately-excluded deep-only / cross-chapter-trend checks must
        // never leak into the fast scene-verify subset (design §3.2).
        for excluded in [
            "promise_payoff_detection",
            "scene_purpose_fulfillment",
            "pacing_drift",
            "research_accuracy",
            "scene_divergence",
        ] {
            assert!(
                !SCENE_VERIFY_CHECKS.contains(&excluded),
                "scene-verify subset must exclude {excluded}"
            );
        }
        assert!(SCENE_VERIFY_CHECKS.contains(&"retcon_reachability"));
        assert!(
            SCENE_VERIFY_CHECKS.contains(&"anti_slop"),
            "Phase 3: hard shelves join the scene-scoped verify set"
        );
    }

    #[test]
    fn scene_order_requires_single_chapter_chapter_range_scope() {
        // Legacy shape: no scene_order → ChapterRange, unchanged.
        let plain = ConsistencyScopeInput::chapter_range(1, 1, 1, 1);
        assert!(matches!(
            plain.to_scope(),
            Ok(ConsistencyScope::ChapterRange { .. })
        ));

        // Single-chapter range + scene_order → Scene.
        let scene = ConsistencyScopeInput {
            scene_order: Some(2),
            ..ConsistencyScopeInput::chapter_range(1, 1, 1, 1)
        };
        assert!(matches!(
            scene.to_scope(),
            Ok(ConsistencyScope::Scene {
                book_number: 1,
                chapter_number: 1,
                scene_order: 2
            })
        ));

        // Multi-chapter range + scene_order → error naming the constraint.
        let multi = ConsistencyScopeInput {
            scene_order: Some(1),
            ..ConsistencyScopeInput::chapter_range(1, 1, 1, 2)
        };
        let err = multi
            .to_scope()
            .expect_err("multi-chapter scene_order rejected");
        assert!(err.contains("single-chapter"), "constraint named: {err}");

        // scene_order paired with full/book is a caller mistake.
        let full = ConsistencyScopeInput {
            scene_order: Some(1),
            ..ConsistencyScopeInput::full()
        };
        assert!(full.to_scope().is_err(), "scene_order + full is rejected");
        let book = ConsistencyScopeInput {
            scene_order: Some(1),
            ..ConsistencyScopeInput::book(1)
        };
        assert!(book.to_scope().is_err(), "scene_order + book is rejected");
    }

    #[test]
    fn validate_mining_policy_accepts_known_and_rejects_unknown() {
        // None (default) and explicit "disabled" both canonicalize to None so
        // the run persists NULL = byte-identical disabled state.
        assert_eq!(validate_mining_policy(None), Ok(None));
        assert_eq!(validate_mining_policy(Some("disabled")), Ok(None));
        assert_eq!(validate_mining_policy(Some("  Disabled ")), Ok(None));
        // propose_all is the one enabling value.
        assert_eq!(
            validate_mining_policy(Some("propose_all")),
            Ok(Some("propose_all".to_string()))
        );
        assert_eq!(
            validate_mining_policy(Some("PROPOSE_ALL")),
            Ok(Some("propose_all".to_string()))
        );
        // Anything else is an input error carrying the rejected value verbatim.
        assert_eq!(
            validate_mining_policy(Some("auto_accept")),
            Err("auto_accept".to_string())
        );
    }

    #[test]
    fn validate_replan_policy_accepts_known_and_rejects_unknown() {
        // None (default) and explicit "disabled" both canonicalize to None so
        // the run persists NULL = byte-identical disabled state (ADR 0003 D5).
        assert_eq!(validate_replan_policy(None), Ok(None));
        assert_eq!(validate_replan_policy(Some("disabled")), Ok(None));
        assert_eq!(validate_replan_policy(Some("  Disabled ")), Ok(None));
        // propose_all is the one enabling value.
        assert_eq!(
            validate_replan_policy(Some("propose_all")),
            Ok(Some("propose_all".to_string()))
        );
        assert_eq!(
            validate_replan_policy(Some("PROPOSE_ALL")),
            Ok(Some("propose_all".to_string()))
        );
        // Anything else is an input error carrying the rejected value verbatim.
        assert_eq!(
            validate_replan_policy(Some("auto_accept")),
            Err("auto_accept".to_string())
        );
        assert_eq!(AUTHORING_REPLAN_POLICIES, ["disabled", "propose_all"]);
    }

    #[test]
    fn validate_checkpoint_policy_accepts_known_and_rejects_unknown() {
        // None (default) and explicit "manual" both canonicalize to None so the
        // run persists NULL = manual = byte-identical to the pre-policy flow.
        assert_eq!(validate_checkpoint_policy(None), Ok(None));
        assert_eq!(validate_checkpoint_policy(Some("manual")), Ok(None));
        assert_eq!(validate_checkpoint_policy(Some("  Manual ")), Ok(None));
        // auto_advisory / auto_strict are the two enabling values.
        assert_eq!(
            validate_checkpoint_policy(Some("auto_advisory")),
            Ok(Some("auto_advisory".to_string()))
        );
        assert_eq!(
            validate_checkpoint_policy(Some("AUTO_STRICT")),
            Ok(Some("auto_strict".to_string()))
        );
        // Anything else is an input error carrying the rejected value verbatim.
        assert_eq!(
            validate_checkpoint_policy(Some("auto")),
            Err("auto".to_string())
        );
    }

    #[test]
    fn validate_max_revise_attempts_bounds_zero_to_two() {
        // None (default / disabled) and explicit 0 both canonicalize to None so
        // the run persists NULL = byte-identical to the pre-revise loop.
        assert_eq!(validate_max_revise_attempts(None), Ok(None));
        assert_eq!(validate_max_revise_attempts(Some(0)), Ok(None));
        // 1 and 2 are the only enabling bounds.
        assert_eq!(validate_max_revise_attempts(Some(1)), Ok(Some(1)));
        assert_eq!(validate_max_revise_attempts(Some(2)), Ok(Some(2)));
        // Above the bound and negative are input errors carrying the rejected value.
        assert_eq!(validate_max_revise_attempts(Some(3)), Err(3));
        assert_eq!(validate_max_revise_attempts(Some(-1)), Err(-1));
    }

    #[test]
    fn canon_delta_classes_match_adr_d1_exactly() {
        // ADR 0001 D1 — the class vocabulary is a one-way door; this test pins
        // the exact fourteen names and their order so a rename can never slip in
        // unnoticed (orphaned staged rows, broken audit replay).
        assert_eq!(CANON_DELTA_CLASSES.len(), 14);
        assert_eq!(
            CANON_DELTA_CLASSES,
            [
                "canonical_fact",
                "promise_planted",
                "promise_payoff_candidate",
                "promise_reinforced",
                "relationship_shift",
                "character_state",
                "knowledge_learned",
                "beat_annotation",
                "try_fail_cycle",
                "consequence_delivered",
                "escalation_demonstrated",
                "arc_milestone_reached",
                "quantity_change",
                "entity_candidate",
            ]
        );
    }

    #[test]
    fn canon_delta_class_validation_rejects_unknown_and_accepts_known() {
        assert!(is_canon_delta_class("beat_annotation"));
        assert!(is_canon_delta_class("entity_candidate"));
        assert!(!is_canon_delta_class("made_up_class"));
        assert!(!is_canon_delta_class(""));
        // Classes are case-sensitive validated strings, not an enum.
        assert!(!is_canon_delta_class("Canonical_Fact"));
    }

    #[test]
    fn canon_delta_read_model_round_trips() {
        let delta = CanonDelta {
            id: "canon_delta:01J".to_string(),
            project_id: "project:demo".to_string(),
            branch_id: "bible_branch:main".to_string(),
            scene_id: "scene:xyz".to_string(),
            authoring_run_id: Some("authoring_run:run1".to_string()),
            delta_class: "relationship_shift".to_string(),
            target_id: Some("relationship:ab".to_string()),
            payload: serde_json::json!({ "trust_delta": -2 }),
            evidence: "She turned away without a word.".to_string(),
            confidence: "high".to_string(),
            status: "staged".to_string(),
            decided_at: None,
            decided_by: None,
            created_at: "2026-07-19T00:00:00Z".to_string(),
            updated_at: "2026-07-19T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&delta).expect("serialize canon delta");
        let decoded: CanonDelta = serde_json::from_str(&json).expect("deserialize canon delta");
        assert_eq!(decoded.id, delta.id);
        assert_eq!(decoded.delta_class, "relationship_shift");
        assert_eq!(decoded.status, "staged");
        assert_eq!(decoded.payload["trust_delta"], -2);
        assert!(decoded.decided_at.is_none());
    }

    #[test]
    fn plan_amendment_classes_match_adr_d1_exactly() {
        // ADR 0003 D1 — the class vocabulary is a one-way door; this test pins
        // the exact eight names and their order so a rename can never slip in
        // unnoticed (orphaned staged rows, broken outline history).
        assert_eq!(PLAN_AMENDMENT_CLASSES.len(), 8);
        assert_eq!(
            PLAN_AMENDMENT_CLASSES,
            [
                "synopsis_update",
                "scene_add",
                "scene_drop",
                "scene_replace",
                "scene_reorder",
                "thread_promote",
                "thread_retire",
                "promise_followup",
            ]
        );
    }

    #[test]
    fn plan_amendment_class_validation_rejects_unknown_and_accepts_known() {
        assert!(is_plan_amendment_class("synopsis_update"));
        assert!(is_plan_amendment_class("promise_followup"));
        assert!(!is_plan_amendment_class("made_up_class"));
        assert!(!is_plan_amendment_class(""));
        // Classes are case-sensitive validated strings, not an enum.
        assert!(!is_plan_amendment_class("Synopsis_Update"));
    }

    #[test]
    fn plan_amendment_read_model_round_trips() {
        let amendment = PlanAmendment {
            id: "plan_amendment:01J".to_string(),
            project_id: "project:demo".to_string(),
            branch_id: "bible_branch:main".to_string(),
            source_chapter: 3,
            book_number: 1,
            authoring_run_id: Some("authoring_run:run1".to_string()),
            amendment_class: "synopsis_update".to_string(),
            target_chapter: Some(5),
            payload: serde_json::json!({ "synopsis": "The gate falls." }),
            rationale: "chapter 3 resolved the siege early".to_string(),
            confidence: "high".to_string(),
            status: "staged".to_string(),
            decided_at: None,
            decided_by: None,
            prior_state: None,
            created_at: "2026-07-20T00:00:00Z".to_string(),
            updated_at: "2026-07-20T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&amendment).expect("serialize plan amendment");
        let decoded: PlanAmendment =
            serde_json::from_str(&json).expect("deserialize plan amendment");
        assert_eq!(decoded.id, amendment.id);
        assert_eq!(decoded.amendment_class, "synopsis_update");
        assert_eq!(decoded.status, "staged");
        assert_eq!(decoded.target_chapter, Some(5));
        assert_eq!(decoded.payload["synopsis"], "The gate falls.");
        assert!(decoded.decided_at.is_none());
        assert!(decoded.prior_state.is_none());
    }

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

    #[test]
    fn authoring_enums_round_trip_canonical_values_and_compatibility_aliases() {
        for (raw, expected, uses_agent) in [
            ("hybrid", AuthoringMode::Hybrid, false),
            ("  HYBRID ", AuthoringMode::Hybrid, false),
            ("agent", AuthoringMode::Agent, true),
            ("  Agent ", AuthoringMode::Agent, true),
            ("agent_draft", AuthoringMode::Agent, true),
            ("agent-draft", AuthoringMode::Agent, true),
            ("agent_drafting", AuthoringMode::Agent, true),
            ("agent-drafting", AuthoringMode::Agent, true),
            ("internal", AuthoringMode::Agent, true),
            ("internal_draft", AuthoringMode::Agent, true),
            ("internal-draft", AuthoringMode::Agent, true),
        ] {
            let parsed: AuthoringMode =
                serde_json::from_value(serde_json::json!(raw)).expect("parse authoring mode");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.uses_agent_drafting(), uses_agent);
            let round_tripped: AuthoringMode = serde_json::from_str(
                &serde_json::to_string(&parsed).expect("serialize authoring mode"),
            )
            .expect("round-trip authoring mode");
            assert_eq!(round_tripped, expected);
        }
        assert!(
            serde_json::from_value::<AuthoringMode>(serde_json::json!("unknown_mode")).is_err()
        );

        for (raw, expected, persisted) in [
            ("disabled", AuthoringMiningPolicy::Disabled, None),
            ("  DISABLED ", AuthoringMiningPolicy::Disabled, None),
            (
                "propose_all",
                AuthoringMiningPolicy::ProposeAll,
                Some("propose_all".to_string()),
            ),
            (
                "  PROPOSE_ALL ",
                AuthoringMiningPolicy::ProposeAll,
                Some("propose_all".to_string()),
            ),
        ] {
            let parsed: AuthoringMiningPolicy =
                serde_json::from_value(serde_json::json!(raw)).expect("parse mining policy");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.into_persisted_option(), persisted);
            let round_tripped: AuthoringMiningPolicy = serde_json::from_str(
                &serde_json::to_string(&parsed).expect("serialize mining policy"),
            )
            .expect("round-trip mining policy");
            assert_eq!(round_tripped, expected);
        }

        for (raw, expected, persisted) in [
            ("disabled", AuthoringReplanPolicy::Disabled, None),
            ("  Disabled ", AuthoringReplanPolicy::Disabled, None),
            (
                "propose_all",
                AuthoringReplanPolicy::ProposeAll,
                Some("propose_all".to_string()),
            ),
            (
                "  PROPOSE_ALL ",
                AuthoringReplanPolicy::ProposeAll,
                Some("propose_all".to_string()),
            ),
        ] {
            let parsed: AuthoringReplanPolicy =
                serde_json::from_value(serde_json::json!(raw)).expect("parse replan policy");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.into_persisted_option(), persisted);
            let round_tripped: AuthoringReplanPolicy = serde_json::from_str(
                &serde_json::to_string(&parsed).expect("serialize replan policy"),
            )
            .expect("round-trip replan policy");
            assert_eq!(round_tripped, expected);
        }

        for (raw, expected, persisted) in [
            ("manual", AuthoringCheckpointPolicy::Manual, None),
            ("  Manual ", AuthoringCheckpointPolicy::Manual, None),
            (
                "auto_advisory",
                AuthoringCheckpointPolicy::AutoAdvisory,
                Some("auto_advisory".to_string()),
            ),
            (
                "  AUTO_STRICT ",
                AuthoringCheckpointPolicy::AutoStrict,
                Some("auto_strict".to_string()),
            ),
        ] {
            let parsed: AuthoringCheckpointPolicy =
                serde_json::from_value(serde_json::json!(raw)).expect("parse checkpoint policy");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.into_persisted_option(), persisted);
            let round_tripped: AuthoringCheckpointPolicy = serde_json::from_str(
                &serde_json::to_string(&parsed).expect("serialize checkpoint policy"),
            )
            .expect("round-trip checkpoint policy");
            assert_eq!(round_tripped, expected);
        }
    }

    #[test]
    fn prepare_scene_intent_round_trips_and_rejects_unknown_values() {
        for (raw, expected) in [
            ("draft", PrepareSceneIntent::Draft),
            ("  DRAFT ", PrepareSceneIntent::Draft),
            ("revise", PrepareSceneIntent::Revise),
            ("  Revise ", PrepareSceneIntent::Revise),
            ("continue", PrepareSceneIntent::Continue),
            ("  CONTINUE ", PrepareSceneIntent::Continue),
        ] {
            let parsed: PrepareSceneIntent =
                serde_json::from_value(serde_json::json!(raw)).expect("parse prepare_scene intent");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), expected.as_str());
            let round_tripped: PrepareSceneIntent = serde_json::from_str(
                &serde_json::to_string(&parsed).expect("serialize prepare_scene intent"),
            )
            .expect("round-trip prepare_scene intent");
            assert_eq!(round_tripped, expected);
        }

        let err = serde_json::from_value::<PrepareSceneIntent>(serde_json::json!("unknown"))
            .expect_err("unknown prepare_scene intent must be rejected");
        assert!(
            err.to_string()
                .contains("unsupported prepare_scene intent 'unknown'")
        );
    }
}
