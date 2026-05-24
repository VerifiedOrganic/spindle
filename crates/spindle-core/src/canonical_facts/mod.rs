pub mod validator;

pub use validator::{
    CanonicalFactForValidation, CanonicalFactViolation, ValidatorConfig, ViolationSeverity,
    parse_written_numeral, validate_prose_against_facts,
};
