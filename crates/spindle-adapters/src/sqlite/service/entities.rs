use super::*;
use crate::sqlite::repository::{legacy_claim, legacy_snapshot_claim};
use spindle_core::canon::*;
use spindle_core::models::CharacterStateAnchor;

impl SqliteSpindleService {
    // =========================================================================
    // Entity creation tools
    // =========================================================================

    pub async fn create_character(
        &self,
        input: CreateCharacterInput,
    ) -> Result<CreateCharacterOutput> {
        self.repository.get_project(&input.project_id).await?;
        let (character, voice_profile, emotional_profile, state) =
            self.repository.create_character(&input).await?;

        // Mirror the SurrealDB service: clear voice_drift validator findings
        // on the active branch when a new character lands. Search-embedding
        // refresh happens here too so the new character is searchable.
        let active_branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        self.resolve_phase_four_caches(
            &input.project_id,
            &active_branch_id,
            &[PhaseFourCacheId::VoiceDrift],
        )
        .await?;
        let document = character_search_document(&character);
        self.repository
            .refresh_search_embedding_for_entity(
                &input.project_id,
                &active_branch_id,
                &character.id,
                &document,
            )
            .await?;

        Ok(CreateCharacterOutput {
            character_id: character.id,
            voice_profile_id: voice_profile.id,
            emotional_profile_id: emotional_profile.id,
            state_id: state.id,
        })
    }

    pub async fn create_location(
        &self,
        input: CreateLocationInput,
    ) -> Result<CreateLocationOutput> {
        self.repository.get_project(&input.project_id).await?;
        let (location, world_state) = self.repository.create_location(&input).await?;

        let active_branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        let document = SearchDocument {
            entity_table: "location".into(),
            title: location.name.clone(),
            excerpt: location.summary.clone(),
            content: format!("{}\n\n{}", location.name, location.summary),
        };
        self.repository
            .refresh_search_embedding_for_entity(
                &input.project_id,
                &active_branch_id,
                &location.id,
                &document,
            )
            .await?;

        Ok(CreateLocationOutput {
            location_id: location.id,
            world_state_id: world_state.id,
        })
    }

    pub async fn create_relationship(
        &self,
        input: CreateRelationshipInput,
    ) -> Result<CreateRelationshipOutput> {
        let source_character = self.repository.get_character(&input.character_a_id).await?;
        let target_character = self.repository.get_character(&input.character_b_id).await?;
        if source_character.project_id != target_character.project_id {
            anyhow::bail!("relationship characters must belong to the same project");
        }
        let branch_id = self
            .repository
            .active_branch_id_public(&source_character.project_id)
            .await?;
        let rel = self
            .repository
            .create_relationship(&branch_id, &input)
            .await?;
        // The SQLite schema's relates_to has no surrogate id — the composite
        // key (branch_id, in_id, out_id) is the natural identifier. Encode
        // it into the output's relationship_id field so callers can roundtrip.
        Ok(CreateRelationshipOutput {
            relationship_id: format!("relates_to:{}:{}:{}", rel.branch_id, rel.in_id, rel.out_id),
        })
    }

    /// Commit a character state snapshot at the moment of a scene, or at the
    /// story-start/backstory anchor when no scene exists yet.
    /// Merges the input patch on top of the most recent character_state at
    /// or before the target placement, then appends a new state row + a
    /// progression event recording the change.
    pub async fn commit_character_state(
        &self,
        input: CommitCharacterStateInput,
    ) -> Result<CommitCharacterStateOutput> {
        let character = self.repository.get_character(&input.character_id).await?;
        let scene = match (input.scene_id.as_deref(), input.anchor) {
            (Some(scene_id), None) => Some(self.repository.get_scene(scene_id).await?),
            (None, Some(CharacterStateAnchor::StoryStart)) => None,
            (Some(_), Some(_)) => {
                anyhow::bail!("commit_character_state accepts scene_id or anchor, not both")
            }
            (None, None) => {
                anyhow::bail!("commit_character_state requires scene_id or anchor")
            }
        };
        if let Some(scene) = &scene
            && scene.project_id != character.project_id
        {
            anyhow::bail!("scene and character must belong to the same project");
        }

        let story_start = scene.is_none();
        let scene_id = scene.as_ref().map(|scene| scene.id.clone());
        let commit_branch_id = match &scene {
            Some(scene) => scene.branch_id.clone(),
            None => {
                self.repository
                    .active_branch_id_public(&character.project_id)
                    .await?
            }
        };
        let book_number = scene.as_ref().map_or(1, |scene| scene.book_number);
        let chapter_number = scene.as_ref().map_or(1, |scene| scene.chapter_number);
        let scene_order = scene.as_ref().map_or(0, |scene| scene.scene_order);
        let progression_delta = serde_json::to_value(&input.changes)?;
        let changes = input.changes;
        let mirror_statuses = changes.status.clone();
        let mirror_emotional_state = changes.emotional_state.clone();
        let mirror_revision_id = match &scene_id {
            Some(scene_id) => format!(
                "legacy:character_state:{}:{}:{}:{}:{}",
                scene_id, character.id, book_number, chapter_number, scene_order
            ),
            None => format!("legacy:character_state:story_start:{}", character.id),
        };
        let mirror_placement = StoryPlacement {
            book_number,
            chapter_number,
            scene_order: Some(scene_order),
            note: story_start.then(|| "story start".to_string()),
        };

        let state_id = self
            .repository
            .with_transaction(move |tx| {
                let target_branch_id = match &scene_id {
                    Some(_) => {
                        tx.resolve_effective_scene_branch(&character.project_id, &commit_branch_id)?
                    }
                    None => commit_branch_id.clone(),
                };
                // Scene commits include the state at the scene itself. A
                // story-start anchor must not absorb a later scene-1 snapshot,
                // so its inclusive cutoff is exactly its own placement.
                let cutoff_order = if story_start {
                    scene_order
                } else {
                    scene_order + 1
                };
                let latest_state = tx.resolve_character_state_for_branch(
                    &target_branch_id,
                    &character.id,
                    book_number,
                    chapter_number,
                    cutoff_order,
                )?;
                let merged = CharacterStatePatch {
                    emotional_state: if changes.emotional_state.is_empty() {
                        latest_state
                            .as_ref()
                            .map(|s| s.emotional_state.clone())
                            .unwrap_or_default()
                    } else {
                        changes.emotional_state
                    },
                    goals: Some(changes.goals.unwrap_or_else(|| {
                        latest_state
                            .as_ref()
                            .map(|s| s.goals.clone())
                            .unwrap_or_default()
                    })),
                    status: Some(changes.status.unwrap_or_else(|| {
                        latest_state
                            .as_ref()
                            .map(|s| s.status.clone())
                            .unwrap_or_default()
                    })),
                    notes: Some(changes.notes.unwrap_or_else(|| {
                        latest_state
                            .as_ref()
                            .map(|s| s.notes.clone())
                            .unwrap_or_default()
                    })),
                    source_summary: changes
                        .source_summary
                        .or_else(|| latest_state.as_ref().and_then(|s| s.source_summary.clone()))
                        .or_else(|| {
                            Some(if story_start {
                                format!("story-start update for {}", character.name)
                            } else {
                                format!("scene update for {}", character.name)
                            })
                        }),
                };

                let state = tx.append_character_state(AppendCharacterStateParams {
                    project_id: character.project_id.clone(),
                    branch_id: target_branch_id.clone(),
                    character_id: character.id.clone(),
                    scene_id: scene_id.clone(),
                    book_number,
                    chapter_number,
                    scene_order,
                    patch: merged,
                })?;

                let progression = tx.append_progression_event(AppendProgressionEventParams {
                    project_id: character.project_id.clone(),
                    branch_id: target_branch_id.clone(),
                    subject_table: "character".to_string(),
                    subject_id: character.id.clone(),
                    overlay_id: None,
                    kind: "character_state_commit".to_string(),
                    delta_json: progression_delta,
                    source_scene_id: scene_id.clone(),
                    placement: Some(StoryPlacement {
                        book_number,
                        chapter_number,
                        scene_order: Some(scene_order),
                        note: story_start.then(|| "story start".to_string()),
                    }),
                    created_at: chrono::Utc::now(),
                })?;

                let mut mirror_claims = Vec::new();
                let snapshot_index = mirror_claims.len();
                mirror_claims.push(legacy_snapshot_claim(
                    &character.project_id,
                    &target_branch_id,
                    &mirror_revision_id,
                    scene_id.as_deref(),
                    &mirror_placement,
                    &character.id,
                    format!("legacy.character_state.snapshot.{}", state.id),
                    &state,
                    Some(0),
                    snapshot_index,
                )?);
                if let Some(statuses) = &mirror_statuses {
                    for status in statuses {
                        let assertion = match status.trim().to_ascii_lowercase().as_str() {
                            "dead" | "deceased" | "status:deceased" => ClaimAssertion::LifeStatus {
                                status: LifeStatus::Dead,
                            },
                            "alive" | "status:alive" => ClaimAssertion::LifeStatus {
                                status: LifeStatus::Alive,
                            },
                            _ => ClaimAssertion::Attribute {
                                operation: AttributeOperation::Add,
                                key: "status".into(),
                                value: ClaimScalar::Text {
                                    value: status.clone(),
                                },
                                nature: AttributeNature::Unknown,
                            },
                        };
                        let index = mirror_claims.len();
                        mirror_claims.push(legacy_claim(
                            &character.project_id,
                            &target_branch_id,
                            &mirror_revision_id,
                            scene_id.as_deref(),
                            &mirror_placement,
                            ClaimEntity::Resolved {
                                id: character.id.clone(),
                            },
                            assertion,
                            Some(0),
                            index,
                        ));
                    }
                }
                for (key, value) in &mirror_emotional_state {
                    let text = match value {
                        serde_json::Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    let index = mirror_claims.len();
                    mirror_claims.push(legacy_claim(
                        &character.project_id,
                        &target_branch_id,
                        &mirror_revision_id,
                        scene_id.as_deref(),
                        &mirror_placement,
                        ClaimEntity::Resolved {
                            id: character.id.clone(),
                        },
                        ClaimAssertion::Attribute {
                            operation: AttributeOperation::Set,
                            key: format!("emotional.{key}"),
                            value: ClaimScalar::Text { value: text },
                            nature: AttributeNature::Evolving,
                        },
                        Some(0),
                        index,
                    ));
                }
                let mirrored = tx.mirror_canon_claims(
                    &character.project_id,
                    &target_branch_id,
                    &mirror_revision_id,
                    scene_id.as_deref(),
                    mirror_claims,
                )?;
                if mirrored.is_some() {
                    tx.mark_canon_source_rows_mapped(
                        &character.project_id,
                        &target_branch_id,
                        vec![
                            ("character_state".to_string(), state.id.clone()),
                            ("progression_event".to_string(), progression.id.clone()),
                        ],
                    )?;
                }

                Ok(state.id)
            })
            .await?;

        Ok(CommitCharacterStateOutput { state_id })
    }

    /// Update a relationship's trust + tension by deltas, and/or correct its
    /// label/dynamics. A scene_id keeps the scene-scoped delta behavior; when
    /// it is omitted, only label/dynamics are written and the relationship's
    /// trust/tension are left untouched.
    pub async fn update_relationship(
        &self,
        input: UpdateRelationshipInput,
    ) -> Result<UpdateRelationshipOutput> {
        self.update_relationship_with_source(input, "scene").await
    }

    pub(crate) async fn update_relationship_with_source(
        &self,
        input: UpdateRelationshipInput,
        source: &str,
    ) -> Result<UpdateRelationshipOutput> {
        let source_character = self.repository.get_character(&input.character_a_id).await?;
        let target_character = self.repository.get_character(&input.character_b_id).await?;
        if source_character.project_id != target_character.project_id {
            anyhow::bail!("relationship characters must belong to the same project");
        }

        let scene = match input.scene_id.as_deref() {
            Some(scene_id) => {
                let scene = self.repository.get_scene(scene_id).await?;
                if scene.project_id != source_character.project_id {
                    anyhow::bail!("scene does not belong to the same project as the characters");
                }
                Some(scene)
            }
            None => None,
        };

        let input = normalize_relationship_update(input, scene.is_some())?;
        let project_id = source_character.project_id.clone();
        let base_branch_id = match &scene {
            Some(scene) => scene.branch_id.clone(),
            None => self.repository.get_active_branch(&project_id).await?.id,
        };

        let relationship = self
            .repository
            .update_relationship_with_source(&base_branch_id, &input, source)
            .await?;

        Ok(UpdateRelationshipOutput {
            relationship_id: format!(
                "relates_to:{}:{}:{}",
                relationship.branch_id, relationship.in_id, relationship.out_id
            ),
            trust: relationship.trust,
            tension: relationship.tension,
            relationship_type: relationship.relationship_type,
            dynamics: relationship.dynamics,
        })
    }

    /// MCP search_bible: find Bible entities matching the query. Uses the
    /// vec0 kNN path for `Semantic` mode and FTS5 for `Exact` / `Fuzzy`.
    /// Returns the canonical SearchBibleOutput shape that MCP exposes.
    pub async fn search_bible(&self, input: SearchBibleInput) -> Result<SearchBibleOutput> {
        let mode = input.mode.unwrap_or(SearchBibleMode::Semantic);
        let limit = input.limit.unwrap_or(20).min(100);
        let project_id = input.project_id.clone();
        let branch_id = self.repository.active_branch_id_public(&project_id).await?;

        let results: Vec<SearchBibleResultItem> = match mode {
            SearchBibleMode::Semantic => {
                // Use the configured embedder to produce a query vector,
                // then ask the vec0 mirror for the k nearest matches.
                let query_embedding = self
                    .repository
                    .embed_query_for_active_generation(&project_id, &input.query)
                    .await?;
                let hits = self
                    .repository
                    .knn_search_embeddings(&project_id, Some(&branch_id), &query_embedding, limit)
                    .await?;
                hits.into_iter()
                    .map(|(se, distance)| SearchBibleResultItem {
                        entity_type: se.entity_table,
                        entity_id: se.entity_id,
                        title: se.title,
                        excerpt: se.excerpt,
                        // Lower distance = more similar; report as a score where
                        // higher is better, in the [0, 1] range that callers
                        // typically display.
                        score: 1.0 / (1.0 + distance),
                    })
                    .collect()
            }
            SearchBibleMode::Exact | SearchBibleMode::Fuzzy => {
                // FTS5: combine scene + character + location + world_rule hits.
                let mut combined = Vec::new();
                for (entity_type, hits) in [
                    (
                        "scene",
                        self.repository
                            .fts_search_scenes(&project_id, Some(&branch_id), &input.query, limit)
                            .await?,
                    ),
                    (
                        "character",
                        self.repository
                            .fts_search_characters(
                                &project_id,
                                Some(&branch_id),
                                &input.query,
                                limit,
                            )
                            .await?,
                    ),
                    (
                        "location",
                        self.repository
                            .fts_search_locations(
                                &project_id,
                                Some(&branch_id),
                                &input.query,
                                limit,
                            )
                            .await?,
                    ),
                    (
                        "world_rule",
                        self.repository
                            .fts_search_world_rules(
                                &project_id,
                                Some(&branch_id),
                                &input.query,
                                limit,
                            )
                            .await?,
                    ),
                ] {
                    for (id, rank, snippet) in hits {
                        combined.push(SearchBibleResultItem {
                            entity_type: entity_type.to_string(),
                            entity_id: id,
                            title: String::new(),
                            excerpt: snippet,
                            // FTS5 rank is negative (smaller = better). Flip sign
                            // for a "higher is better" score so callers can sort
                            // both modes the same way.
                            score: -rank,
                        });
                    }
                }
                combined.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                combined.truncate(limit);
                combined
            }
        };

        Ok(SearchBibleOutput {
            results,
            canonical_facts: Vec::new(),
            markdown: None,
        })
    }
}

/// Trim and validate a relationship update. Scene-scoped calls keep the
/// existing delta semantics; canon-only calls must name a label or dynamics
/// and may not carry trust/tension deltas without a scene to attribute them to.
fn normalize_relationship_update(
    mut input: UpdateRelationshipInput,
    has_scene: bool,
) -> Result<UpdateRelationshipInput> {
    input.reason = input.reason.trim().to_string();
    if input.reason.is_empty() {
        anyhow::bail!("update_relationship requires a non-empty reason");
    }
    if !has_scene {
        if input.trust_delta.is_some_and(|delta| delta != 0)
            || input.tension_delta.is_some_and(|delta| delta != 0)
        {
            anyhow::bail!("scene_id is required when updating trust or tension");
        }
        if input.relationship_type.is_none() && input.dynamics.is_none() {
            anyhow::bail!(
                "update_relationship requires relationship_type and/or dynamics when scene_id is omitted"
            );
        }
    }
    if let Some(relationship_type) = input.relationship_type.as_mut() {
        let trimmed = relationship_type.trim();
        if trimmed.is_empty() {
            anyhow::bail!("relationship_type must not be empty");
        }
        *relationship_type = trimmed.to_string();
    }
    if let Some(dynamics) = input.dynamics.as_mut() {
        let mut normalized = Vec::with_capacity(dynamics.len());
        for value in std::mem::take(dynamics) {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                anyhow::bail!("relationship dynamics entries must not be empty");
            }
            if !normalized
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
            {
                normalized.push(trimmed.to_string());
            }
        }
        *dynamics = normalized;
    }
    Ok(input)
}
