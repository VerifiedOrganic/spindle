use super::*;
use crate::sqlite::repository::legacy_claim;
use spindle_core::canon::*;

impl SqliteSpindleService {
    /// Commit a bundle of scene-aligned canon mutations (character states,
    /// canonical facts, relationship updates) and surface any world-rule
    /// hits. Mirrors `services/mod.rs:11234..~11490` in 705b835^.
    ///
    /// Behaviour notes for the SQLite port:
    /// - World-rule scanning is wired through `scan_world_rules` (a thin
    ///   wrapper over `spindle_core::world_rules::scan_prose_for_world_rules`).
    ///   `Likely`-severity hits block the commit unless
    ///   `accept_world_rule_risks=true`.
    /// - Per-character and per-fact failures are aggregated into the
    ///   response (not bailed) — matches the reference's structured-error
    ///   contract so MCP callers can surface partial successes.
    /// - When any character state succeeded, we run a branch-wide
    ///   `resolve_validator_findings_for_validator("voice_drift")` instead
    ///   of the SurrealDB `voice_profile_affected_scene_ids` fan-out
    ///   (same simplification as `set_character_voice_profile`).
    /// - When any canonical fact succeeded, the Surreal reference calls
    ///   `resolve_validator_findings_for_scenes(&[scene_id])` for
    ///   `canonical_fact_prose_drift`; we mirror that.
    ///
    /// - After all per-entry mutations land, the Phase-4 validator
    ///   registry is re-run for this scene (cache bypassed) and the
    ///   results populate `findings_summary`.
    pub async fn commit_scene_changes(
        &self,
        input: spindle_core::models::CommitSceneChangesInput,
    ) -> Result<spindle_core::models::CommitSceneChangesOutput> {
        use spindle_core::models::{
            CommitSceneCanonicalFactResult, CommitSceneChangesOutput,
            CommitSceneCharacterStateResult, CommitSceneRelationshipResult, WorldRuleSeverity,
        };

        let project_id = input.project_id.clone();
        let scene_id_input = input.scene_id.clone();
        let mut scene = self.repository.get_scene(&scene_id_input).await?;
        if scene.project_id != project_id {
            let error = anyhow::anyhow!("scene does not belong to the requested project");
            return Err(self
                .append_tool_error_activity(
                    &project_id,
                    &scene.branch_id,
                    "commit_scene_changes",
                    Some("scene"),
                    Some(&scene.id),
                    error,
                )
                .await);
        }

        // Resolve the effective target branch before validation so canon checks
        // (world rules, canonical-fact contradictions, retcon reachability) and
        // subsequent writes evaluate against the exact same branch view when
        // committing through an inherited ancestor scene.
        let target_branch_id = self
            .repository
            .resolve_effective_scene_branch(&project_id, &scene.branch_id)
            .await?;
        scene.branch_id = target_branch_id;

        let scene_id = scene.id.clone();
        let book_number = scene.book_number;
        let chapter_number = scene.chapter_number;

        let world_rule_hits = self
            .scan_world_rules(&project_id, &scene.branch_id, &scene.full_text)
            .await?;

        // Only Likely hits (pattern match + violation context in surrounding
        // prose) block the commit. Possible hits are returned in the response
        // for caller visibility but do not require an override.
        let blocking_hits: Vec<&spindle_core::models::WorldRuleHit> = world_rule_hits
            .iter()
            .filter(|hit| hit.severity == WorldRuleSeverity::Likely)
            .collect();

        if !input.accept_world_rule_risks && !blocking_hits.is_empty() {
            let hit_descriptions: Vec<String> = blocking_hits
                .iter()
                .map(|hit| {
                    format!(
                        "{} (likely) at byte {}-{}: {}",
                        hit.rule_id,
                        hit.byte_range.start,
                        hit.byte_range.end,
                        hit.surrounding_text.chars().take(80).collect::<String>(),
                    )
                })
                .collect();
            anyhow::bail!(
                "commit_scene_changes blocked by {} likely world rule violation(s); \
                 set accept_world_rule_risks=true to override: {}",
                blocking_hits.len(),
                hit_descriptions.join("; ")
            );
        }

        // ── Write-time continuity gate ────────────────────────────────
        // Detect canonical-fact contradictions (scope/validity-window aware,
        // including the prospective facts in this commit) and prose retcons
        // BEFORE any mutation, so unattended runs cannot silently bake in drift
        // the validators already detect.
        let continuity_gate = input
            .continuity_gate
            .unwrap_or(spindle_core::models::CommitContinuityGate::BlockErrors);
        let (blocking_continuity_findings, retcon_findings) = if continuity_gate
            == spindle_core::models::CommitContinuityGate::Off
        {
            (Vec::new(), Vec::new())
        } else {
            let active_facts = self
                .repository
                .list_active_canonical_facts_by_project_and_branch(&project_id, &scene.branch_id)
                .await?;
            let mut candidates: Vec<crate::format::ContradictionCandidate> = active_facts
                .iter()
                .map(crate::format::candidate_from_canonical_fact)
                .collect();
            for (idx, entry) in input.canonical_facts.iter().enumerate() {
                if let Some(candidate) = crate::format::candidate_from_commit_entry(
                    entry,
                    format!("(pending fact {idx})"),
                ) {
                    candidates.push(candidate);
                }
            }
            let mut findings: Vec<spindle_core::models::ConsistencyIssue> =
                    crate::format::detect_fact_contradictions(&candidates)
                        .into_iter()
                        .map(|contradiction| spindle_core::models::ConsistencyIssue {
                            severity: "error".to_string(),
                            check_type: "canonical_fact_consistency".to_string(),
                            message: format!(
                                "committing this scene would leave canonical fact '{}' with conflicting active values: {}",
                                contradiction.composite_key,
                                contradiction
                                    .values
                                    .iter()
                                    .map(|value| format!("'{value}'"))
                                    .collect::<Vec<_>>()
                                    .join(" vs ")
                            ),
                            entity_ids: contradiction.conflicting_ids,
                            suggested_action: Some(
                                "supersede the outdated fact (supersedes_fact_id) or set distinct validity windows".to_string(),
                            ),
                        })
                        .collect();
            let retcons = self.scan_retcon_findings(&project_id, &scene).await?;
            findings.extend(retcons.iter().map(retcon_finding_to_consistency_issue));
            (findings, retcons)
        };

        // Intra-scene temporal coherence — advisory only. Held in its own field
        // (never `blocking_continuity_findings`) and always `severity: "warning"`,
        // so a temporal finding can NEVER block a commit under any
        // `continuity_gate`, including BlockErrors. Skipped on Off to match the
        // retcon scan above.
        let temporal_findings =
            if continuity_gate == spindle_core::models::CommitContinuityGate::Off {
                Vec::new()
            } else {
                let scene_clock = self.repository.get_scene_clock(&scene.id).await?;
                scan_temporal_findings(&scene.id, &scene.full_text, scene_clock.as_ref())
            };

        if continuity_gate == spindle_core::models::CommitContinuityGate::BlockErrors
            && !input.accept_continuity_risks
        {
            let blocking: Vec<&spindle_core::models::ConsistencyIssue> =
                blocking_continuity_findings
                    .iter()
                    .filter(|issue| issue.severity == "error")
                    .collect();
            if !blocking.is_empty() {
                anyhow::bail!(
                    "commit_scene_changes blocked by {} continuity error(s); \
                     set accept_continuity_risks=true to override: {}",
                    blocking.len(),
                    blocking
                        .iter()
                        .map(|issue| issue.message.clone())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
        }

        let tx_project_id = project_id.clone();
        let tx_scene = scene.clone();
        let tx_character_states = input.character_states;
        let tx_canonical_facts = input.canonical_facts;
        let tx_relationship_updates = input.relationship_updates;
        let (character_states, canonical_facts, relationship_updates) = self
            .repository
            .with_transaction(move |tx| {
                // Use the branch already checked by the continuity gate.
                let target_branch_id = tx_scene.branch_id.clone();
                let mirror_revision_id = format!("legacy:commit_scene_changes:{}", tx_scene.id);
                let mirror_placement = StoryPlacement {
                    book_number: tx_scene.book_number,
                    chapter_number: tx_scene.chapter_number,
                    scene_order: Some(tx_scene.scene_order),
                    note: None,
                };
                let mut mirror_claims: Vec<spindle_core::canon::ClaimV1> = Vec::new();
                let mut mirror_source_keys: Vec<(String, String)> = Vec::new();
                let mut character_states = Vec::with_capacity(tx_character_states.len());
                for entry in tx_character_states {
                    let character = tx.get_character(&entry.character_id)?;
                    if character.project_id != tx_project_id {
                        anyhow::bail!(
                            "character '{}' does not belong to project '{}'",
                            entry.character_id,
                            tx_project_id
                        );
                    }
                    let mirror_statuses = entry.changes.status.clone();
                    let mirror_emotional_state = entry.changes.emotional_state.clone();
                    let mirror_character_id = character.id.clone();
                    let latest_state = tx.resolve_character_state_for_branch(
                        &target_branch_id,
                        &character.id,
                        tx_scene.book_number,
                        tx_scene.chapter_number,
                        tx_scene.scene_order + 1,
                    )?;
                    let progression_delta = serde_json::to_value(&entry.changes)?;
                    let changes = entry.changes;
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
                            .or_else(|| {
                                latest_state.as_ref().and_then(|s| s.source_summary.clone())
                            })
                            .or_else(|| Some(format!("scene update for {}", character.name))),
                    };
                    let state = tx.append_character_state(AppendCharacterStateParams {
                        project_id: tx_project_id.clone(),
                        branch_id: target_branch_id.clone(),
                        character_id: character.id.clone(),
                        scene_id: Some(tx_scene.id.clone()),
                        book_number: tx_scene.book_number,
                        chapter_number: tx_scene.chapter_number,
                        scene_order: tx_scene.scene_order,
                        patch: merged,
                    })?;
                    let progression =
                        tx.append_progression_event(AppendProgressionEventParams {
                            project_id: tx_project_id.clone(),
                            branch_id: target_branch_id.clone(),
                            subject_table: "character".to_string(),
                            subject_id: character.id.clone(),
                            overlay_id: None,
                            kind: "character_state_commit".to_string(),
                            delta_json: progression_delta,
                            source_scene_id: Some(tx_scene.id.clone()),
                            placement: Some(StoryPlacement {
                                book_number: tx_scene.book_number,
                                chapter_number: tx_scene.chapter_number,
                                scene_order: Some(tx_scene.scene_order),
                                note: None,
                            }),
                            created_at: chrono::Utc::now(),
                        })?;
                    let mirror_claim_count_before = mirror_claims.len();
                    if let Some(statuses) = &mirror_statuses {
                        for status in statuses {
                            let assertion = match status.trim().to_ascii_lowercase().as_str() {
                                "dead" | "deceased" | "status:deceased" => {
                                    ClaimAssertion::LifeStatus {
                                        status: LifeStatus::Dead,
                                    }
                                }
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
                                &tx_project_id,
                                &target_branch_id,
                                &mirror_revision_id,
                                Some(&tx_scene.id),
                                &mirror_placement,
                                ClaimEntity::Resolved {
                                    id: mirror_character_id.clone(),
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
                            &tx_project_id,
                            &target_branch_id,
                            &mirror_revision_id,
                            Some(&tx_scene.id),
                            &mirror_placement,
                            ClaimEntity::Resolved {
                                id: mirror_character_id.clone(),
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
                    if mirror_claims.len() > mirror_claim_count_before {
                        mirror_source_keys.push(("character_state".to_string(), state.id.clone()));
                        mirror_source_keys
                            .push(("progression_event".to_string(), progression.id.clone()));
                    }
                    character_states.push(CommitSceneCharacterStateResult {
                        character_id: entry.character_id,
                        state_id: Some(state.id),
                        error: None,
                    });
                }

                let mut canonical_facts = Vec::with_capacity(tx_canonical_facts.len());
                for entry in tx_canonical_facts {
                    let fact_type = entry
                        .fact_type
                        .clone()
                        .or_else(|| entry.predicate.clone())
                        .unwrap_or_else(|| "typed_fact".to_string());
                    let key = entry
                        .key
                        .clone()
                        .or_else(|| entry.predicate.clone())
                        .unwrap_or_else(|| "typed_fact".to_string());
                    let value_kind = entry.value_kind.clone().unwrap_or_else(|| {
                        if entry.value_number.is_some() {
                            "number".to_string()
                        } else if entry.value_json.is_some() {
                            "json".to_string()
                        } else {
                            "string".to_string()
                        }
                    });
                    let predicate = entry
                        .predicate
                        .clone()
                        .or_else(|| entry.key.clone())
                        .ok_or_else(|| anyhow::anyhow!("predicate or key is required"))?;
                    let subject_table = entry
                        .subject_table
                        .clone()
                        .unwrap_or_else(|| "project".to_string());
                    let scope = match entry.scope {
                        Some(CanonicalFactScope::Invariant) | None => "invariant",
                        Some(CanonicalFactScope::Evolving) => "evolving",
                        Some(CanonicalFactScope::Conditional) => "conditional",
                    }
                    .to_string();
                    let mirror_subject_id = entry
                        .subject_id
                        .clone()
                        .filter(|id| !id.trim().is_empty())
                        .unwrap_or_else(|| format!("project:{tx_project_id}"));
                    let mirror_predicate = predicate.clone();
                    let mirror_scalar = if let Some(number) =
                        entry.value_number.filter(|value| value.is_finite())
                    {
                        Some(ClaimScalar::Decimal {
                            value: number.to_string(),
                            unit: entry.value_unit.clone(),
                        })
                    } else if let Some(serde_json::Value::Bool(value)) = &entry.value_json {
                        Some(ClaimScalar::Boolean { value: *value })
                    } else {
                        entry
                            .value_text
                            .clone()
                            .or_else(|| entry.value.clone())
                            .filter(|value| !value.trim().is_empty())
                            .map(|value| ClaimScalar::Text { value })
                    };
                    let mirror_nature = match entry.scope {
                        Some(CanonicalFactScope::Invariant) => AttributeNature::Invariant,
                        Some(CanonicalFactScope::Evolving) => AttributeNature::Evolving,
                        Some(CanonicalFactScope::Conditional) => AttributeNature::Conditional,
                        None => AttributeNature::Unknown,
                    };
                    let fact = tx.create_canonical_fact(
                        crate::sqlite::repository::CreateCanonicalFactParams {
                            project_id: tx_project_id.clone(),
                            branch_id: target_branch_id.clone(),
                            scene_id: Some(tx_scene.id.clone()),
                            book_number,
                            chapter_number,
                            subject_table,
                            subject_id: entry.subject_id,
                            predicate,
                            value_kind,
                            value_text: entry.value_text.or(entry.value),
                            value_number: entry.value_number,
                            unit: entry.value_unit,
                            value_json: entry.value_json,
                            aliases: entry.aliases,
                            scope,
                            valid_from: entry.valid_from,
                            valid_until: entry.valid_until,
                            legacy_untyped: false,
                            secret: false,
                            concealment_note: None,
                        },
                    )?;
                    let superseded_fact_id = match entry.supersedes_fact_id {
                        Some(old) => {
                            tx.supersede_canonical_fact(&fact.branch_id, &old, &fact.id)?;
                            Some(old)
                        }
                        None => None,
                    };
                    let mirror_fact_added = mirror_scalar.is_some();
                    if let Some(value) = mirror_scalar {
                        let index = mirror_claims.len();
                        mirror_claims.push(legacy_claim(
                            &tx_project_id,
                            &target_branch_id,
                            &mirror_revision_id,
                            Some(&tx_scene.id),
                            &mirror_placement,
                            ClaimEntity::Resolved {
                                id: mirror_subject_id,
                            },
                            ClaimAssertion::Attribute {
                                operation: AttributeOperation::Set,
                                key: mirror_predicate,
                                value,
                                nature: mirror_nature,
                            },
                            Some(0),
                            index,
                        ));
                    }
                    if mirror_fact_added {
                        mirror_source_keys.push(("canonical_fact".to_string(), fact.id.clone()));
                    }
                    canonical_facts.push(CommitSceneCanonicalFactResult {
                        fact_type,
                        key,
                        canonical_fact_id: Some(fact.id),
                        superseded_fact_id,
                        error: None,
                    });
                }

                let incoming_pairs: Vec<(String, String)> = tx_relationship_updates
                    .iter()
                    .map(|entry| (entry.character_a_id.clone(), entry.character_b_id.clone()))
                    .collect();
                tx.reconcile_continuity_package_relationships(
                    &target_branch_id,
                    &tx_scene.id,
                    &incoming_pairs,
                )?;

                let mut relationship_updates = Vec::with_capacity(tx_relationship_updates.len());
                for entry in tx_relationship_updates {
                    let source_character = tx.get_character(&entry.character_a_id)?;
                    let target_character = tx.get_character(&entry.character_b_id)?;
                    if source_character.project_id != target_character.project_id {
                        anyhow::bail!("relationship characters must belong to the same project");
                    }
                    if tx_scene.project_id != source_character.project_id {
                        anyhow::bail!(
                            "scene does not belong to the same project as the characters"
                        );
                    }
                    let rel_input = spindle_core::models::UpdateRelationshipInput {
                        character_a_id: entry.character_a_id.clone(),
                        character_b_id: entry.character_b_id.clone(),
                        trust_delta: entry.trust_delta,
                        tension_delta: entry.tension_delta,
                        reason: entry.reason,
                        scene_id: tx_scene.id.clone(),
                    };
                    let rel = tx.update_relationship(
                        &target_branch_id,
                        &rel_input,
                        "continuity_package",
                    )?;
                    let index = mirror_claims.len();
                    mirror_claims.push(legacy_claim(
                        &tx_project_id,
                        &target_branch_id,
                        &format!(
                            "{mirror_revision_id}:relationship:{}:{}",
                            entry.character_a_id, entry.character_b_id
                        ),
                        Some(&tx_scene.id),
                        &mirror_placement,
                        ClaimEntity::Resolved {
                            id: entry.character_a_id.clone(),
                        },
                        ClaimAssertion::Relationship {
                            operation: RelationshipOperation::Delta,
                            other: ClaimEntity::Resolved {
                                id: entry.character_b_id.clone(),
                            },
                            trust: entry.trust_delta,
                            tension: entry.tension_delta,
                        },
                        Some(0),
                        index,
                    ));
                    mirror_source_keys.push((
                        "relates_to".to_string(),
                        format!("{}|{}|{}", rel.branch_id, rel.in_id, rel.out_id),
                    ));
                    mirror_source_keys.push((
                        "relationship_scene_delta".to_string(),
                        format!(
                            "{}|{}|{}|{}|{}",
                            rel.branch_id, rel.in_id, rel.out_id, tx_scene.id, "continuity_package"
                        ),
                    ));
                    relationship_updates.push(CommitSceneRelationshipResult {
                        character_a_id: entry.character_a_id,
                        character_b_id: entry.character_b_id,
                        relationship_id: Some(format!(
                            "relates_to:{}:{}:{}",
                            rel.branch_id, rel.in_id, rel.out_id
                        )),
                        trust: Some(rel.trust),
                        tension: Some(rel.tension),
                        error: None,
                    });
                }

                if !character_states.is_empty() {
                    tx.resolve_validator_findings_for_validator(
                        &tx_project_id,
                        &target_branch_id,
                        PhaseFourCacheId::VoiceDrift.as_str(),
                    )?;
                }
                if !canonical_facts.is_empty() {
                    tx.resolve_validator_findings_for_validator(
                        &tx_project_id,
                        &target_branch_id,
                        PhaseFourCacheId::CanonicalFactProseDrift.as_str(),
                    )?;
                    tx.resolve_validator_findings_for_scenes(
                        &target_branch_id,
                        std::slice::from_ref(&tx_scene.id),
                    )?;
                }

                let mirrored = tx.mirror_canon_claims(
                    &tx_project_id,
                    &target_branch_id,
                    &mirror_revision_id,
                    Some(&tx_scene.id),
                    mirror_claims,
                )?;
                if mirrored.is_some() {
                    tx.mark_canon_source_rows_mapped(
                        &tx_project_id,
                        &target_branch_id,
                        mirror_source_keys,
                    )?;
                }

                Ok((character_states, canonical_facts, relationship_updates))
            })
            .await?;

        // ── Phase-4 validator fan-out ───────────────────────────────
        // Re-runs the four Phase-4 validators against just the scene we
        // committed to so `findings_summary` reports the post-commit state.
        // Cache is intentionally bypassed (`use_cache = false`) because the
        // mutations above (character_state / canonical_fact / relationship)
        // may have changed the validator outcome for this scene.
        let phase_four_checks =
            requested_phase_four_validator_checks(&std::collections::BTreeSet::new());
        let findings = self
            .run_phase_four_validator_checks_for_scenes(
                &project_id,
                &scene.branch_id,
                std::slice::from_ref(&scene),
                &phase_four_checks,
                false,
            )
            .await?;
        let findings_summary = summarize_commit_scene_findings(&findings);

        let output = CommitSceneChangesOutput {
            scene_id: scene_id.clone(),
            character_states,
            canonical_facts,
            relationship_updates,
            world_rule_hits,
            findings_summary,
            blocking_continuity_findings,
            retcon_findings,
            temporal_findings,
        };

        // Best-effort session activity log: failure here doesn't fail the
        // overall commit. Matches the reference semantics.
        if let Err(error) = self
            .repository
            .append_session_activity(AppendSessionActivityParams {
                project_id: project_id.clone(),
                branch_id: scene.branch_id.clone(),
                kind: "scene_committed".to_string(),
                subject_table: Some("scene".to_string()),
                subject_id: Some(scene.id.clone()),
                summary: format!(
                    "Committed scene changes for scene {}.{}.{}.",
                    scene.book_number, scene.chapter_number, scene.scene_order
                ),
                details_json: Some(serde_json::json!({
                    "scene_id": output.scene_id,
                    "book_number": scene.book_number,
                    "chapter_number": scene.chapter_number,
                    "scene_order": scene.scene_order,
                    "character_states": output.character_states.len(),
                    "canonical_facts": output.canonical_facts.len(),
                    "relationship_updates": output.relationship_updates.len(),
                })),
            })
            .await
        {
            tracing::warn!("session activity log failed: {error}");
        }

        Ok(output)
    }

    /// Best-effort logger that records a tool-level failure in the
    /// session_activity stream and returns the original error so the
    /// caller can keep its error-propagation chain. Mirrors
    /// `append_tool_error_activity` from `services/mod.rs:350` in
    /// 705b835^.
    pub(super) async fn append_tool_error_activity(
        &self,
        project_id: &str,
        branch_id: &str,
        tool: &str,
        subject_table: Option<&str>,
        subject_id: Option<&str>,
        error: anyhow::Error,
    ) -> anyhow::Error {
        let summary = format!("tool {tool} failed: {error}");
        let _ = self
            .repository
            .append_session_activity(AppendSessionActivityParams {
                project_id: project_id.to_string(),
                branch_id: branch_id.to_string(),
                kind: format!("{tool}_failed"),
                subject_table: subject_table.map(str::to_string),
                subject_id: subject_id.map(str::to_string),
                summary,
                details_json: Some(serde_json::json!({
                    "tool": tool,
                    "error": error.to_string(),
                })),
            })
            .await;
        error
    }
}
