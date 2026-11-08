use super::*;

impl SqliteSpindleService {
    /// Project-scoped diff between two branches' scenes, character
    /// states, relationships, and pacing trackers. Pure SQLite reads;
    /// no LLM. Mirrors services/mod.rs:7072..7362 in 705b835^.
    ///
    /// `RelatesTo` field-name delta vs reference: the SurrealDB record
    /// used `r#in` / `out`; the SQLite record uses `in_id` / `out_id`
    /// (composite primary key, no surrogate id).
    pub async fn diff_branches(
        &self,
        input: spindle_core::models::DiffBranchesInput,
    ) -> Result<spindle_core::models::DiffBranchesOutput> {
        use spindle_core::models::{
            CharacterStateDiffItem, DiffBranchesOutput, PacingDiffItem, RelationshipDiffItem,
            SceneDiffItem,
        };
        use std::collections::{BTreeMap, BTreeSet};

        let project = self.repository.get_project(&input.project_id).await?;
        let base_branch = self.repository.get_branch(&input.base_branch_id).await?;
        let compare_branch = self.repository.get_branch(&input.compare_branch_id).await?;

        for branch in [&base_branch, &compare_branch] {
            if branch
                .project_id
                .as_deref()
                .is_some_and(|id| id != project.id)
            {
                anyhow::bail!(
                    "branch {} does not belong to project {}",
                    branch.id,
                    project.id
                );
            }
        }

        let characters = self
            .repository
            .list_characters_by_project_and_branch(&project.id, &input.base_branch_id)
            .await?;
        let character_names: BTreeMap<String, String> =
            characters.into_iter().map(|c| (c.id, c.name)).collect();

        let base_scenes = self
            .repository
            .list_scenes_by_project_and_branch(&project.id, &input.base_branch_id)
            .await?;
        let compare_scenes = self
            .repository
            .list_scenes_by_project_and_branch(&project.id, &input.compare_branch_id)
            .await?;
        let base_scene_map: BTreeMap<(i32, i32, i32), &_> = base_scenes
            .iter()
            .map(|s| ((s.book_number, s.chapter_number, s.scene_order), s))
            .collect();
        let compare_scene_map: BTreeMap<(i32, i32, i32), &_> = compare_scenes
            .iter()
            .map(|s| ((s.book_number, s.chapter_number, s.scene_order), s))
            .collect();
        let scene_keys: BTreeSet<_> = base_scene_map
            .keys()
            .chain(compare_scene_map.keys())
            .copied()
            .collect();
        let scene_diffs = scene_keys
            .into_iter()
            .filter_map(
                |key| match (base_scene_map.get(&key), compare_scene_map.get(&key)) {
                    (Some(base), Some(compare)) if base.summary != compare.summary => {
                        Some(SceneDiffItem {
                            book_number: key.0,
                            chapter_number: key.1,
                            scene_order: key.2,
                            base_summary: Some(base.summary.clone()),
                            compare_summary: Some(compare.summary.clone()),
                            change_type: "modified".to_string(),
                        })
                    }
                    (Some(base), None) => Some(SceneDiffItem {
                        book_number: key.0,
                        chapter_number: key.1,
                        scene_order: key.2,
                        base_summary: Some(base.summary.clone()),
                        compare_summary: None,
                        change_type: "removed".to_string(),
                    }),
                    (None, Some(compare)) => Some(SceneDiffItem {
                        book_number: key.0,
                        chapter_number: key.1,
                        scene_order: key.2,
                        base_summary: None,
                        compare_summary: Some(compare.summary.clone()),
                        change_type: "added".to_string(),
                    }),
                    _ => None,
                },
            )
            .collect::<Vec<_>>();

        let base_states = self
            .repository
            .list_character_states_by_project_and_branch(&project.id, &input.base_branch_id)
            .await?;
        let compare_states = self
            .repository
            .list_character_states_by_project_and_branch(&project.id, &input.compare_branch_id)
            .await?;
        let base_state_map: BTreeMap<(String, i32, i32, i32), &_> = base_states
            .iter()
            .map(|s| {
                (
                    (
                        s.character_id.clone(),
                        s.book_number,
                        s.chapter_number,
                        s.scene_order,
                    ),
                    s,
                )
            })
            .collect();
        let compare_state_map: BTreeMap<(String, i32, i32, i32), &_> = compare_states
            .iter()
            .map(|s| {
                (
                    (
                        s.character_id.clone(),
                        s.book_number,
                        s.chapter_number,
                        s.scene_order,
                    ),
                    s,
                )
            })
            .collect();
        let state_keys: BTreeSet<_> = base_state_map
            .keys()
            .chain(compare_state_map.keys())
            .cloned()
            .collect();
        let character_state_diffs = state_keys
            .into_iter()
            .filter_map(|key| {
                let base = base_state_map.get(&key);
                let compare = compare_state_map.get(&key);
                if let (Some(base), Some(compare)) = (base, compare)
                    && base.status == compare.status
                    && base.goals == compare.goals
                {
                    return None;
                }
                Some(CharacterStateDiffItem {
                    character_id: key.0.clone(),
                    character_name: character_names
                        .get(&key.0)
                        .cloned()
                        .unwrap_or_else(|| key.0.clone()),
                    position: format!("{}:{}:{}", key.1, key.2, key.3),
                    base_status: base.map(|s| s.status.clone()).unwrap_or_default(),
                    compare_status: compare.map(|s| s.status.clone()).unwrap_or_default(),
                    base_goals: base.map(|s| s.goals.clone()).unwrap_or_default(),
                    compare_goals: compare.map(|s| s.goals.clone()).unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();

        let base_relationships = self
            .repository
            .list_relationships_by_branch(&input.base_branch_id)
            .await?;
        let compare_relationships = self
            .repository
            .list_relationships_by_branch(&input.compare_branch_id)
            .await?;
        let base_rel_map: BTreeMap<(String, String, String), &_> = base_relationships
            .iter()
            .map(|r| {
                (
                    (
                        r.in_id.clone(),
                        r.out_id.clone(),
                        r.relationship_type.clone(),
                    ),
                    r,
                )
            })
            .collect();
        let compare_rel_map: BTreeMap<(String, String, String), &_> = compare_relationships
            .iter()
            .map(|r| {
                (
                    (
                        r.in_id.clone(),
                        r.out_id.clone(),
                        r.relationship_type.clone(),
                    ),
                    r,
                )
            })
            .collect();
        let rel_keys: BTreeSet<_> = base_rel_map
            .keys()
            .chain(compare_rel_map.keys())
            .cloned()
            .collect();
        let relationship_diffs = rel_keys
            .into_iter()
            .filter_map(|key| {
                let base = base_rel_map.get(&key);
                let compare = compare_rel_map.get(&key);
                if let (Some(base), Some(compare)) = (base, compare)
                    && base.trust == compare.trust
                    && base.tension == compare.tension
                {
                    return None;
                }
                Some(RelationshipDiffItem {
                    source_character_id: key.0,
                    target_character_id: key.1,
                    relationship_type: key.2,
                    base_trust: base.map(|r| r.trust),
                    compare_trust: compare.map(|r| r.trust),
                    base_tension: base.map(|r| r.tension),
                    compare_tension: compare.map(|r| r.tension),
                })
            })
            .collect::<Vec<_>>();

        let base_pacing = self
            .repository
            .list_pacing_trackers_by_project_and_branch(&project.id, &input.base_branch_id)
            .await?;
        let compare_pacing = self
            .repository
            .list_pacing_trackers_by_project_and_branch(&project.id, &input.compare_branch_id)
            .await?;
        let base_pacing_map: BTreeMap<String, &_> = base_pacing
            .iter()
            .map(|t| (t.character_arc_id.clone(), t))
            .collect();
        let compare_pacing_map: BTreeMap<String, &_> = compare_pacing
            .iter()
            .map(|t| (t.character_arc_id.clone(), t))
            .collect();
        let pacing_keys: BTreeSet<_> = base_pacing_map
            .keys()
            .chain(compare_pacing_map.keys())
            .cloned()
            .collect();
        let pacing_diffs = pacing_keys
            .into_iter()
            .filter_map(|key| {
                let base = base_pacing_map.get(&key);
                let compare = compare_pacing_map.get(&key);
                if let (Some(base), Some(compare)) = (base, compare)
                    && (base.current_progress - compare.current_progress).abs() < f64::EPSILON
                    && base.status == compare.status
                {
                    return None;
                }
                Some(PacingDiffItem {
                    character_arc_id: key,
                    tracker_id: compare
                        .map(|t| t.id.clone())
                        .or_else(|| base.map(|t| t.id.clone()))
                        .unwrap_or_default(),
                    base_progress: base.map(|t| t.current_progress),
                    compare_progress: compare.map(|t| t.current_progress),
                    base_status: base.map(|t| t.status.clone()),
                    compare_status: compare.map(|t| t.status.clone()),
                })
            })
            .collect::<Vec<_>>();

        Ok(DiffBranchesOutput {
            base_branch: crate::format::branch_summary(
                &base_branch,
                project.active_branch_id.as_deref(),
            ),
            compare_branch: crate::format::branch_summary(
                &compare_branch,
                project.active_branch_id.as_deref(),
            ),
            narrative_impact_summary: summarize_branch_impact(
                &scene_diffs,
                &character_state_diffs,
                &relationship_diffs,
                &pacing_diffs,
            ),
            scene_diffs,
            character_state_diffs,
            relationship_diffs,
            pacing_diffs,
        })
    }

    /// Merge a feature branch into a target branch. In the per-project
    /// design the default target is the project's `main` branch (looked
    /// up by name, not via a singleton record-id). Mirrors
    /// services/mod.rs:7363..7502 in 705b835^.
    ///
    /// Per-project main branches (Risk #6): the SurrealDB reference
    /// defaulted `target_branch_id` to the singleton `bible_branch:main`.
    /// Here we resolve "the project's main branch" by listing branches
    /// for the project and matching on `name == "main"`. This keeps the
    /// per-project main-branch design (cleaner FK semantics, no shared
    /// singleton) without breaking callers that didn't pass an explicit
    /// `target_branch_id`.
    ///
    /// Conflict detection runs purely in-process via
    /// `detect_merge_scene_conflicts`. Once conflicts are identified, the
    /// scene/character_state slices are filtered to drop conflicting
    /// positions, and the mergeable slice is committed via
    /// `Repository::merge_branch_snapshot`.
    pub async fn merge_branch(
        &self,
        input: spindle_core::models::MergeBranchInput,
    ) -> Result<spindle_core::models::MergeBranchOutput> {
        use spindle_core::models::MergeBranchOutput;
        use std::collections::{BTreeMap, BTreeSet};

        let project = self.repository.get_project(&input.project_id).await?;

        // Per-project main lookup. We need this for both:
        //   * defaulting target_branch_id when the caller omits it
        //   * deciding whether a target_branch_id IS the project's main
        //     (the conflict detector uses that to skip the main-fallback)
        let main_branch = self
            .repository
            .list_branches_by_project(&project.id)
            .await?
            .into_iter()
            .find(|b| b.name == "main")
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "project {} has no main branch; cannot resolve merge target",
                    project.id
                )
            })?;

        let target_branch_id = input
            .target_branch_id
            .clone()
            .unwrap_or_else(|| main_branch.id.clone());

        if input.merge_type != "fast_forward" && input.merge_type != "squash" {
            anyhow::bail!("unsupported merge type: {}", input.merge_type);
        }

        let source_branch = self.repository.get_branch(&input.source_branch_id).await?;
        let target_branch = self.repository.get_branch(&target_branch_id).await?;
        for branch in [&source_branch, &target_branch] {
            if branch
                .project_id
                .as_deref()
                .is_some_and(|id| id != project.id)
            {
                anyhow::bail!(
                    "branch {} does not belong to project {}",
                    branch.id,
                    project.id
                );
            }
        }

        let source_scenes = self
            .repository
            .list_scenes_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?;
        let target_local_scenes = self
            .repository
            .list_scenes_by_project_and_branch(&project.id, &target_branch_id)
            .await?;
        // Skip the main-fallback fetch when the target IS the main branch.
        let main_scenes = if target_branch_id == main_branch.id {
            Vec::new()
        } else {
            self.repository
                .list_scenes_by_project_and_branch(&project.id, &main_branch.id)
                .await?
        };
        let source_states: Vec<_> = self
            .repository
            .list_character_states_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?
            .into_iter()
            .filter(|s| s.branch_id == input.source_branch_id)
            .collect();
        let source_relationships: Vec<_> = self
            .repository
            .list_relationships_by_branch(&input.source_branch_id)
            .await?
            .into_iter()
            .filter(|r| r.branch_id == input.source_branch_id)
            .collect();
        let source_pacing: Vec<_> = self
            .repository
            .list_pacing_trackers_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?
            .into_iter()
            .filter(|p| p.branch_id == input.source_branch_id)
            .collect();

        let target_local_scene_map: BTreeMap<(i32, i32, i32), &_> = target_local_scenes
            .iter()
            .map(|s| ((s.book_number, s.chapter_number, s.scene_order), s))
            .collect();
        let main_scene_map: BTreeMap<(i32, i32, i32), &_> = main_scenes
            .iter()
            .map(|s| ((s.book_number, s.chapter_number, s.scene_order), s))
            .collect();

        let conflicts = detect_merge_scene_conflicts(
            &source_scenes,
            &source_branch,
            &target_branch_id,
            &main_branch.id,
            &target_local_scene_map,
            &main_scene_map,
        );
        let conflict_positions: BTreeSet<(i32, i32, i32)> = conflicts
            .iter()
            .map(|c| (c.book_number, c.chapter_number, c.scene_order))
            .collect();

        let mergeable_scenes: Vec<_> = source_scenes
            .iter()
            .filter(|s| {
                !conflict_positions.contains(&(s.book_number, s.chapter_number, s.scene_order))
            })
            .cloned()
            .collect();
        let mergeable_states: Vec<_> = source_states
            .iter()
            .filter(|s| {
                !conflict_positions.contains(&(s.book_number, s.chapter_number, s.scene_order))
            })
            .cloned()
            .collect();

        self.repository
            .merge_branch_snapshot(
                &project.id,
                &target_branch_id,
                &mergeable_scenes,
                &mergeable_states,
                &source_relationships,
                &source_pacing,
            )
            .await?;

        // Carry branch-local world-rule canon, which the snapshot above silently
        // drops. Only rules created ON the source branch are re-homed to target.
        let source_local_rule_ids: Vec<String> = self
            .repository
            .list_world_rules_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?
            .into_iter()
            .filter(|rule| rule.branch_id == input.source_branch_id)
            .map(|rule| rule.id)
            .collect();
        self.repository
            .merge_world_rules_to_branch(&target_branch_id, &source_local_rule_ids)
            .await?;

        // Re-home the remaining branch-local canon (also dropped by the
        // scene-only snapshot): canonical facts, promises, timeline events,
        // knowledge facts, and character arcs created on the source branch.
        let canonical_fact_ids: Vec<String> = self
            .repository
            .list_active_canonical_facts_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?
            .into_iter()
            .filter(|f| f.branch_id == input.source_branch_id)
            .map(|f| f.id)
            .collect();
        self.repository
            .rehome_rows_to_branch("canonical_fact", &target_branch_id, &canonical_fact_ids)
            .await?;

        let promise_ids: Vec<String> = self
            .repository
            .list_narrative_promises_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?
            .into_iter()
            .filter(|p| p.branch_id == input.source_branch_id)
            .map(|p| p.id)
            .collect();
        self.repository
            .rehome_rows_to_branch("narrative_promise", &target_branch_id, &promise_ids)
            .await?;

        let timeline_ids: Vec<String> = self
            .repository
            .list_timeline_events_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?
            .into_iter()
            .filter(|e| e.branch_id == input.source_branch_id)
            .map(|e| e.id)
            .collect();
        self.repository
            .rehome_rows_to_branch("timeline_event", &target_branch_id, &timeline_ids)
            .await?;

        let knowledge_ids: Vec<String> = self
            .repository
            .list_knowledge_facts_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?
            .into_iter()
            .filter(|k| k.branch_id == input.source_branch_id)
            .map(|k| k.id)
            .collect();
        self.repository
            .rehome_rows_to_branch("knowledge_fact", &target_branch_id, &knowledge_ids)
            .await?;

        let arc_ids: Vec<String> = self
            .repository
            .list_character_arcs_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?
            .into_iter()
            .filter(|a| a.branch_id == input.source_branch_id)
            .map(|a| a.id)
            .collect();
        self.repository
            .rehome_rows_to_branch("character_arc", &target_branch_id, &arc_ids)
            .await?;

        // Carry branch-local quantity schemes + stamped quantity state, also
        // dropped by the scene-only snapshot. Schemes upsert by (project,
        // measure) into the target; state rows re-home by id.
        for stored in self
            .repository
            .list_project_quantity_schemes(&project.id, &input.source_branch_id)
            .await?
            .into_iter()
            .filter(|s| s.branch_id == input.source_branch_id)
        {
            self.repository
                .upsert_project_quantity_scheme(&project.id, &target_branch_id, &stored.scheme)
                .await?;
        }
        let quantity_state_ids: Vec<String> = self
            .repository
            .list_quantity_states_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?
            .into_iter()
            .filter(|state| state.branch_id == input.source_branch_id)
            .map(|state| state.id)
            .collect();
        self.repository
            .rehome_rows_to_branch("quantity_state", &target_branch_id, &quantity_state_ids)
            .await?;

        Ok(MergeBranchOutput {
            source_branch_id: input.source_branch_id,
            target_branch_id,
            merge_type: input.merge_type,
            applied_scenes: mergeable_scenes.len(),
            applied_character_states: mergeable_states.len(),
            applied_relationships: source_relationships.len(),
            applied_pacing_trackers: source_pacing.len(),
            has_conflicts: !conflicts.is_empty(),
            conflicts,
        })
    }

    /// Rewind the project's active branch to the state captured by
    /// `save_point_id`. Mirrors `restore_save_point` (`services/mod.rs:
    /// 3125..3216` in 705b835^):
    ///
    ///   1. Validate the save_point belongs to the project and to the
    ///      active branch.
    ///   2. Read its snapshot file, verify the recorded sha256.
    ///   3. Parse + validate the JSON payload (`spindle-save-point-v1`).
    ///   4. Build the per-table restore set filtered to the active branch.
    ///   5. Auto-create a `pre-restore-…` backup save_point so the operator
    ///      can undo the restore.
    ///   6. Call `Repository::restore_branch_snapshot`.
    ///   7. Rebuild the project's search index.
    pub async fn restore_save_point(
        &self,
        input: spindle_core::models::RestoreSavePointInput,
    ) -> Result<spindle_core::models::RestoreSavePointOutput> {
        use spindle_core::models::{CreateSavePointInput, RestoreSavePointOutput};

        // 1. Resolve project + active branch + save_point identity.
        let project = self.repository.get_project(&input.project_id).await?;
        let active_branch_id = project
            .active_branch_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?
            .to_string();

        let save_point = self.repository.get_save_point(&input.save_point_id).await?;
        if save_point.project_id != input.project_id {
            anyhow::bail!(
                "save point {} does not belong to project {}",
                save_point.id,
                input.project_id
            );
        }
        if save_point.branch_id != active_branch_id {
            anyhow::bail!(
                "save point belongs to branch {}, but active branch is {}",
                save_point.branch_id,
                active_branch_id
            );
        }

        // 2. Read snapshot bytes and verify sha256.
        let snapshot_relative_path = save_point
            .snapshot_file_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("save point does not have a persisted snapshot file"))?;
        if save_point.snapshot_format.as_deref() != Some("spindle-save-point-v1") {
            anyhow::bail!("save point snapshot format is missing or unsupported");
        }
        let snapshot_path = self.repository.data_dir().join(&snapshot_relative_path);
        let snapshot_bytes = std::fs::read(&snapshot_path).with_context(|| {
            format!(
                "failed to read save point snapshot {}",
                snapshot_path.display()
            )
        })?;
        let snapshot_sha256 = crate::sqlite::import::sha256_hex(&snapshot_bytes);
        if save_point.snapshot_sha256.as_deref() != Some(snapshot_sha256.as_str()) {
            anyhow::bail!("save point snapshot hash does not match persisted metadata");
        }

        // 3. Parse + validate payload.
        let payload: serde_json::Value = serde_json::from_slice(&snapshot_bytes)
            .context("failed to parse save point snapshot json")?;
        validate_save_point_snapshot_payload(
            &payload,
            &input.project_id,
            &input.save_point_id,
            &active_branch_id,
        )?;

        // 4. Build the branch-filtered restore snapshot.
        let restore_snapshot = build_branch_restore_snapshot(
            &self.repository,
            &input.project_id,
            &active_branch_id,
            &payload,
        )
        .await?;
        let restored_tables = count_restore_tables(&restore_snapshot);
        let restored_records = count_restore_records(&restore_snapshot);

        // 5. Auto-create a pre-restore backup save_point.
        let backup = self
            .create_save_point(CreateSavePointInput {
                project_id: input.project_id.clone(),
                name: format!(
                    "pre-restore-{}-{}",
                    slugify_filename_component(&save_point.name),
                    chrono::Utc::now().format("%Y%m%d%H%M%S")
                ),
                description: Some(format!(
                    "Automatic backup before restoring save point {}",
                    save_point.name
                )),
            })
            .await?;

        // 6. Apply the restore.
        self.repository
            .restore_branch_snapshot(&input.project_id, &active_branch_id, &restore_snapshot)
            .await?;

        // 7. Rebuild the search index for the project so the restored
        //    branch's characters/scenes/etc. are reachable via FTS/vec0.
        self.rebuild_search_index(spindle_core::models::RebuildSearchIndexInput {
            project_id: input.project_id.clone(),
        })
        .await?;

        Ok(RestoreSavePointOutput {
            save_point_id: input.save_point_id,
            branch_id: active_branch_id,
            backup_save_point_id: backup.save_point_id,
            status: "restored".to_string(),
            restored_tables,
            restored_records,
        })
    }

    /// Promote a legacy canonical fact to a typed canonical fact via the
    /// upgrade spec. The new fact supersedes the old one, preserving the
    /// canonical-fact audit chain.
    ///
    /// Schema note (v029): the `legacy_untyped` column was dropped from
    /// `canonical_fact` after the SurrealDB reference was written. The
    /// reference rejected migration when `fact.legacy_untyped == false`
    /// ("already typed"). Without the column, that pre-check has no
    /// in-schema source of truth, so the SQLite implementation forwards
    /// any unsuperseded fact through the migration and lets callers re-run
    /// `register_canonical_fact` if they want pure-typed creation. The
    /// `set_canonical_fact_legacy_untyped` call survives as a no-op shim
    /// (see `crate::sqlite::repository::set_canonical_fact_legacy_untyped`).
    pub async fn migrate_canonical_fact(
        &self,
        input: spindle_core::models::MigrateCanonicalFactInput,
    ) -> Result<spindle_core::models::MigrateCanonicalFactOutput> {
        use spindle_core::models::{
            CanonicalFactScope, MigrateCanonicalFactOutput, RegisterCanonicalFactInput,
        };

        let fact = self.repository.get_canonical_fact(&input.fact_id).await?;
        if fact.superseded_by.is_some() {
            anyhow::bail!("canonical fact {} is already superseded", input.fact_id);
        }

        let active_branch = self.repository.get_active_branch(&fact.project_id).await?;
        if fact.branch_id != active_branch.id {
            anyhow::bail!(
                "canonical fact {} is not on the project's active branch",
                input.fact_id
            );
        }

        let subject_id = input
            .upgrade_spec
            .subject_id
            .clone()
            .or_else(|| fact.subject_id.clone());
        let aliases = if input.upgrade_spec.aliases.is_empty() {
            fact.aliases.clone()
        } else {
            input.upgrade_spec.aliases.clone()
        };
        let scope = input
            .upgrade_spec
            .scope
            .clone()
            .or(match fact.scope.as_str() {
                "invariant" => Some(CanonicalFactScope::Invariant),
                "evolving" => Some(CanonicalFactScope::Evolving),
                "conditional" => Some(CanonicalFactScope::Conditional),
                _ => None,
            });
        let valid_from = input
            .upgrade_spec
            .valid_from
            .clone()
            .or_else(|| fact.valid_from.as_ref().map(|p| p.clone().into_core()));
        let valid_until = input
            .upgrade_spec
            .valid_until
            .clone()
            .or_else(|| fact.valid_until.as_ref().map(|p| p.clone().into_core()));

        let migrated = self
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: fact.project_id.clone(),
                scene_id: fact.scene_id.clone(),
                book_number: fact.book_number,
                chapter_number: fact.chapter_number,
                fact_type: Some("typed_fact".to_string()),
                key: Some(input.upgrade_spec.predicate.clone()),
                value: None,
                context: None,
                subject_table: Some(input.upgrade_spec.subject_table.clone()),
                subject_id,
                predicate: Some(input.upgrade_spec.predicate.clone()),
                value_kind: Some(input.upgrade_spec.value_kind.clone()),
                value_text: input.upgrade_spec.value_text.clone(),
                value_number: input.upgrade_spec.value_number,
                value_unit: input.upgrade_spec.value_unit.clone(),
                value_json: input.upgrade_spec.value_json.clone(),
                aliases,
                scope,
                valid_from,
                valid_until,
                legacy_untyped: Some(false),
                supersedes_fact_id: Some(input.fact_id.clone()),
                // Secrecy is a declaration-time concern; the untyped→typed
                // upgrade path does not re-declare it (Part A scope). A secret
                // fact's circle links are on knowledge_fact rows and survive the
                // upgrade independently of this re-register.
                secrecy: None,
            })
            .await?;
        // No-op against the v029+ schema (column dropped); kept for
        // contract-level parity with the SurrealDB reference.
        self.repository
            .set_canonical_fact_legacy_untyped(&input.fact_id, false)
            .await?;

        Ok(MigrateCanonicalFactOutput {
            canonical_fact_id: migrated.canonical_fact_id,
            superseded_fact_id: input.fact_id,
        })
    }
}
