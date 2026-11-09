use super::*;
use spindle_core::models::{
    ChangeReceipt, ListChangesInput, ListChangesOutput, UndoChangeInput, UndoChangeOutput,
    UndoSupport,
};

fn micros_to_rfc3339(micros: i64) -> String {
    chrono::DateTime::from_timestamp_micros(micros)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| micros.to_string())
}

impl SqliteSpindleService {
    /// Unified, paginated discovery over scene versions, branch checkpoints,
    /// style applications, and style revision patches. Each receipt states
    /// whether the existing explicit revision tool is safe to use.
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
            for version in self
                .repository
                .list_scene_versions_by_project(&input.project_id)
                .await?
            {
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
                        "snapshot preserves prose but not approved continuity deltas; review and use restore_scene_version explicitly"
                            .to_string(),
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
                    created_at: micros_to_rfc3339(save_point.created_at.timestamp_micros()),
                    undo_support: UndoSupport::CheckpointOnly,
                    undo_reason: Some(
                        "branch snapshot; use restore_save_point with the original checkpoint scope"
                            .to_string(),
                    ),
                    undone: false,
                });
            }
        }

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
                    undo_support: UndoSupport::CheckpointOnly,
                    undo_reason: Some(if reversible {
                        "multi-step style rollback; use rollback_style_profile_application explicitly"
                            .to_string()
                    } else {
                        format!("already rolled back ({})", app.rollback_status)
                    }),
                    undone: !reversible,
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
                    undo_support: UndoSupport::CheckpointOnly,
                    undo_reason: Some(if reversible {
                        "multi-scene style patch rollback; use rollback_style_revision_patch explicitly"
                            .to_string()
                    } else {
                        format!("already rolled back ({})", audit.rollback_status)
                    }),
                    undone: !reversible,
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

    /// Unified undo is a scope check, not a mutating adapter yet: the existing
    /// scene/style revivers are multi-step and are not atomic with their own
    /// audits, so this surface refuses to pretend a single operation id can
    /// safely replay them. Callers use the named tool after reviewing effects.
    pub async fn undo_change(&self, input: UndoChangeInput) -> Result<UndoChangeOutput> {
        self.repository.get_project(&input.project_id).await?;
        let Some((prefix, _id)) = input.operation_id.split_once(':') else {
            anyhow::bail!(
                "operation_id must be prefix-qualified; supported scopes: scene_version, save_point, style_application, style_patch"
            );
        };
        match prefix {
            "scene_version" => anyhow::bail!(
                "checkpoint_only: scene snapshots do not capture approved continuity deltas; review them and use restore_scene_version explicitly"
            ),
            "save_point" => anyhow::bail!(
                "checkpoint_only: use restore_save_point with the checkpoint's original branch scope"
            ),
            "style_application" => anyhow::bail!(
                "checkpoint_only: style rollback is multi-step; use rollback_style_profile_application explicitly"
            ),
            "style_patch" => anyhow::bail!(
                "checkpoint_only: style patch rollback restores multiple scenes; use rollback_style_revision_patch explicitly"
            ),
            _ => anyhow::bail!(
                "unsupported history scope '{prefix}'; supported: scene_version, save_point, style_application, style_patch"
            ),
        }
    }
}
