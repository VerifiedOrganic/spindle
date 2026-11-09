use super::*;
use crate::sqlite::repository::legacy_claim;
use spindle_core::canon::*;

impl SqliteSpindleService {
    pub async fn batch_set_character_voice_profiles(
        &self,
        input: BatchSetCharacterVoiceProfilesInput,
    ) -> Result<BatchSetCharacterVoiceProfilesOutput> {
        let mut profiles = Vec::with_capacity(input.items.len());
        for item in input.items {
            let updated = self
                .set_character_voice_profile(SetCharacterVoiceProfileInput {
                    project_id: input.project_id.clone(),
                    character_id: item.character_id,
                    branch_id: input.branch_id.clone(),
                    profile: item.profile,
                })
                .await?;
            profiles.push(updated);
        }
        let updated = profiles.len();
        Ok(BatchSetCharacterVoiceProfilesOutput { profiles, updated })
    }

    pub async fn record_knowledge(
        &self,
        input: RecordKnowledgeInput,
    ) -> Result<RecordKnowledgeOutput> {
        let character = self.repository.get_character(&input.character_id).await?;
        if character.project_id != input.project_id {
            anyhow::bail!("character does not belong to the requested project");
        }
        let branch_id = match input.branch_id {
            Some(b) => b,
            None => {
                self.repository
                    .active_branch_id_public(&input.project_id)
                    .await?
            }
        };
        // Reveal path (design §2.3.1): a `secret_of_fact_id` link must point at
        // an existing canonical fact that is actually marked secret, or the
        // "reveal" would grant circle membership in a non-secret. Validate at
        // the boundary and surface a clear domain error otherwise.
        if let Some(fact_id) = input.secret_of_fact_id.as_deref() {
            let linked = self
                .repository
                .get_canonical_fact(fact_id)
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "secret_of_fact_id '{fact_id}' does not reference an existing canonical fact"
                    )
                })?;
            if !linked.secret {
                anyhow::bail!(
                    "secret_of_fact_id '{fact_id}' references a canonical fact that is not marked secret"
                );
            }
        }
        let normalized_fact = spindle_core::models::normalize_name(&input.fact);
        let project_id = input.project_id.clone();
        let character_id = input.character_id.clone();
        let fact_text = input.fact.clone();
        let source_summary = input.source_summary.clone();
        let learned_at = input.learned_at.clone();
        let confidence = input.confidence;
        let tags = input.tags.clone();
        let reader_visible = input.reader_visible;
        let secret_of_fact_id = input.secret_of_fact_id.clone();
        let tx_branch_id = branch_id.clone();
        let mirror_revision_id = format!(
            "legacy:knowledge:{}:{}:{}",
            branch_id,
            input.character_id,
            spindle_core::models::normalize_name(&input.fact)
        );
        let mirror_placement = learned_at.clone().unwrap_or(StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: None,
            note: None,
        });
        let mirror_secret = secret_of_fact_id.is_some();

        let (fact, knows) = self
            .repository
            .with_transaction(move |tx| {
                let mirror_character_id = character_id.clone();
                let fact = tx.upsert_knowledge_fact(UpsertKnowledgeFactParams {
                    project_id: project_id.clone(),
                    branch_id: tx_branch_id.clone(),
                    character_id: character_id.clone(),
                    fact: fact_text,
                    normalized_fact,
                    source_summary: source_summary.clone(),
                    learned_at: learned_at.clone(),
                    confidence,
                    tags,
                    reader_visible,
                    source_import_session_id: None,
                    secret_of_fact_id: secret_of_fact_id.clone(),
                })?;
                let knows = tx.upsert_knows(UpsertKnowsParams {
                    project_id: project_id.clone(),
                    branch_id: tx_branch_id.clone(),
                    character_id,
                    knowledge_fact_id: fact.id.clone(),
                    source_summary: Some(source_summary),
                    learned_at,
                    confidence,
                    reader_visible,
                    source_import_session_id: None,
                })?;
                let assertion = if mirror_secret {
                    ClaimAssertion::Knowledge {
                        operation: KnowledgeOperation::Disclose,
                        proposition: ClaimEntity::Resolved {
                            id: fact.id.clone(),
                        },
                        audience: vec![ClaimEntity::Resolved {
                            id: mirror_character_id.clone(),
                        }],
                        source: None,
                    }
                } else {
                    ClaimAssertion::Knowledge {
                        operation: KnowledgeOperation::Learn,
                        proposition: ClaimEntity::Resolved {
                            id: fact.id.clone(),
                        },
                        audience: Vec::new(),
                        source: None,
                    }
                };
                let mirror_claim = legacy_claim(
                    &fact.project_id,
                    &tx_branch_id,
                    &mirror_revision_id,
                    None,
                    &mirror_placement,
                    ClaimEntity::Resolved {
                        id: mirror_character_id,
                    },
                    assertion,
                    None,
                    0,
                );
                let mirrored = tx.mirror_canon_claims(
                    &fact.project_id,
                    &tx_branch_id,
                    &mirror_revision_id,
                    None,
                    vec![mirror_claim],
                )?;
                if mirrored.is_some() {
                    let knows_pk = format!("{}|{}|{}", knows.branch_id, knows.in_id, knows.out_id);
                    tx.mark_canon_source_rows_mapped(
                        &fact.project_id,
                        &tx_branch_id,
                        vec![
                            ("knowledge_fact".to_string(), fact.id.clone()),
                            ("knows".to_string(), knows_pk),
                        ],
                    )?;
                }
                Ok((fact, Some(knows)))
            })
            .await?;
        let knows_edge_id =
            knows.map(|k| format!("knows:{}:{}:{}", k.branch_id, k.in_id, k.out_id));

        Ok(RecordKnowledgeOutput {
            fact: spindle_core::models::KnowledgeFactSummary {
                knowledge_fact_id: fact.id,
                branch_id: fact.branch_id,
                character_id: fact.character_id,
                fact: fact.fact,
                source_summary: fact.source_summary,
                learned_at: fact.learned_at.map(|p| StoryPlacement {
                    book_number: p.book_number,
                    chapter_number: p.chapter_number,
                    scene_order: p.scene_order,
                    note: p.note,
                }),
                confidence: fact.confidence,
                tags: fact.tags,
                reader_visible: fact.reader_visible,
            },
            knows_edge_id,
        })
    }

    pub async fn record_note(&self, input: RecordNoteInput) -> Result<RecordNoteOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = match input.branch_id {
            Some(b) => b,
            None => project
                .active_branch_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?,
        };
        let summary = input.note.lines().next().unwrap_or("").to_string();
        let activity = self
            .repository
            .append_session_activity(AppendSessionActivityParams {
                project_id: input.project_id,
                branch_id: branch_id.clone(),
                kind: "note".to_string(),
                subject_table: None,
                subject_id: None,
                summary: summary.clone(),
                details_json: Some(serde_json::json!({ "note": input.note })),
            })
            .await?;
        let created_at = chrono::DateTime::<chrono::Utc>::from_timestamp_micros(
            activity.created_at.timestamp_micros(),
        )
        .unwrap_or_default();
        Ok(RecordNoteOutput {
            activity_id: activity.id,
            branch_id,
            kind: "note".to_string(),
            summary,
            created_at: created_at.to_rfc3339(),
        })
    }

    pub async fn update_writer_position(
        &self,
        input: UpdateWriterPositionInput,
    ) -> Result<spindle_core::models::WriterPosition> {
        let position = self
            .repository
            .upsert_writer_position(UpsertWriterPositionParams {
                project_id: input.project_id,
                branch_id: input.branch_id,
                book_id: input.book_id,
                chapter_id: input.chapter_id,
                scene_id: input.scene_id,
                intent: input.intent,
                next_focus: input.next_focus,
                updated_by: "spindle-mcp".to_string(),
                updated_at: chrono::Utc::now(),
            })
            .await?;
        Ok(spindle_core::models::WriterPosition {
            project_id: position.project_id,
            branch_id: position.branch_id,
            book_id: position.book_id,
            chapter_id: position.chapter_id,
            scene_id: position.scene_id,
            intent: position.intent,
            next_focus: position.next_focus,
            updated_by: position.updated_by,
        })
    }
}
