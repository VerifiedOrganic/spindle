//! SQLite-backed import pipeline.
//!
//! Ports the SurrealDB-era `crate::import::*` source tree to the SQLite
//! repository surface. The module hierarchy mirrors the original layout:
//!
//! - `slicer`: source ingest + structural analysis (chapter/scene boundaries).
//! - `prompts`: import-pass prompt builders.
//! - `extract`: per-segment entity-candidate extractor.
//! - `consolidate`: mention -> cluster aggregator.
//! - `character`: cluster -> character-dossier builder.
//! - `world`, `narrative`, `final_state`: deeper analysis passes.
//!
//! Nothing in this module touches the database directly. Service code calls
//! these as pure functions, then routes results through
//! `crate::sqlite::repository::Repository::upsert_import_*` writers.

pub mod character;
pub mod consolidate;
pub mod extract;
pub mod final_state;
pub mod narrative;
pub mod prompts;
pub mod slicer;
pub mod world;

pub use slicer::{
    AnalyzedChapter, AnalyzedScene, IngestSourcesOptions, IngestedSourceDocument,
    StructuralAnalysisResult, analyze_structure, detect_source_format, ingest_sources,
    normalize_text, sha256_hex, structural_summary,
};
