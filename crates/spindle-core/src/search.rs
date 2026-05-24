use std::collections::BTreeMap;

pub const EMBEDDING_VERSION: &str = "token-hash-v1";
const EMBEDDING_DIMENSIONS: usize = 64;

#[derive(Debug, Clone)]
pub struct SearchDocument {
    pub entity_table: String,
    pub title: String,
    pub excerpt: String,
    pub content: String,
}

pub fn embed_text(input: &str) -> Vec<f64> {
    let tokens = tokenize(input);
    let mut vector = vec![0.0; EMBEDDING_DIMENSIONS];
    if tokens.is_empty() {
        return vector;
    }

    for (token, count) in token_counts(tokens) {
        let slot = hash_token(&token) % EMBEDDING_DIMENSIONS;
        vector[slot] += count as f64;
    }

    normalize(vector)
}

pub fn cosine_similarity(left: &[f64], right: &[f64]) -> f64 {
    if left.is_empty() || right.is_empty() || left.len() != right.len() {
        return 0.0;
    }
    left.iter().zip(right).map(|(l, r)| l * r).sum()
}

pub fn tokenize(input: &str) -> Vec<String> {
    input
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_lowercase();
            if token.len() >= 2 { Some(token) } else { None }
        })
        .collect()
}

fn token_counts(tokens: Vec<String>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for token in tokens {
        *counts.entry(token).or_insert(0) += 1;
    }
    counts
}

fn hash_token(token: &str) -> usize {
    let mut value: usize = 2166136261;
    for byte in token.bytes() {
        value ^= usize::from(byte);
        value = value.wrapping_mul(16777619);
    }
    value
}

fn normalize(mut vector: Vec<f64>) -> Vec<f64> {
    let magnitude = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
    if magnitude == 0.0 {
        return vector;
    }
    for value in &mut vector {
        *value /= magnitude;
    }
    vector
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_prefers_related_queries() {
        let military = embed_text("loyal border regiment military garrison");
        let query = embed_text("military border soldiers");
        let romance = embed_text("forbidden romance ballroom dance");

        assert!(cosine_similarity(&military, &query) > cosine_similarity(&romance, &query));
    }
}
