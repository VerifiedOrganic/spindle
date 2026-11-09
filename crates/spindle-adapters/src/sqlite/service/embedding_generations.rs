//! Configurable embedding index generation rebuild, activation and rollback
//! (P2 PR H).

use super::*;
use crate::ai::SearchDocument;
use crate::sqlite::row::timestamp_to_micros;
use spindle_core::embeddings::*;
use std::collections::BTreeSet;

impl SqliteSpindleService {
    pub async fn list_embedding_generations(&self) -> Result<Vec<EmbeddingGenerationV1>> {
        self.repository.list_embedding_generations().await
    }

    pub async fn embedding_index_state(
        &self,
        project_id: &str,
    ) -> Result<Option<EmbeddingIndexStateV1>> {
        self.repository.get_embedding_index_state(project_id).await
    }

    pub async fn active_embedding_generation(
        &self,
        project_id: &str,
    ) -> Result<EmbeddingGenerationV1> {
        self.repository
            .active_embedding_generation(project_id)
            .await
    }

    /// Rebuild a project's index into a new generation. Existing rows for the
    /// target generation are skipped, so restart/resume continues where the
    /// previous run stopped. The currently active generation keeps serving
    /// until the completeness and optional quality gates pass and the switch is
    /// atomic.
    pub async fn rebuild_embedding_generation(
        &self,
        input: RebuildEmbeddingGenerationInputV1,
    ) -> Result<RebuildEmbeddingGenerationOutputV1> {
        input.spec.validate()?;
        let previous = self
            .repository
            .active_embedding_generation(&input.project_id)
            .await?;
        let generation = self
            .repository
            .create_embedding_generation(input.spec.clone())
            .await?;
        if generation.status == EmbeddingGenerationStatusV1::Active {
            let state = self
                .repository
                .get_embedding_index_state(&input.project_id)
                .await?;
            let indexed_records = self
                .repository
                .embedding_row_count(&input.project_id, &generation.generation_id)
                .await?;
            return Ok(RebuildEmbeddingGenerationOutputV1 {
                generation_id: generation.generation_id,
                indexed_records,
                expected_records: 0,
                activated: true,
                quality_passed: true,
                active_generation_id: state
                    .as_ref()
                    .map(|state| state.active_generation_id.clone()),
                previous_generation_id: state
                    .as_ref()
                    .and_then(|state| state.previous_generation_id.clone()),
                rollback_until_micros: state.as_ref().and_then(|state| state.rollback_until_micros),
                status: EmbeddingGenerationStatusV1::Active,
                retry_from_entity_id: None,
            });
        }
        if generation.status == EmbeddingGenerationStatusV1::Failed {
            self.repository
                .resume_embedding_generation(&generation.generation_id)
                .await?;
        }

        let source = self
            .repository
            .list_embeddings_for_generation(&input.project_id, &previous.generation_id)
            .await?;
        let expected_records = source.len();
        let source_keys: BTreeSet<(String, String)> = source
            .iter()
            .map(|row| (row.entity_table.clone(), row.entity_id.clone()))
            .collect();
        let existing: BTreeSet<(String, String)> = self
            .repository
            .list_embeddings_for_generation(&input.project_id, &generation.generation_id)
            .await?
            .into_iter()
            .map(|row| (row.entity_table, row.entity_id))
            .collect();
        let mut indexed_records = existing.len();
        let mut retry_from_entity_id = None;

        for row in source {
            let key = (row.entity_table.clone(), row.entity_id.clone());
            if existing.contains(&key) {
                continue;
            }
            let document = SearchDocument {
                entity_table: row.entity_table.clone(),
                title: row.title.clone(),
                excerpt: row.excerpt.clone(),
                content: row.content.clone(),
            };
            let embedding = match self
                .repository
                .model_router()
                .embed_for_generation(&row.content, input.spec.provider, input.spec.dimension)
                .await
            {
                Ok(embedding) => embedding,
                Err(error) => {
                    self.repository
                        .fail_embedding_generation(&generation.generation_id)
                        .await?;
                    return Err(error);
                }
            };
            if let Err(error) = validate_embedding_vector(&embedding, input.spec.dimension) {
                self.repository
                    .fail_embedding_generation(&generation.generation_id)
                    .await?;
                return Err(anyhow::anyhow!(error.to_string()));
            }
            self.repository
                .upsert_embedding_document_for_generation(
                    &input.project_id,
                    row.branch_id.as_deref(),
                    &row.entity_id,
                    &document,
                    &generation,
                    &embedding,
                )
                .await?;
            indexed_records += 1;
            retry_from_entity_id = Some(row.entity_id);
        }

        self.repository
            .prune_embedding_generation(&input.project_id, &generation, &source_keys)
            .await?;
        let actual_records = self
            .repository
            .embedding_row_count(&input.project_id, &generation.generation_id)
            .await?;
        if actual_records < expected_records {
            return Ok(RebuildEmbeddingGenerationOutputV1 {
                generation_id: generation.generation_id,
                indexed_records,
                expected_records,
                activated: false,
                quality_passed: false,
                active_generation_id: Some(previous.generation_id),
                previous_generation_id: None,
                rollback_until_micros: None,
                status: EmbeddingGenerationStatusV1::Building,
                retry_from_entity_id,
            });
        }

        let mut quality_passed = true;
        if let Some(probe) = &input.quality_probe {
            quality_passed = self
                .embedding_quality_probe(&input, &generation, probe)
                .await?;
        }
        if !quality_passed {
            self.repository
                .fail_embedding_generation(&generation.generation_id)
                .await?;
            return Ok(RebuildEmbeddingGenerationOutputV1 {
                generation_id: generation.generation_id,
                indexed_records,
                expected_records,
                activated: false,
                quality_passed: false,
                active_generation_id: Some(previous.generation_id),
                previous_generation_id: None,
                rollback_until_micros: None,
                status: EmbeddingGenerationStatusV1::Failed,
                retry_from_entity_id: None,
            });
        }

        let now = timestamp_to_micros(chrono::Utc::now());
        let rollback_until = now.saturating_add(input.rollback_window_seconds.max(0) * 1_000_000);
        let state = self
            .repository
            .activate_embedding_generation(
                &input.project_id,
                &generation.generation_id,
                &previous.generation_id,
                rollback_until,
            )
            .await?;
        Ok(RebuildEmbeddingGenerationOutputV1 {
            generation_id: state.active_generation_id.clone(),
            indexed_records,
            expected_records,
            activated: true,
            quality_passed,
            active_generation_id: Some(state.active_generation_id),
            previous_generation_id: state.previous_generation_id,
            rollback_until_micros: state.rollback_until_micros,
            status: EmbeddingGenerationStatusV1::Active,
            retry_from_entity_id: None,
        })
    }

    async fn embedding_quality_probe(
        &self,
        input: &RebuildEmbeddingGenerationInputV1,
        generation: &EmbeddingGenerationV1,
        probe: &EmbeddingQualityProbeV1,
    ) -> Result<bool> {
        let query = self
            .repository
            .model_router()
            .embed_for_generation(&probe.query, input.spec.provider, input.spec.dimension)
            .await?;
        let rows = self
            .repository
            .list_embeddings_for_generation(&input.project_id, &generation.generation_id)
            .await?;
        let mut ranked: Vec<(&str, f64)> = rows
            .iter()
            .map(|row| {
                (
                    row.entity_id.as_str(),
                    cosine_similarity(&query, &row.embedding),
                )
            })
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        Ok(ranked
            .iter()
            .position(|(entity_id, _)| *entity_id == probe.expected_entity_id)
            .is_some_and(|rank| rank <= probe.max_rank))
    }

    /// Roll back inside the configured window. A failed rebuild never changes
    /// the active pointer, so this is only needed after a successful switch.
    pub async fn rollback_embedding_generation(
        &self,
        project_id: &str,
    ) -> Result<EmbeddingRollbackOutputV1> {
        self.repository
            .rollback_embedding_generation(project_id)
            .await
    }
}
