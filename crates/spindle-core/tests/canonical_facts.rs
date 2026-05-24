#[path = "canonical_facts/helpers.rs"]
mod helpers;

#[path = "canonical_facts/alias_out_of_window.rs"]
mod alias_out_of_window;
#[path = "canonical_facts/boolean_drift.rs"]
mod boolean_drift;
#[path = "canonical_facts/cole_age_drift.rs"]
mod cole_age_drift;
#[path = "canonical_facts/cole_age_no_drift.rs"]
mod cole_age_no_drift;
#[path = "canonical_facts/date_drift.rs"]
mod date_drift;
#[path = "canonical_facts/enum_choice_contrast.rs"]
mod enum_choice_contrast;
#[path = "canonical_facts/hair_color_contrast.rs"]
mod hair_color_contrast;
#[path = "canonical_facts/legacy_untyped_skipped.rs"]
mod legacy_untyped_skipped;
#[path = "canonical_facts/list_alias_locality.rs"]
mod list_alias_locality;
#[path = "canonical_facts/list_forbidden_present.rs"]
mod list_forbidden_present;
#[path = "canonical_facts/list_required_missing.rs"]
mod list_required_missing;
#[cfg(feature = "perf")]
#[path = "canonical_facts/performance_large_fact_set.rs"]
mod performance_large_fact_set;
#[path = "canonical_facts/range_out_of_bounds.rs"]
mod range_out_of_bounds;
#[path = "canonical_facts/string_generic_mismatch.rs"]
mod string_generic_mismatch;
#[path = "canonical_facts/unicode_punctuation.rs"]
mod unicode_punctuation;
