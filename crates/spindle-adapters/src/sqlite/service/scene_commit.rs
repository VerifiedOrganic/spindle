use super::*;

mod prose;

use spindle_core::models::{
    CommitSceneAppliedDeltas, CommitSceneContinuityPackage, CommitSceneCoverage, CommitSceneCursor,
    CommitSceneFollowUp, CommitSceneFollowUpKind, CommitSceneInput, CommitSceneIssue,
    CommitSceneIssueSeverity, CommitSceneOutput, CommitSceneUndoSupport, ConsistencyIssue,
    DraftAuthorship, RelationshipUpdateEntry, SaveSceneDraftInput, SceneAuthorship,
    ScenePreparationFreshness, StoryPlacement,
};

/// Result of applying a typed continuity package inside the commit transaction.
#[derive(Debug, Default)]
struct AppliedContinuity {
    deltas: CommitSceneAppliedDeltas,
}

/// Apply the author-declared continuity package in the caller's transaction.
///
/// Mined canon proposals are deliberately absent: this path only applies the
/// author's explicit declarations and never auto-ratifies extraction output.
fn apply_continuity_in_tx(
    tx: &crate::sqlite::repository::CanonTransaction<'_>,
    project_id: &str,
    branch_id: &str,
    scene: &crate::sqlite::records::Scene,
    package: &CommitSceneContinuityPackage,
) -> Result<AppliedContinuity> {
    use spindle_core::models::{
        CanonicalFactScope, CharacterStatePatch, StoryPlacement, UpdateRelationshipInput,
    };

    let mut applied = AppliedContinuity::default();

    for entry in &package.character_states {
        let character = tx.get_character(&entry.character_id)?;
        if character.project_id != project_id {
            anyhow::bail!(
                "character '{}' does not belong to project '{}'",
                entry.character_id,
                project_id
            );
        }
        let latest_state = tx.resolve_character_state_for_branch(
            branch_id,
            &character.id,
            scene.book_number,
            scene.chapter_number,
            scene.scene_order + 1,
        )?;
        let progression_delta = serde_json::to_value(&entry.changes)?;
        let changes: CharacterStatePatch = entry.changes.clone().into();
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
                .or_else(|| latest_state.as_ref().and_then(|s| s.source_summary.clone())),
        };
        let state = tx.append_character_state(AppendCharacterStateParams {
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            character_id: character.id.clone(),
            scene_id: Some(scene.id.clone()),
            book_number: scene.book_number,
            chapter_number: scene.chapter_number,
            scene_order: scene.scene_order,
            patch: merged,
        })?;
        tx.append_progression_event(AppendProgressionEventParams {
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            subject_table: "character".to_string(),
            subject_id: character.id.clone(),
            overlay_id: None,
            kind: "character_state_commit".to_string(),
            delta_json: progression_delta,
            source_scene_id: Some(scene.id.clone()),
            placement: Some(StoryPlacement {
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: Some(scene.scene_order),
                note: None,
            }),
            created_at: chrono::Utc::now(),
        })?;
        applied.deltas.character_state_ids.push(state.id);
    }

    for entry in &package.canonical_facts {
        let entry: spindle_core::models::CanonicalFactEntry = entry.clone().into();
        let predicate = entry
            .predicate
            .clone()
            .or_else(|| entry.key.clone())
            .ok_or_else(|| anyhow::anyhow!("predicate or key is required"))?;
        let value_kind = entry.value_kind.clone().unwrap_or_else(|| {
            if entry.value_number.is_some() {
                "number".to_string()
            } else if entry.value_json.is_some() {
                "json".to_string()
            } else {
                "string".to_string()
            }
        });
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
        let fact =
            tx.create_canonical_fact(crate::sqlite::repository::CreateCanonicalFactParams {
                project_id: project_id.to_string(),
                branch_id: branch_id.to_string(),
                scene_id: Some(scene.id.clone()),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                subject_table,
                subject_id: entry.subject_id.clone(),
                predicate,
                value_kind,
                value_text: entry.value_text.clone().or_else(|| entry.value.clone()),
                value_number: entry.value_number,
                unit: entry.value_unit.clone(),
                value_json: entry.value_json.clone(),
                aliases: entry.aliases.clone(),
                scope,
                valid_from: entry.valid_from.clone(),
                valid_until: entry.valid_until.clone(),
                legacy_untyped: false,
                secret: false,
                concealment_note: None,
            })?;
        if let Some(old) = entry.supersedes_fact_id.as_deref() {
            tx.supersede_canonical_fact(&fact.branch_id, old, &fact.id)?;
        }
        applied.deltas.canonical_fact_ids.push(fact.id);
    }

    let relationships: Vec<RelationshipUpdateEntry> = package
        .relationship_updates
        .iter()
        .cloned()
        .map(Into::into)
        .collect();
    let incoming_pairs: Vec<(String, String)> = relationships
        .iter()
        .map(|entry| (entry.character_a_id.clone(), entry.character_b_id.clone()))
        .collect();
    tx.reconcile_continuity_package_relationships(branch_id, &scene.id, &incoming_pairs)?;

    for entry in &relationships {
        let source_character = tx.get_character(&entry.character_a_id)?;
        let target_character = tx.get_character(&entry.character_b_id)?;
        if source_character.project_id != target_character.project_id {
            anyhow::bail!("relationship characters must belong to the same project");
        }
        if project_id != source_character.project_id {
            anyhow::bail!("scene does not belong to the same project as the characters");
        }
        let rel = tx.update_relationship(
            branch_id,
            &UpdateRelationshipInput {
                character_a_id: entry.character_a_id.clone(),
                character_b_id: entry.character_b_id.clone(),
                trust_delta: Some(entry.trust_delta),
                tension_delta: Some(entry.tension_delta),
                reason: entry.reason.clone(),
                scene_id: Some(scene.id.clone()),
                ..Default::default()
            },
            "continuity_package",
        )?;
        applied.deltas.relationship_ids.push(format!(
            "relates_to:{}:{}:{}",
            rel.branch_id, rel.in_id, rel.out_id
        ));
    }

    let knowledge_entries: Vec<spindle_core::models::KnowledgeLearnedEntry> = package
        .knowledge_learned
        .iter()
        .cloned()
        .map(Into::into)
        .collect();
    for entry in &knowledge_entries {
        let normalized_fact = spindle_core::models::normalize_name(&entry.fact);
        let learned_at = StoryPlacement {
            book_number: scene.book_number,
            chapter_number: scene.chapter_number,
            scene_order: Some(scene.scene_order),
            note: None,
        };
        let fact = tx.upsert_knowledge_fact(UpsertKnowledgeFactParams {
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            character_id: entry.character_id.clone(),
            fact: entry.fact.clone(),
            normalized_fact,
            source_summary: entry.source_summary.clone().unwrap_or_default(),
            learned_at: Some(learned_at.clone()),
            confidence: None,
            tags: Vec::new(),
            reader_visible: entry.reader_visible.unwrap_or(true),
            source_import_session_id: None,
            secret_of_fact_id: entry.secret_of_fact_id.clone(),
        })?;
        applied.deltas.knowledge_fact_ids.push(fact.id);
    }

    for entry in &package.quantity_states {
        let state: spindle_core::models::QuantityState = entry.state.clone().into();
        let id = tx.append_quantity_state_in_tx(
            project_id,
            branch_id,
            &entry.subject_table,
            &entry.subject_id,
            &entry.measure,
            &state,
            Some(&scene.id),
            scene.book_number,
            scene.chapter_number,
            scene.scene_order,
        )?;
        applied.deltas.quantity_state_ids.push(id);
    }

    for entry in &package.narrative_promises {
        let planted_at: StoryPlacement = (&entry.planted_at).into();
        let planned_payoff = entry.planned_payoff.as_ref().map(StoryPlacement::from);
        let id = tx.create_narrative_promise_in_tx(
            project_id,
            branch_id,
            &entry.promise_type,
            &entry.description,
            &planted_at,
            planned_payoff.as_ref(),
            &entry.notes,
        )?;
        applied.deltas.narrative_promise_ids.push(id);
    }

    for entry in &package.timeline_events {
        let placement: StoryPlacement = (&entry.placement).into();
        let id = tx.create_timeline_event_in_tx(
            project_id,
            branch_id,
            &entry.title,
            &entry.event_type,
            &placement,
            &entry.summary,
            &entry.related_entity_ids,
        )?;
        applied.deltas.timeline_event_ids.push(id);
    }

    applied.deltas.total_applied = applied.deltas.character_state_ids.len()
        + applied.deltas.canonical_fact_ids.len()
        + applied.deltas.relationship_ids.len()
        + applied.deltas.knowledge_fact_ids.len()
        + applied.deltas.quantity_state_ids.len()
        + applied.deltas.narrative_promise_ids.len()
        + applied.deltas.timeline_event_ids.len();
    Ok(applied)
}

/// Canonical payload hash for idempotency comparison. `dry_run` and the key
/// itself are excluded so a preview followed by the live call does not claim
/// or collide with the key, while every content-bearing field is included.
pub(super) fn commit_scene_payload_hash(input: &CommitSceneInput) -> Result<String> {
    let mut value = serde_json::to_value(input)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("dry_run");
        object.remove("idempotency_key");
        // Preserve pre-0.4 inline payload hashes: absent file-only fields were
        // not part of the old DTO and serialized as null in the new one.
        if object
            .get("prose_path")
            .is_some_and(|value| value.is_null())
        {
            object.remove("prose_path");
        }
        if object
            .get("expected_sha256")
            .is_some_and(|value| value.is_null())
        {
            object.remove("expected_sha256");
        }
    }
    Ok(generation_sha256_hex(&serde_json::to_vec(&value)?))
}

fn commit_scene_operation_id() -> String {
    format!("op:{}", ulid::Ulid::new().to_string().to_lowercase())
}

fn scene_placement(book_number: i32, chapter_number: i32, scene_order: i32) -> StoryPlacement {
    StoryPlacement {
        book_number,
        chapter_number,
        scene_order: Some(scene_order),
        note: None,
    }
}

fn scene_current_version(versions: &[crate::sqlite::records::SceneVersion]) -> i32 {
    versions
        .first()
        .map(|version| version.version_number + 1)
        .unwrap_or(1)
}

fn commit_scene_issue_from_consistency(issue: ConsistencyIssue) -> CommitSceneIssue {
    CommitSceneIssue {
        severity: match issue.severity.as_str() {
            "error" => CommitSceneIssueSeverity::Error,
            "info" => CommitSceneIssueSeverity::Info,
            _ => CommitSceneIssueSeverity::Warning,
        },
        check_type: issue.check_type,
        message: issue.message,
        entity_ids: issue.entity_ids,
        suggested_action: issue.suggested_action,
    }
}

impl SqliteSpindleService {
    fn commit_scene_workspace_root(&self) -> Option<std::path::PathBuf> {
        let cwd = std::env::current_dir().ok()?;
        prose::workspace_root_from_data_dir(self.repository.data_dir(), &cwd)
    }

    fn resolve_commit_scene_prose(
        &self,
        input: &mut CommitSceneInput,
    ) -> Result<spindle_core::models::CommitSceneProseReceipt> {
        let root = self.commit_scene_workspace_root();
        let resolved = prose::resolve_prose(
            root.as_deref(),
            &input.full_text,
            input.prose_path.as_deref(),
            input.expected_sha256.as_deref(),
        )?;
        input.full_text = resolved.text;
        Ok(resolved.receipt)
    }

    /// Return a stored idempotency receipt for this exact payload, if one
    /// exists. Used by both the service and the authoring-run adapter so a
    /// retry can reach its receipt before any live-state phase assertion.
    pub async fn commit_scene_replay(
        &self,
        input: &CommitSceneInput,
    ) -> Result<Option<CommitSceneOutput>> {
        if input.dry_run {
            return Ok(None);
        }
        let Some(key) = input
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        else {
            return Ok(None);
        };
        let mut resolved = input.clone();
        self.resolve_commit_scene_prose(&mut resolved)?;
        let payload_hash = commit_scene_payload_hash(&resolved)?;
        let Some((stored_hash, stored_receipt)) =
            self.repository.find_scene_commit_receipt(key).await?
        else {
            return Ok(None);
        };
        if stored_hash != payload_hash {
            anyhow::bail!(
                "idempotency_key {key:?} was already used with a different commit payload; \
                 use a new key for a different scene or content"
            );
        }
        let mut receipt: CommitSceneOutput = serde_json::from_str(&stored_receipt)
            .with_context(|| format!("stored receipt for idempotency key {key:?} is invalid"))?;
        receipt.idempotent_replay = true;
        Ok(Some(receipt))
    }

    /// Atomic ordinary-scene commit: prose, author-declared continuity, beats,
    /// optional chapter summary, writer cursor, and an idempotency receipt all
    /// land in one SQLite write transaction. Blocking deterministic gates are
    /// evaluated before the transaction opens, so a failed gate writes nothing.
    pub async fn commit_scene(&self, mut input: CommitSceneInput) -> Result<CommitSceneOutput> {
        if let Some(replay) = self.commit_scene_replay(&input).await? {
            return Ok(replay);
        }
        let prose_receipt = self.resolve_commit_scene_prose(&mut input)?;
        if input.full_text.trim().is_empty() {
            anyhow::bail!("commit_scene requires non-empty prose");
        }
        if input.summary.trim().is_empty() {
            anyhow::bail!("commit_scene requires a non-empty summary");
        }

        let payload_hash = commit_scene_payload_hash(&input)?;

        let project = self.repository.get_project(&input.project_id).await?;
        let branch = match input
            .branch_id
            .as_deref()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
        {
            Some(branch_id) => {
                let branch = self.repository.get_branch(branch_id).await?;
                if branch.project_id.as_deref() != Some(input.project_id.as_str()) {
                    anyhow::bail!("branch does not belong to the requested project");
                }
                branch
            }
            None => self.repository.get_active_branch(&input.project_id).await?,
        };

        // Resolve the target scene and placement. An explicit scene id must be
        // on the requested branch: committing an inherited parent scene would
        // mutate a different branch than the caller selected.
        let (existing_scene, book_number, chapter_number, scene_order) = match input
            .scene_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(scene_id) => {
                let scene = self.repository.get_scene(scene_id).await?;
                if scene.project_id != input.project_id {
                    anyhow::bail!("scene does not belong to the requested project");
                }
                if scene.branch_id != branch.id {
                    anyhow::bail!(
                        "scene {} belongs to branch {}, not the requested branch {}; \
                             commit on its own branch",
                        scene.id,
                        scene.branch_id,
                        branch.id
                    );
                }
                (
                    Some(scene.clone()),
                    scene.book_number,
                    scene.chapter_number,
                    scene.scene_order,
                )
            }
            None => {
                let book_number = input.book_number.ok_or_else(|| {
                    anyhow::anyhow!(
                        "commit_scene requires scene_id or book_number/chapter_number/scene_order"
                    )
                })?;
                let chapter_number = input.chapter_number.ok_or_else(|| {
                    anyhow::anyhow!("commit_scene requires chapter_number when scene_id is omitted")
                })?;
                let scene_order = input.scene_order.ok_or_else(|| {
                    anyhow::anyhow!("commit_scene requires scene_order when scene_id is omitted")
                })?;
                let existing = self
                    .repository
                    .find_scene_by_natural_key(
                        &input.project_id,
                        &branch.id,
                        book_number,
                        chapter_number,
                        scene_order,
                    )
                    .await?;
                (existing, book_number, chapter_number, scene_order)
            }
        };

        let versions = match existing_scene.as_ref() {
            Some(scene) => self.repository.list_scene_versions(&scene.id).await?,
            None => Vec::new(),
        };
        let current_version = existing_scene
            .as_ref()
            .map(|_| scene_current_version(&versions))
            .unwrap_or(0);
        if let Some(expected) = input.expected_scene_version
            && expected != current_version
        {
            anyhow::bail!(
                "stale expected_scene_version: caller expected {expected}, current is {current_version}; \
                 re-read the scene or preparation packet before committing"
            );
        }

        // Stale preparation packet: both the composite token and the individual
        // dependency hashes are revalidated. Dependencies cover canon, style,
        // and plan state, not just the scene's own prose hash.
        let placement = scene_placement(book_number, chapter_number, scene_order);
        if let Some(token) = input
            .context_revision_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            let dependencies = self
                .compute_scene_preparation_dependencies(
                    &input.project_id,
                    &branch.id,
                    &placement,
                    existing_scene.as_ref().map(|scene| scene.id.as_str()),
                )
                .await?;
            let current = super::context::composite_context_revision_token(&dependencies);
            if current != token {
                anyhow::bail!(
                    "stale context_revision_token: preparation packet no longer matches canon, \
                     style, plan, or scene text; call prepare_scene again"
                );
            }
        }
        if let Some(expected) = input.dependency_tokens.clone() {
            let freshness: ScenePreparationFreshness = self
                .check_prepare_scene_freshness(
                    &input.project_id,
                    &branch.id,
                    &placement,
                    existing_scene.as_ref().map(|scene| scene.id.as_str()),
                    &expected,
                )
                .await?;
            if !freshness.fresh {
                anyhow::bail!(
                    "stale scene preparation dependencies: {}; call prepare_scene again",
                    freshness.stale_components.join(", ")
                );
            }
        }

        // Provenance/rating gates mirror `save_scene_draft`. The receipt is
        // provenance only; the caller's prose is always what persists.
        let receipt_id = input
            .generation_receipt
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty());
        let mut draft_origin = match input.authorship {
            SceneAuthorship::Assistant => Some("assistant:host".to_string()),
            SceneAuthorship::Human => Some("operator".to_string()),
        };
        let mut receipt_claim: Option<(String, String)> = None;
        if input.content_rating == spindle_core::models::ContentRating::Explicit {
            if let Some(receipt_id) = receipt_id {
                let receipt = self
                    .verified_explicit_save_receipt(Some(receipt_id))
                    .await?;
                draft_origin = Some(format!("agent:{}", receipt.agent_id));
                receipt_claim = Some((
                    receipt.id.clone(),
                    explicit_receipt_scene_key(&SaveSceneDraftInput {
                        project_id: input.project_id.clone(),
                        book_number,
                        chapter_number,
                        chapter_id: None,
                        scene_order,
                        full_text: input.full_text.clone(),
                        summary: input.summary.clone(),
                        content_rating: input.content_rating.clone(),
                        tone: input.tone.clone(),
                        location_id: input.location_id.clone(),
                        ..Default::default()
                    }),
                ));
            } else if crate::sqlite::import_service::contains_explicit_sexual_prose(
                &input.full_text,
            ) {
                anyhow::bail!(
                    "generation_receipt is required when committing explicit sexual prose; \
                     obtain one from a cleared explicit-capable draft generation"
                );
            }
        } else if let Some(receipt_id) = receipt_id
            && let Some(receipt) = self
                .verified_revisable_draft_receipt(Some(receipt_id))
                .await?
        {
            draft_origin = Some(format!("agent:{}", receipt.agent_id));
        }

        // Secret-link validation happens before any write so an invalid reveal
        // fails the whole commit atomically.
        for entry in &input.continuity.knowledge_learned {
            let character = self.repository.get_character(&entry.character_id).await?;
            if character.project_id != input.project_id {
                anyhow::bail!(
                    "knowledge_learned entry names character '{}', which does not belong to the requested project",
                    entry.character_id
                );
            }
            if let Some(fact_id) = entry.secret_of_fact_id.as_deref() {
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
        }
        for entry in &input.continuity.quantity_states {
            let scheme = self
                .repository
                .get_project_quantity_scheme(&input.project_id, &branch.id, &entry.measure)
                .await?
                .map(|stored| stored.scheme);
            let state: spindle_core::models::QuantityState = entry.state.clone().into();
            state
                .validate(scheme.as_ref())
                .map_err(|error| anyhow::anyhow!("invalid quantity state: {error}"))?;
        }

        // Deterministic gates. A blocking failure returns findings without
        // opening the write transaction, so nothing is persisted.
        let mut issues: Vec<CommitSceneIssue> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut checks_run = vec!["world_rules".to_string()];
        let mut checks_skipped: Vec<String> = Vec::new();

        let world_rule_hits = self
            .scan_world_rules(&input.project_id, &branch.id, &input.full_text)
            .await?;
        for hit in &world_rule_hits {
            let severity = match hit.severity {
                spindle_core::models::WorldRuleSeverity::Likely => CommitSceneIssueSeverity::Error,
                spindle_core::models::WorldRuleSeverity::Possible => {
                    CommitSceneIssueSeverity::Warning
                }
            };
            issues.push(CommitSceneIssue {
                severity,
                check_type: "world_rule_compliance".to_string(),
                message: format!(
                    "world rule {} matched at bytes {}-{}",
                    hit.rule_id, hit.byte_range.start, hit.byte_range.end
                ),
                entity_ids: vec![hit.rule_id.clone()],
                suggested_action: Some(
                    "revise the prose or update the world rule before committing".to_string(),
                ),
            });
        }

        checks_run.push("canonical_fact_consistency".to_string());
        let active_facts = self
            .repository
            .list_active_canonical_facts_by_project_and_branch(&input.project_id, &branch.id)
            .await?;
        let mut candidates: Vec<crate::format::ContradictionCandidate> = active_facts
            .iter()
            .map(crate::format::candidate_from_canonical_fact)
            .collect();
        for (index, entry) in input.continuity.canonical_facts.iter().enumerate() {
            let entry: spindle_core::models::CanonicalFactEntry = entry.clone().into();
            if let Some(candidate) = crate::format::candidate_from_commit_entry(
                &entry,
                format!("(pending fact {index})"),
            ) {
                candidates.push(candidate);
            }
        }
        for contradiction in crate::format::detect_fact_contradictions(&candidates) {
            issues.push(CommitSceneIssue {
                severity: CommitSceneIssueSeverity::Error,
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
                    "supersede the outdated fact (supersedes_fact_id) or set distinct validity windows"
                        .to_string(),
                ),
            });
        }

        // Retcon/temporal scans need a scene view. For a brand-new scene we
        // still scan the prospective prose with a synthetic record so the gate
        // does not silently disappear; the clock-specific temporal scan is the
        // only pass that requires a persisted scene.
        // Scan the PROPOSED prose, not the persisted revision: a revision that
        // introduces future knowledge must be caught, and one that removes a
        // violation must not inherit the old text's finding.
        let scan_scene = existing_scene
            .clone()
            .map(|mut scene| {
                scene.full_text = input.full_text.clone();
                scene.summary = input.summary.clone();
                scene.content_rating = input.content_rating.as_db_str().to_string();
                scene.tone = input.tone.clone();
                scene.draft_origin = draft_origin.clone();
                scene.location_id = input.location_id.clone().or(scene.location_id);
                scene
            })
            .unwrap_or_else(|| {
                use chrono::Utc;
                crate::sqlite::records::Scene {
                    id: "scene:pending".to_string(),
                    project_id: input.project_id.clone(),
                    branch_id: branch.id.clone(),
                    book_id: String::new(),
                    chapter_id: String::new(),
                    book_number,
                    chapter_number,
                    scene_order,
                    full_text: input.full_text.clone(),
                    summary: input.summary.clone(),
                    content_rating: input.content_rating.as_db_str().to_string(),
                    tone: input.tone.clone(),
                    draft_origin: draft_origin.clone(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                    location_id: input.location_id.clone(),
                }
            });
        checks_run.push("retcon_reachability".to_string());
        let pending_states = pending_character_statuses(
            input
                .continuity
                .character_states
                .iter()
                .map(|entry| (entry.character_id.as_str(), entry.changes.status.as_ref())),
        );
        let retcon_findings = self
            .scan_retcon_findings_with_pending(&input.project_id, &scan_scene, &pending_states)
            .await?;
        for finding in &retcon_findings {
            issues.push(commit_scene_issue_from_consistency(
                retcon_finding_to_consistency_issue(finding),
            ));
        }
        if let Some(scene) = existing_scene.as_ref() {
            checks_run.push("temporal_coherence".to_string());
            let scene_clock = self.repository.get_scene_clock(&scene.id).await?;
            for finding in scan_temporal_findings(&scene.id, &input.full_text, scene_clock.as_ref())
            {
                issues.push(commit_scene_issue_from_consistency(finding));
            }
        } else {
            checks_skipped
                .push("temporal_coherence: no persisted clock for a new scene".to_string());
        }

        checks_run.push("style_contract".to_string());
        let style_directive = self
            .style_directive_for(&input.project_id, &branch.id)
            .await
            .unwrap_or_default();
        let style_hits = style_directive.scan(&spindle_core::style::StyleScanInput {
            prose: &input.full_text,
            declared_tone: input.tone.as_deref(),
            is_chapter_end: input.chapter_summary.is_some(),
        });
        warnings.extend(style_hits.into_iter().map(|hit| hit.message));

        let blocking_count = issues
            .iter()
            .filter(|issue| issue.severity == CommitSceneIssueSeverity::Error)
            .count();
        let advisory_total = issues.len().saturating_sub(blocking_count);
        let coverage = CommitSceneCoverage {
            checks_run,
            checks_skipped,
            findings_total: issues.len(),
            advisory_total,
        };

        let mut follow_ups = Vec::new();
        if input.chapter_summary.is_none() {
            follow_ups.push(CommitSceneFollowUp {
                kind: CommitSceneFollowUpKind::ChapterSummary,
                message: "provide chapter_summary when this is the chapter's final scene"
                    .to_string(),
            });
        } else {
            follow_ups.push(CommitSceneFollowUp {
                kind: CommitSceneFollowUpKind::CheckpointAudit,
                message: "run a checkpoint audit after the chapter-completion summary".to_string(),
            });
        }
        if input.run_id.is_some() {
            follow_ups.push(CommitSceneFollowUp {
                kind: CommitSceneFollowUpKind::AuthoringRun,
                message: "advance the authoring run phase for the committed scene".to_string(),
            });
        }

        let scene_id_for_receipt = existing_scene
            .as_ref()
            .map(|scene| scene.id.clone())
            .unwrap_or_else(|| format!("scene:pending:{payload_hash}"));

        if blocking_count > 0 || input.dry_run {
            let proposed = &input.continuity;
            let applied = CommitSceneAppliedDeltas {
                total_applied: proposed.len(),
                character_state_ids: Vec::new(),
                canonical_fact_ids: Vec::new(),
                relationship_ids: Vec::new(),
                knowledge_fact_ids: Vec::new(),
                quantity_state_ids: Vec::new(),
                narrative_promise_ids: Vec::new(),
                timeline_event_ids: Vec::new(),
            };
            return Ok(CommitSceneOutput {
                operation_id: if blocking_count > 0 {
                    format!("op:blocked:{}", &payload_hash[..16])
                } else {
                    format!("op:preview:{}", &payload_hash[..16])
                },
                scene_id: scene_id_for_receipt,
                version_number: current_version.max(1),
                branch_id: branch.id,
                dry_run: input.dry_run,
                idempotent_replay: false,
                issues,
                warnings,
                blocking_count,
                coverage,
                applied,
                beat_count: input.beat_annotations.len(),
                chapter_summary_saved: false,
                writer_cursor: None,
                follow_ups,
                undo_support: CommitSceneUndoSupport::Irreversible,
                undo_reason: Some(if blocking_count > 0 {
                    "blocked commit wrote nothing to undo".to_string()
                } else {
                    "dry run wrote nothing; a live commit revalidates first".to_string()
                }),
                prose: Some(prose_receipt.clone()),
            });
        }

        // Live commit. Everything below is one SQLite transaction.
        let save_input = SaveSceneDraftInput {
            project_id: input.project_id.clone(),
            book_number,
            chapter_number,
            chapter_id: None,
            scene_order,
            full_text: input.full_text.clone(),
            authorship: match input.authorship {
                SceneAuthorship::Assistant => DraftAuthorship::Assistant,
                SceneAuthorship::Human => DraftAuthorship::Human,
            },
            summary: input.summary.clone(),
            content_rating: input.content_rating.clone(),
            tone: input.tone.clone(),
            generation_id: None,
            source_path: None,
            location_id: input.location_id.clone(),
            ..Default::default()
        };

        let project_id = input.project_id.clone();
        let branch_id = branch.id.clone();
        let project_slug = project.name.clone();
        let continuity = input.continuity.clone();
        let beat_annotations = input.beat_annotations.clone();
        let chapter_summary = input.chapter_summary.clone();
        let run_id = input.run_id.clone();
        let idempotency_key = input
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string);
        let operation_id = commit_scene_operation_id();
        let log_project_id = project_id.clone();
        let log_branch_id = branch_id.clone();
        let log_operation_id = operation_id.clone();
        let payload_hash_owned = payload_hash.clone();
        let receipt_claim_owned = receipt_claim.clone();
        let draft_origin_owned = draft_origin.clone();
        let expected_version = input.expected_scene_version;
        let existing_writer = self
            .repository
            .get_writer_position(&project_id, &branch_id)
            .await?;
        let intent = existing_writer
            .as_ref()
            .map(|position| position.intent.clone())
            .unwrap_or_else(|| "drafting".to_string());
        let next_focus = existing_writer.and_then(|position| position.next_focus);
        let issues_owned = issues.clone();
        let warnings_owned = warnings.clone();
        let coverage_owned = coverage.clone();
        let follow_ups_owned = follow_ups.clone();

        let output = self
            .repository
            .with_transaction(move |tx| {
                // Re-resolve the natural key INSIDE the writer transaction so
                // two racing creates with expected_scene_version=0 cannot both
                // land: the first creates the scene, the second sees version 1.
                if let Some(expected) = expected_version {
                    let actual = match tx.find_scene_by_natural_key_in_tx(
                        &project_id,
                        &branch_id,
                        book_number,
                        chapter_number,
                        scene_order,
                    )? {
                        Some(scene) => tx.current_scene_version_in_tx(&scene.id)?,
                        None => 0,
                    };
                    if actual != expected {
                        anyhow::bail!(
                            "stale expected_scene_version: caller expected {expected}, current is {actual}"
                        );
                    }
                }

                let (scene, _created) = tx.persist_scene_in_tx(
                    &project_id,
                    &branch_id,
                    &save_input,
                    receipt_claim_owned
                        .as_ref()
                        .map(|(receipt_id, key)| (receipt_id.as_str(), key.as_str())),
                    draft_origin_owned.as_deref(),
                )?;

                let applied = apply_continuity_in_tx(tx, &project_id, &branch_id, &scene, &continuity)?;

                let beat_count = if beat_annotations.is_empty() {
                    0
                } else {
                    let beats: Vec<spindle_core::models::AnnotatedBeat> = beat_annotations
                        .iter()
                        .map(|beat| spindle_core::models::AnnotatedBeat {
                            beat_type: beat.beat_type.clone(),
                            summary: beat.summary.clone(),
                        })
                        .collect();
                    tx.annotate_scene_beats_in_tx(
                        &project_id,
                        &branch_id,
                        &scene.id,
                        &[],
                        &[],
                        &[],
                        &beats,
                        None,
                    )?;
                    beats.len()
                };

                let chapter_summary_saved = if let Some(summary) = chapter_summary.as_ref() {
                    tx.save_chapter_summary_in_tx(
                        &project_id,
                        &branch_id,
                        scene.book_number,
                        scene.chapter_number,
                        &summary.summary,
                        &summary.key_events,
                        &summary.character_changes,
                        &summary.relationship_shifts,
                        &summary.arc_advances,
                        &summary.promise_events,
                    )?;
                    true
                } else {
                    false
                };

                tx.upsert_writer_position_in_tx(crate::sqlite::repository::UpsertWriterPositionParams {
                    project_id: project_id.clone(),
                    branch_id: branch_id.clone(),
                    book_id: Some(scene.book_id.clone()),
                    chapter_id: Some(scene.chapter_id.clone()),
                    scene_id: Some(scene.id.clone()),
                    intent: intent.clone(),
                    next_focus: next_focus.clone(),
                    updated_by: "spindle-mcp".to_string(),
                    updated_at: chrono::Utc::now(),
                })?;

                let version_number = tx.current_scene_version_in_tx(&scene.id)?;
                let output = CommitSceneOutput {
                    operation_id: operation_id.clone(),
                    scene_id: scene.id.clone(),
                    version_number,
                    branch_id: branch_id.clone(),
                    dry_run: false,
                    idempotent_replay: false,
                    issues: issues_owned,
                    warnings: warnings_owned,
                    blocking_count: 0,
                    coverage: coverage_owned,
                    applied: applied.deltas,
                    beat_count,
                    chapter_summary_saved,
                    writer_cursor: Some(CommitSceneCursor {
                        scene_id: scene.id.clone(),
                        book_number: scene.book_number,
                        chapter_number: scene.chapter_number,
                        scene_order: scene.scene_order,
                    }),
                    follow_ups: follow_ups_owned,
                    undo_support: CommitSceneUndoSupport::CheckpointOnly,
                    undo_reason: Some(
                        "prose restores from the saved scene version; approved continuity deltas \
                         require a branch checkpoint until undo_change ships"
                            .to_string(),
                    ),
                    prose: Some(prose_receipt.clone()),
                };

                if let Some(key) = idempotency_key.as_deref() {
                    let receipt_json = serde_json::to_string(&output)?;
                    tx.store_scene_commit_receipt_in_tx(
                        &operation_id,
                        &project_id,
                        &branch_id,
                        &scene.id,
                        key,
                        &payload_hash_owned,
                        &receipt_json,
                    )?;
                }

                Ok((output, scene.id, scene.book_number, scene.chapter_number, scene.scene_order))
            })
            .await?;

        let (output, scene_id, committed_book, committed_chapter, committed_order) = output;

        self.log_scene_commit_activity(
            &log_project_id,
            &log_branch_id,
            &scene_id,
            &log_operation_id,
            created_count(&output),
            &project_slug,
            committed_book,
            committed_chapter,
            committed_order,
            run_id.as_deref(),
        )
        .await;

        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    async fn log_scene_commit_activity(
        &self,
        project_id: &str,
        branch_id: &str,
        scene_id: &str,
        operation_id: &str,
        applied_count: usize,
        project_slug: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
        run_id: Option<&str>,
    ) {
        if let Err(error) = self
            .repository
            .append_session_activity(AppendSessionActivityParams {
                project_id: project_id.to_string(),
                branch_id: branch_id.to_string(),
                kind: "scene_committed".to_string(),
                subject_table: Some("scene".to_string()),
                subject_id: Some(scene_id.to_string()),
                summary: format!(
                    "Committed scene {book_number}.{chapter_number}.{scene_order} for {project_slug}."
                ),
                details_json: Some(serde_json::json!({
                    "scene_id": scene_id,
                    "operation_id": operation_id,
                    "applied_continuity": applied_count,
                    "run_id": run_id,
                })),
            })
            .await
        {
            tracing::warn!("scene commit activity log failed: {error}");
        }
    }
}

fn created_count(output: &CommitSceneOutput) -> usize {
    output.applied.total_applied
}
