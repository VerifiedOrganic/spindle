//! Configurable embedding index generations (P2 PR H).
//!
//! A generation identity includes provider, model, dimension, metric,
//! normalization and version. Vectors are validated (length, finite values)
//! before storage or query; mixed generations/dimensions are never compared.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const LEGACY_TOKEN_HASH_DIMENSION: usize = 64;
pub const LEGACY_TOKEN_HASH_GENERATION_ID: &str = "token-hash-v1:64";
pub const EMBEDDING_WRITE_CONFLICT: &str = "embedding_write_conflict";
pub const MAX_EMBEDDING_DIMENSION: usize = 4096;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingProviderKindV1 {
    TokenHash,
    Http,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingMetricV1 {
    Cosine,
    L2,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingNormalizationV1 {
    L2,
    None,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingGenerationStatusV1 {
    Building,
    Active,
    Retired,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingGenerationSpecV1 {
    pub provider: EmbeddingProviderKindV1,
    pub model: String,
    pub dimension: usize,
    pub metric: EmbeddingMetricV1,
    pub normalization: EmbeddingNormalizationV1,
    pub version: String,
}

impl EmbeddingGenerationSpecV1 {
    pub fn token_hash_legacy() -> Self {
        Self {
            provider: EmbeddingProviderKindV1::TokenHash,
            model: "token-hash-v1".into(),
            dimension: LEGACY_TOKEN_HASH_DIMENSION,
            metric: EmbeddingMetricV1::Cosine,
            normalization: EmbeddingNormalizationV1::L2,
            version: "token-hash-v1".into(),
        }
    }

    pub fn validate(&self) -> Result<(), EmbeddingValidationError> {
        if self.model.trim().is_empty() || self.version.trim().is_empty() {
            return Err(EmbeddingValidationError::Identity);
        }
        if self.dimension == 0 || self.dimension > MAX_EMBEDDING_DIMENSION {
            return Err(EmbeddingValidationError::DimensionOutOfRange {
                dimension: self.dimension,
            });
        }
        Ok(())
    }

    pub fn generation_id(&self) -> String {
        if self.provider == EmbeddingProviderKindV1::TokenHash
            && self.dimension == LEGACY_TOKEN_HASH_DIMENSION
            && self.version == "token-hash-v1"
        {
            return LEGACY_TOKEN_HASH_GENERATION_ID.to_string();
        }
        let mut hasher = Sha256::new();
        hasher.update(format!(
            "{}|{}|{}|{:?}|{:?}|{}",
            match self.provider {
                EmbeddingProviderKindV1::TokenHash => "token_hash",
                EmbeddingProviderKindV1::Http => "http",
            },
            self.model.trim(),
            self.dimension,
            self.metric,
            self.normalization,
            self.version.trim(),
        ));
        format!("embedding_generation:{:x}", hasher.finalize())
    }

    pub fn table_name(&self) -> String {
        generation_table_name(&self.generation_id())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum EmbeddingValidationError {
    Identity,
    DimensionOutOfRange {
        dimension: usize,
    },
    EmptyVector,
    WrongDimension {
        expected: usize,
        actual: usize,
    },
    NonFinite {
        index: usize,
    },
    Overflow {
        index: usize,
    },
    MixedGeneration {
        query_generation: String,
        index_generation: String,
    },
    QualityProbeFailed {
        expected_entity_id: String,
        rank: Option<usize>,
    },
    Incomplete {
        expected: usize,
        actual: usize,
    },
    NoActiveGeneration,
    RollbackWindowClosed,
}

impl std::fmt::Display for EmbeddingValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identity => write!(f, "embedding generation identity is empty"),
            Self::DimensionOutOfRange { dimension } => {
                write!(f, "embedding dimension {dimension} is out of range")
            }
            Self::EmptyVector => write!(f, "embedding vector is empty"),
            Self::WrongDimension { expected, actual } => {
                write!(
                    f,
                    "embedding vector dimension {actual} != expected {expected}"
                )
            }
            Self::NonFinite { index } => {
                write!(f, "embedding vector value at index {index} is not finite")
            }
            Self::Overflow { index } => {
                write!(
                    f,
                    "embedding vector value at index {index} overflows f32 storage"
                )
            }
            Self::MixedGeneration {
                query_generation,
                index_generation,
            } => write!(
                f,
                "query generation '{query_generation}' does not match serving index '{index_generation}'"
            ),
            Self::QualityProbeFailed {
                expected_entity_id,
                rank,
            } => write!(
                f,
                "quality probe did not retrieve {expected_entity_id} (rank {rank:?})"
            ),
            Self::Incomplete { expected, actual } => {
                write!(
                    f,
                    "embedding generation incomplete: {actual}/{expected} rows"
                )
            }
            Self::NoActiveGeneration => write!(f, "no active embedding generation"),
            Self::RollbackWindowClosed => write!(f, "embedding rollback window is closed"),
        }
    }
}

impl std::error::Error for EmbeddingValidationError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingGenerationV1 {
    pub generation_id: String,
    pub spec: EmbeddingGenerationSpecV1,
    pub table_name: String,
    pub status: EmbeddingGenerationStatusV1,
    pub created_at_micros: i64,
    pub activated_at_micros: Option<i64>,
    pub retired_at_micros: Option<i64>,
    pub rollback_until_micros: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingIndexStateV1 {
    pub project_id: String,
    pub active_generation_id: String,
    pub previous_generation_id: Option<String>,
    pub rollback_until_micros: Option<i64>,
    pub updated_at_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingQualityProbeV1 {
    pub query: String,
    pub expected_entity_id: String,
    pub max_rank: usize,
}

/// A query vector tagged with the generation that produced it. Index search
/// refuses a tag/dimension mismatch before any vec0 comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingQueryVectorV1 {
    pub generation_id: String,
    pub dimension: usize,
    pub values: Vec<f64>,
}

impl EmbeddingQueryVectorV1 {
    pub fn new(
        generation_id: impl Into<String>,
        dimension: usize,
        values: Vec<f64>,
    ) -> Result<Self, EmbeddingValidationError> {
        let generation_id = generation_id.into();
        if generation_id.trim().is_empty() {
            return Err(EmbeddingValidationError::NoActiveGeneration);
        }
        validate_embedding_vector(&values, dimension)?;
        Ok(Self {
            generation_id,
            dimension,
            values,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RebuildEmbeddingGenerationInputV1 {
    pub project_id: String,
    pub spec: EmbeddingGenerationSpecV1,
    /// Optional retrieval-quality gate. Activation only succeeds when the
    /// expected entity is returned within `max_rank`.
    pub quality_probe: Option<EmbeddingQualityProbeV1>,
    /// Rollback window kept after activation.
    pub rollback_window_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RebuildEmbeddingGenerationOutputV1 {
    pub generation_id: String,
    pub indexed_records: usize,
    pub expected_records: usize,
    pub activated: bool,
    pub quality_passed: bool,
    pub active_generation_id: Option<String>,
    pub previous_generation_id: Option<String>,
    pub rollback_until_micros: Option<i64>,
    pub status: EmbeddingGenerationStatusV1,
    pub retry_from_entity_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingRollbackOutputV1 {
    pub project_id: String,
    pub active_generation_id: String,
    pub retired_generation_id: String,
}

/// Reject empty, wrong-dimension, non-finite, or f32-overflowing vectors before
/// any storage/query comparison.
pub fn validate_embedding_vector(
    values: &[f64],
    expected_dimension: usize,
) -> Result<(), EmbeddingValidationError> {
    if values.is_empty() {
        return Err(EmbeddingValidationError::EmptyVector);
    }
    if values.len() != expected_dimension {
        return Err(EmbeddingValidationError::WrongDimension {
            expected: expected_dimension,
            actual: values.len(),
        });
    }
    for (index, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(EmbeddingValidationError::NonFinite { index });
        }
        if !(*value as f32).is_finite() {
            return Err(EmbeddingValidationError::Overflow { index });
        }
    }
    Ok(())
}

pub fn normalize_embedding(
    mut values: Vec<f64>,
    normalization: EmbeddingNormalizationV1,
) -> Vec<f64> {
    if normalization == EmbeddingNormalizationV1::None {
        return values;
    }
    let magnitude = values.iter().map(|value| value * value).sum::<f64>().sqrt();
    if magnitude == 0.0 || !magnitude.is_finite() {
        return values;
    }
    for value in &mut values {
        *value /= magnitude;
    }
    values
}

/// Deterministic token-hash embedding for any positive dimension. The legacy
/// 64-dimension path preserves the original `token-hash-v1` arithmetic.
pub fn token_hash_embedding(input: &str, dimensions: usize) -> Vec<f64> {
    let dimensions = dimensions.max(1);
    let mut vector = vec![0.0; dimensions];
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for token in input
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(|token| token.trim().to_lowercase())
        .filter(|token| token.len() >= 2)
    {
        *counts.entry(token).or_insert(0) += 1;
    }
    for (token, count) in counts {
        let mut hash: usize = 2166136261;
        for byte in token.bytes() {
            hash ^= usize::from(byte);
            hash = hash.wrapping_mul(16777619);
        }
        vector[hash % dimensions] += count as f64;
    }
    normalize_embedding(vector, EmbeddingNormalizationV1::L2)
}

pub fn embedding_generation_table_name(generation_id: &str) -> String {
    generation_table_name(generation_id)
}

fn generation_table_name(generation_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(generation_id.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("vec_embedding_gen_{}", &digest[..24])
}

pub fn cosine_similarity(left: &[f64], right: &[f64]) -> f64 {
    if left.is_empty() || left.len() != right.len() {
        return 0.0;
    }
    left.iter().zip(right).map(|(l, r)| l * r).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_hash_supports_fixture_dimensions_and_legacy_vector_is_stable() {
        for dimension in [64, 384, 1536] {
            let vector = token_hash_embedding("loyal border regiment", dimension);
            assert_eq!(vector.len(), dimension);
            validate_embedding_vector(&vector, dimension).expect("valid fixture vector");
            assert!((vector.iter().map(|v| v * v).sum::<f64>() - 1.0).abs() < 1e-9);
        }
        assert_eq!(
            token_hash_embedding("loyal border regiment", 64),
            token_hash_embedding("loyal border regiment", 64)
        );
    }

    #[test]
    fn wrong_dimension_non_finite_and_empty_vectors_are_rejected() {
        assert!(matches!(
            validate_embedding_vector(&[0.1, 0.2], 3),
            Err(EmbeddingValidationError::WrongDimension { .. })
        ));
        assert!(matches!(
            validate_embedding_vector(&[], 3),
            Err(EmbeddingValidationError::EmptyVector)
        ));
        assert!(matches!(
            validate_embedding_vector(&[0.0, f64::NAN], 2),
            Err(EmbeddingValidationError::NonFinite { index: 1 })
        ));
        assert!(matches!(
            validate_embedding_vector(&[f64::INFINITY, 0.0], 2),
            Err(EmbeddingValidationError::NonFinite { index: 0 })
        ));
        assert!(matches!(
            validate_embedding_vector(&[f64::MAX, 0.0], 2),
            Err(EmbeddingValidationError::Overflow { index: 0 })
        ));
    }

    #[test]
    fn generation_identity_includes_provider_model_dimension_and_version() {
        let a = EmbeddingGenerationSpecV1::token_hash_legacy();
        assert_eq!(a.generation_id(), LEGACY_TOKEN_HASH_GENERATION_ID);
        assert!(a.table_name().starts_with("vec_embedding_gen_"));
        let b = EmbeddingGenerationSpecV1 {
            dimension: 384,
            ..a.clone()
        };
        assert_ne!(a.generation_id(), b.generation_id());
        assert_ne!(a.table_name(), b.table_name());
        let c = EmbeddingGenerationSpecV1 {
            model: "other".into(),
            ..b.clone()
        };
        assert_ne!(b.generation_id(), c.generation_id());
    }

    #[test]
    fn query_vectors_are_tagged_and_validated() {
        let query = EmbeddingQueryVectorV1::new("generation:a", 3, vec![1.0, 0.0, 0.0])
            .expect("valid tagged query");
        assert_eq!(query.generation_id, "generation:a");
        assert!(EmbeddingQueryVectorV1::new("generation:a", 2, vec![1.0]).is_err());
        assert!(EmbeddingQueryVectorV1::new("generation:a", 2, vec![f64::NAN, 0.0]).is_err());
        assert!(EmbeddingQueryVectorV1::new("", 2, vec![1.0, 0.0]).is_err());
    }

    #[test]
    fn dimension_bounds_are_enforced() {
        let mut spec = EmbeddingGenerationSpecV1::token_hash_legacy();
        spec.dimension = 0;
        assert!(spec.validate().is_err());
        spec.dimension = MAX_EMBEDDING_DIMENSION + 1;
        assert!(spec.validate().is_err());
        spec.dimension = 1536;
        assert!(spec.validate().is_ok());
    }
}
