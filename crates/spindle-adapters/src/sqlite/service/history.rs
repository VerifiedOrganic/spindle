use super::*;
use spindle_core::models::{
    ChangeReceipt, ListChangesInput, ListChangesOutput, UndoAffected, UndoChangeInput,
    UndoChangeOutput, UndoSupport,
};

fn micros_to_rfc3339(micros: i64) -> String {
    chrono::DateTime::from_timestamp_micros(micros)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| micros.to_string())
}

impl SqliteSpindleService {
    /// Unified, paginated history over scene versions, branch checkpoints,
    /// style applications, style revision patches, and prior undo records.
    pub async fn list_changes(&self, input: ListChangesInput) -> Result<ListChangesOutput> {
        self.repository.get_project(&input.project_id).await?;
        let branch_id = match input.branch_id {
            Some(branch_id) => {
                let branch = self.repository.get_branch(&branch_id).await?;
                anyhow::ensure!(
                    branch.project_id.as_deref() == Some(input.project_id.as_str()),
                    "branch does not belong to the requested project"
                );
                branch_id
            }
            None => {
                self.repository
                    .active_branch_id_public(&input.project_id)
                    .await?
            }
        };
        let limit = input.limit.unwrap_or(20).clamp(1, 100);
        let offset = input.offset.unwrap_or(0) as usize;
        let kind_filter = input.kind.as_deref();
        let mut changes = Vec::new();

        if kind_filter.is_none_or(|kind| kind == "scene_save") {
            let versions = self
                .repository
                .list_scene_versions_by_project(&input.project_id)
                .await?;
            let mut latest_by_scene: std::collections::BTreeMap<String, i32> =
                std::collections::BTreeMap::new();
            for version in &versions {
                latest_by_scene
                    .entry(version.scene_id.clone())
                    .and_modify(|number| *number = (*number).max(version.version_number))
                    .or_insert(version.version_number);
            }
            for version in versions {
                if version.branch_id != branch_id {
                    continue;
                }
                if input
                    .scene_id
                    .as_deref()
                    .is_some_and(|scene_id| scene_id != version.scene_id)
                {
                    continue;
                }
                let is_current =
                    latest_by_scene.get(&version.scene_id) == Some(&version.version_number);
                changes.push(ChangeReceipt {
                    operation_id: format!("scene_version:{}", version.id),
                    project_id: version.project_id,
                    branch_id: version.branch_id,
                    kind: "scene_save".to_string(),
                    subject_id: version.scene_id,
                    summary: format!(
                        "scene version {} (chapter {}.{}, scene {})",
                        version.version_number,
                        version.book_number,
                        version.chapter_number,
                        version.scene_order
                    ),
                    created_at: version.created_at.to_rfc3339(),
                    undo_support: if is_current {
                        UndoSupport::Irreversible
                    } else {
                        UndoSupport::Reversible
                    },
                    undo_reason: is_current.then(|| {
                        "current scene version; save a new version before undoing".to_string()
                    }),
                    undone: false,
                });
            }
        }

        if kind_filter.is_none_or(|kind| kind == "branch_checkpoint") && input.scene_id.is_none() {
            for save_point in self
                .repository
                .list_save_points_by_project_and_branch(&input.project_id, &branch_id)
                .await?
            {
                changes.push(ChangeReceipt {
                    operation_id: format!("save_point:{}", save_point.id),
                    project_id: save_point.project_id,
                    branch_id: save_point.branch_id,
                    kind: "branch_checkpoint".to_string(),
                    subject_id: save_point.id.clone(),
                    summary: save_point.name,
                    created_at: save_point.created_at.to_rfc3339(),
                    undo_support: UndoSupport::CheckpointOnly,
                    undo_reason: Some(
                        "branch snapshot; use restore_save_point with the original checkpoint scope"
                            .to_string(),
                    ),
                    undone: false,
                });
            }
        }

        let mut undo_records = self
            .repository
            .list_history_undo_by_project_and_branch(&input.project_id, &branch_id)
            .await?;
        let undone_sources: std::collections::BTreeSet<String> = undo_records
            .iter()
            .map(|record| record.source_operation_id.clone())
            .collect();

        if kind_filter.is_none_or(|kind| kind == "style_apply") && input.scene_id.is_none() {
            for app in self
                .repository
                .list_style_profile_applications(&input.project_id)
                .await?
            {
                let reversible = app.rollback_status == "applied";
                changes.push(ChangeReceipt {
                    operation_id: format!("style_application:{}", app.id),
                    project_id: app.project_id.clone(),
                    branch_id: branch_id.clone(),
                    kind: "style_apply".to_string(),
                    subject_id: app.profile_id.clone(),
                    summary: format!("style application {}", app.id),
                    created_at: app.applied_at,
                    undo_support: if reversible {
                        UndoSupport::Reversible
                    } else {
                        UndoSupport::Irreversible
                    },
                    undo_reason: (!reversible)
                        .then(|| format!("already rolled back ({})", app.rollback_status)),
                    undone: !reversible
                        || undone_sources.contains(&format!("style_application:{}", app.id)),
                });
            }
        }

        if kind_filter.is_none_or(|kind| kind == "style_patch") && input.scene_id.is_none() {
            for audit in self
                .repository
                .list_style_revision_patch_audits(&input.project_id)
                .await?
            {
                let reversible = audit.rollback_status == "applied";
                changes.push(ChangeReceipt {
                    operation_id: format!("style_patch:{}", audit.id),
                    project_id: audit.project_id.clone(),
                    branch_id: branch_id.clone(),
                    kind: "style_patch".to_string(),
                    subject_id: audit.profile_id.clone(),
                    summary: format!(
                        "style revision patch {} ({} targets)",
                        audit.id,
                        audit.target_ids.len()
                    ),
                    created_at: audit.applied_at,
                    undo_support: if reversible {
                        UndoSupport::Reversible
                    } else {
                        UndoSupport::Irreversible
                    },
                    undo_reason: (!reversible)
                        .then(|| format!("already rolled back ({})", audit.rollback_status)),
                    undone: !reversible
                        || undone_sources.contains(&format!("style_patch:{}", audit.id)),
                });
            }
        }

        if kind_filter.is_none_or(|kind| kind == "undo") && input.scene_id.is_none() {
            for record in undo_records.drain(..) {
                changes.push(ChangeReceipt {
                    operation_id: record.id.clone(),
                    project_id: record.project_id,
                    branch_id: record.branch_id,
                    kind: "undo".to_string(),
                    subject_id: record.source_operation_id,
                    summary: record.summary,
                    created_at: micros_to_rfc3339(record.created_at),
                    undo_support: UndoSupport::Irreversible,
                    undo_reason: Some("undo records are audit history".to_string()),
                    undone: false,
                });
            }
        }

        changes.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.operation_id.cmp(&b.operation_id))
        });
        let total = changes.len();
        let page: Vec<_> = changes
            .into_iter()
            .skip(offset)
            .take(limit as usize)
            .collect();
        let next_offset = (offset + page.len() < total).then_some((offset + page.len()) as u32);
        Ok(ListChangesOutput {
            changes: page,
            next_offset,
        })
    }

    /// Undo one recorded operation with an optimistic head check. Unsupported
    /// scopes (branch checkpoints) are refused explicitly rather than pretending
    /// their ids are single-record operations.
    pub async fn undo_change(&self, input: UndoChangeInput) -> Result<UndoChangeOutput> {
        self.repository.get_project(&input.project_id).await?;
        let Some((prefix, id)) = input.operation_id.split_once(':') else {
            anyhow::bail!(
                "operation_id must be prefix-qualified; supported: scene_version, style_application, style_patch"
            );
        };
        match prefix {
            "scene_version" => {
                let version = self.repository.get_scene_version(id).await?;
                anyhow::ensure!(
                    version.project_id == input.project_id,
                    "scene version does not belong to the requested project"
                );
                let versions = self
                    .repository
                    .list_scene_versions(&version.scene_id)
                    .await?;
                let current = versions
                    .iter()
                    .max_by_key(|version| version.version_number)
                    .ok_or_else(|| anyhow::anyhow!("scene has no versions"))?;
                anyhow::ensure!(
                    current.id == input.expected_head,
                    "stale expected_head: current scene version is {}",
                    current.id
                );
                anyhow::ensure!(
                    current.id != version.id,
                    "operation is already the current scene version"
                );
                let affected = vec![
                    UndoAffected {
                        table: "scene".to_string(),
                        id: version.scene_id.clone(),
                        summary: format!("restore scene version {}", version.version_number),
                    },
                    UndoAffected {
                        table: "scene_version".to_string(),
                        id: version.id.clone(),
                        summary: "source version".to_string(),
                    },
                ];
                if input.dry_run {
                    return Ok(UndoChangeOutput {
                        operation_id: input.operation_id,
                        kind: "scene_save".to_string(),
                        project_id: input.project_id,
                        branch_id: version.branch_id,
                        affected,
                        compensating_operation_id: None,
                        dry_run: true,
                        message: "dry run: would restore the prior scene text and derived state"
                            .to_string(),
                    });
                }
                let restored = self
                    .restore_scene_version(RestoreSceneVersionInput {
                        project_id: input.project_id.clone(),
                        scene_id: version.scene_id.clone(),
                        scene_version_id: version.id.clone(),
                    })
                    .await?;
                let latest = self
                    .repository
                    .list_scene_versions(&version.scene_id)
                    .await?
                    .into_iter()
                    .max_by_key(|version| version.version_number)
                    .map(|version| format!("scene_version:{}", version.id));
                let undo_id = self
                    .repository
                    .append_history_undo(
                        &input.project_id,
                        &version.branch_id,
                        &input.operation_id,
                        latest.as_deref(),
                        "scene_save",
                        &format!(
                            "restored scene version {} ({})",
                            version.version_number, restored.status
                        ),
                    )
                    .await?;
                Ok(UndoChangeOutput {
                    operation_id: input.operation_id,
                    kind: "scene_save".to_string(),
                    project_id: input.project_id,
                    branch_id: version.branch_id,
                    affected,
                    compensating_operation_id: latest,
                    dry_run: false,
                    message: format!("scene restored; undo audit recorded as {undo_id}"),
                })
            }
            "style_application" => {
                let apps = self
                    .repository
                    .list_style_profile_applications(&input.project_id)
                    .await?;
                let app = apps
                    .iter()
                    .find(|app| app.id == id)
                    .ok_or_else(|| anyhow::anyhow!("style application not found"))?;
                anyhow::ensure!(
                    app.rollback_status == "applied",
                    "style application is not reversible ({})",
                    app.rollback_status
                );
                let latest = apps
                    .iter()
                    .max_by(|a, b| a.applied_at.cmp(&b.applied_at))
                    .expect("non-empty style applications");
                anyhow::ensure!(
                    latest.id == input.expected_head,
                    "stale expected_head: latest style application is {}",
                    latest.id
                );
                let branch_id = self
                    .repository
                    .active_branch_id_public(&input.project_id)
                    .await?;
                let affected = vec![UndoAffected {
                    table: "style_profile_application".to_string(),
                    id: app.id.clone(),
                    summary: "restore narrator voice, style notes, and style rule".to_string(),
                }];
                if input.dry_run {
                    return Ok(UndoChangeOutput {
                        operation_id: input.operation_id,
                        kind: "style_apply".to_string(),
                        project_id: input.project_id,
                        branch_id,
                        affected,
                        compensating_operation_id: None,
                        dry_run: true,
                        message: "dry run: would roll back the captured style effects".to_string(),
                    });
                }
                let rolled = self
                    .rollback_style_profile_application(
                        spindle_core::style::RollbackStyleProfileApplicationInput {
                            project_id: input.project_id.clone(),
                            application_id: app.id.clone(),
                        },
                    )
                    .await?;
                let compensating = format!("style_application_rollback:{}", app.id);
                self.repository
                    .append_history_undo(
                        &input.project_id,
                        &branch_id,
                        &input.operation_id,
                        Some(&compensating),
                        "style_apply",
                        &format!("rolled back style application {}", app.id),
                    )
                    .await?;
                Ok(UndoChangeOutput {
                    operation_id: input.operation_id,
                    kind: "style_apply".to_string(),
                    project_id: input.project_id,
                    branch_id,
                    affected,
                    compensating_operation_id: Some(compensating),
                    dry_run: false,
                    message: format!(
                        "style rollback recorded; {} findings invalidated",
                        rolled.invalidated_validator_findings
                    ),
                })
            }
            "style_patch" => {
                let audits = self
                    .repository
                    .list_style_revision_patch_audits(&input.project_id)
                    .await?;
                let audit = audits
                    .iter()
                    .find(|audit| audit.id == id)
                    .ok_or_else(|| anyhow::anyhow!("style revision patch audit not found"))?;
                anyhow::ensure!(
                    audit.rollback_status == "applied",
                    "style revision patch is not reversible ({})",
                    audit.rollback_status
                );
                let latest = audits
                    .iter()
                    .max_by(|a, b| a.applied_at.cmp(&b.applied_at))
                    .expect("non-empty style audits");
                anyhow::ensure!(
                    latest.id == input.expected_head,
                    "stale expected_head: latest style patch is {}",
                    latest.id
                );
                let branch_id = self
                    .repository
                    .active_branch_id_public(&input.project_id)
                    .await?;
                let affected = vec![UndoAffected {
                    table: "style_revision_patch_audit".to_string(),
                    id: audit.id.clone(),
                    summary: format!("restore {} patched scenes", audit.target_ids.len()),
                }];
                if input.dry_run {
                    return Ok(UndoChangeOutput {
                        operation_id: input.operation_id,
                        kind: "style_patch".to_string(),
                        project_id: input.project_id,
                        branch_id,
                        affected,
                        compensating_operation_id: None,
                        dry_run: true,
                        message: "dry run: would restore the captured scene text".to_string(),
                    });
                }
                let rolled = self
                    .rollback_style_revision_patch(
                        spindle_core::style::RollbackStyleRevisionPatchInput {
                            project_id: input.project_id.clone(),
                            audit_id: audit.id.clone(),
                        },
                    )
                    .await?;
                let compensating = format!("style_patch_rollback:{}", audit.id);
                self.repository
                    .append_history_undo(
                        &input.project_id,
                        &branch_id,
                        &input.operation_id,
                        Some(&compensating),
                        "style_patch",
                        &format!("rolled back style revision patch {}", audit.id),
                    )
                    .await?;
                Ok(UndoChangeOutput {
                    operation_id: input.operation_id,
                    kind: "style_patch".to_string(),
                    project_id: input.project_id,
                    branch_id,
                    affected,
                    compensating_operation_id: Some(compensating),
                    dry_run: false,
                    message: format!(
                        "style patch rollback recorded; {} scenes restored",
                        rolled.restored_scene_ids.len()
                    ),
                })
            }
            "save_point" => anyhow::bail!(
                "checkpoint_only: save_point:{id} restores a whole branch snapshot; use restore_save_point with the checkpoint's original scope"
            ),
            _ => anyhow::bail!(
                "unsupported history scope '{prefix}'; supported: scene_version, style_application, style_patch"
            ),
        }
    }
}
