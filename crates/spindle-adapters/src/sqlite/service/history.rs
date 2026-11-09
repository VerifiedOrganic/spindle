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
                    undo_support: UndoSupport::CheckpointOnly,
                    undo_reason: Some(
                        "scene snapshots preserve prose only; approved continuity deltas require explicit review before restore_scene_version".to_string(),
                    ),
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
                let reversible = app.rollback_status != "rolled_back";
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
                let reversible = audit.rollback_status != "rolled_back";
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
                    undo_reason: Some(if record.status == "completed" {
                        "undo records are audit history".to_string()
                    } else {
                        format!(
                            "undo audit is {}; retry undo_change to reconcile",
                            record.status
                        )
                    }),
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
            "scene_version" => anyhow::bail!(
                "checkpoint_only: scene_version:{id} restores prose but not the approved continuity state; review the deltas and use restore_scene_version explicitly"
            ),
            "style_application" => {
                let apps = self
                    .repository
                    .list_style_profile_applications(&input.project_id)
                    .await?;
                let app = apps
                    .iter()
                    .find(|app| app.id == id)
                    .ok_or_else(|| anyhow::anyhow!("style application not found"))?;
                let existing = self
                    .repository
                    .find_history_undo_by_source(&input.project_id, &input.operation_id)
                    .await?;
                if app.rollback_status == "rolled_back" {
                    if let Some(existing) = existing.clone() {
                        if existing.status != "completed" {
                            self.repository.complete_history_undo(&existing.id).await?;
                        }
                        return Ok(UndoChangeOutput {
                            operation_id: input.operation_id,
                            kind: "style_apply".to_string(),
                            project_id: input.project_id,
                            branch_id: existing.branch_id,
                            affected: vec![UndoAffected {
                                table: "style_profile_application".to_string(),
                                id: app.id.clone(),
                                summary: "already rolled back; undo audit reconciled".to_string(),
                            }],
                            compensating_operation_id: existing.compensating_operation_id,
                            dry_run: false,
                            message:
                                "style application was already rolled back; undo audit completed"
                                    .to_string(),
                        });
                    }
                    anyhow::bail!(
                        "style application is not reversible ({})",
                        app.rollback_status
                    );
                }
                let head = apps
                    .iter()
                    .filter(|candidate| candidate.rollback_status != "rolled_back")
                    .max_by(|a, b| a.applied_at.cmp(&b.applied_at))
                    .expect("at least one applied style application");
                anyhow::ensure!(
                    app.id == head.id,
                    "style application is not the current applied head; undo {} first",
                    head.id
                );
                anyhow::ensure!(
                    input.expected_head == head.id,
                    "stale expected_head: current applied style application is {}",
                    head.id
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
                let compensating = format!("style_application_rollback:{}", app.id);
                let undo_id = match existing {
                    Some(existing) if existing.status != "completed" => existing.id,
                    _ => {
                        self.repository
                            .begin_history_undo(
                                &input.project_id,
                                &branch_id,
                                &input.operation_id,
                                Some(&compensating),
                                "style_apply",
                                &format!("rolled back style application {}", app.id),
                            )
                            .await?
                    }
                };
                let rolled = match self
                    .rollback_style_profile_application(
                        spindle_core::style::RollbackStyleProfileApplicationInput {
                            project_id: input.project_id.clone(),
                            application_id: app.id.clone(),
                        },
                    )
                    .await
                {
                    Ok(rolled) => rolled,
                    Err(error) => {
                        let _ = self.repository.fail_history_undo(&undo_id).await;
                        return Err(error);
                    }
                };
                let audit_pending = self
                    .repository
                    .complete_history_undo(&undo_id)
                    .await
                    .is_err();
                Ok(UndoChangeOutput {
                    operation_id: input.operation_id,
                    kind: "style_apply".to_string(),
                    project_id: input.project_id,
                    branch_id,
                    affected,
                    compensating_operation_id: Some(compensating),
                    dry_run: false,
                    message: if audit_pending {
                        format!(
                            "style rollback applied; undo audit {} is pending recovery; {} findings invalidated",
                            undo_id, rolled.invalidated_validator_findings
                        )
                    } else {
                        format!(
                            "style rollback recorded; {} findings invalidated",
                            rolled.invalidated_validator_findings
                        )
                    },
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
                let existing = self
                    .repository
                    .find_history_undo_by_source(&input.project_id, &input.operation_id)
                    .await?;
                if audit.rollback_status == "rolled_back" {
                    if let Some(existing) = existing.clone() {
                        if existing.status != "completed" {
                            self.repository.complete_history_undo(&existing.id).await?;
                        }
                        return Ok(UndoChangeOutput {
                            operation_id: input.operation_id,
                            kind: "style_patch".to_string(),
                            project_id: input.project_id,
                            branch_id: existing.branch_id,
                            affected: vec![UndoAffected {
                                table: "style_revision_patch_audit".to_string(),
                                id: audit.id.clone(),
                                summary: "already rolled back; undo audit reconciled".to_string(),
                            }],
                            compensating_operation_id: existing.compensating_operation_id,
                            dry_run: false,
                            message: "style patch was already rolled back; undo audit completed"
                                .to_string(),
                        });
                    }
                    anyhow::bail!(
                        "style revision patch is not reversible ({})",
                        audit.rollback_status
                    );
                }
                let head = audits
                    .iter()
                    .filter(|candidate| candidate.rollback_status != "rolled_back")
                    .max_by(|a, b| a.applied_at.cmp(&b.applied_at))
                    .expect("at least one applied style patch");
                anyhow::ensure!(
                    audit.id == head.id,
                    "style revision patch is not the current applied head; undo {} first",
                    head.id
                );
                anyhow::ensure!(
                    input.expected_head == head.id,
                    "stale expected_head: current applied style patch is {}",
                    head.id
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
                let compensating = format!("style_patch_rollback:{}", audit.id);
                let undo_id = match existing {
                    Some(existing) if existing.status != "completed" => existing.id,
                    _ => {
                        self.repository
                            .begin_history_undo(
                                &input.project_id,
                                &branch_id,
                                &input.operation_id,
                                Some(&compensating),
                                "style_patch",
                                &format!("rolled back style revision patch {}", audit.id),
                            )
                            .await?
                    }
                };
                let rolled = match self
                    .rollback_style_revision_patch(
                        spindle_core::style::RollbackStyleRevisionPatchInput {
                            project_id: input.project_id.clone(),
                            audit_id: audit.id.clone(),
                        },
                    )
                    .await
                {
                    Ok(rolled) => rolled,
                    Err(error) => {
                        let _ = self.repository.fail_history_undo(&undo_id).await;
                        return Err(error);
                    }
                };
                let audit_pending = self
                    .repository
                    .complete_history_undo(&undo_id)
                    .await
                    .is_err();
                Ok(UndoChangeOutput {
                    operation_id: input.operation_id,
                    kind: "style_patch".to_string(),
                    project_id: input.project_id,
                    branch_id,
                    affected,
                    compensating_operation_id: Some(compensating),
                    dry_run: false,
                    message: if audit_pending {
                        format!(
                            "style patch rollback applied; undo audit {} is pending recovery; {} scenes restored",
                            undo_id,
                            rolled.restored_scene_ids.len()
                        )
                    } else {
                        format!(
                            "style patch rollback recorded; {} scenes restored",
                            rolled.restored_scene_ids.len()
                        )
                    },
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
