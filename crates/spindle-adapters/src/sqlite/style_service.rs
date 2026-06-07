use crate::sqlite::SqliteSpindleService;
use crate::sqlite::service::PhaseFourCacheId;
use crate::sqlite::style_helper;
use anyhow::{Result, anyhow};
use sha2::Digest;
use spindle_core::models::{CreateWorldRuleInput, UpdateWorldRuleInput};
use spindle_core::style::{
    ApplyStyleProfileInput, ApplyStyleProfileOutput, CreateStyleProfileFromMarkdownInput,
    CreateStyleProfileFromMarkdownOutput, GetStyleProfileInput, GetStyleProfileOutput,
    ListStyleProfilesInput, ListStyleProfilesOutput, StyleCorpusSummary, StyleProfileApplyMode,
    StyleProfileCard, StyleProfileGuidance, StyleProfileModelReceipt, StyleProfileSourcePolicy,
    StyleProfileStatus, StyleSourceRef,
};
use std::fs;

pub const STYLE_ANALYZE_ROUTE: &str = "style_analyze";
const MAX_STYLE_SYNTHESIS_SOURCE_WORDS: usize = 12_000;

fn clean_json_response(raw: &str) -> &str {
    let mut s = raw.trim();
    if s.starts_with("```json") {
        s = s.strip_prefix("```json").unwrap_or(s);
    } else if s.starts_with("```") {
        s = s.strip_prefix("```").unwrap_or(s);
    }
    if s.ends_with("```") {
        s = s.strip_suffix("```").unwrap_or(s);
    }
    s.trim()
}

fn trim_to_word_limit(text: &str, max_words: usize) -> String {
    if max_words == 0 {
        return String::new();
    }
    let mut words = text.split_whitespace();
    let mut trimmed = Vec::new();
    for _ in 0..max_words {
        let Some(word) = words.next() else {
            break;
        };
        trimmed.push(word);
    }
    trimmed.join(" ")
}

impl SqliteSpindleService {
    pub async fn create_style_profile_from_markdown(
        &self,
        input: CreateStyleProfileFromMarkdownInput,
    ) -> Result<CreateStyleProfileFromMarkdownOutput> {
        // 1. Validate project exists
        let _project = self.repository().get_project(&input.project_id).await?;

        // 2. Resolve allowed roots
        let data_dir = self.repository().data_dir().to_path_buf();
        let mut allowed_roots = vec![data_dir.clone()];
        if let Some(parent) = data_dir.parent() {
            allowed_roots.push(parent.to_path_buf());
        }

        // 3. Collect Markdown files
        let max_files = input.max_files.unwrap_or(100);
        let max_bytes = input.max_bytes_per_file.unwrap_or(10 * 1024 * 1024); // 10MB
        let max_words = input.max_total_words.unwrap_or(200_000);

        let md_files = style_helper::collect_markdown_files(
            &input.source_paths,
            input.recursive.unwrap_or(false),
            &allowed_roots,
            max_files,
            input.include_globs.as_deref(),
            input.exclude_globs.as_deref(),
        )?;

        if md_files.is_empty() {
            return Err(anyhow!("no analyzable Markdown files found"));
        }

        // 4. Process files
        let mut source_refs = Vec::new();
        let mut all_elements = Vec::new();
        let mut source_elements = Vec::new();
        let mut total_words = 0;
        let mut total_characters = 0;
        let mut analyzed_source_count = 0;
        let mut skipped_source_count = 0;

        for path in &md_files {
            let display_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            let canonical_path_str = path.to_string_lossy().to_string();

            let metadata = match fs::metadata(path) {
                Ok(m) => m,
                Err(_) => {
                    skipped_source_count += 1;
                    source_refs.push(StyleSourceRef {
                        display_name,
                        canonical_path: canonical_path_str,
                        sha256: "".to_string(),
                        word_count: 0,
                        included: false,
                        skip_reason: Some("failed to read metadata".to_string()),
                    });
                    continue;
                }
            };

            if !metadata.is_file() {
                skipped_source_count += 1;
                source_refs.push(StyleSourceRef {
                    display_name,
                    canonical_path: canonical_path_str,
                    sha256: "".to_string(),
                    word_count: 0,
                    included: false,
                    skip_reason: Some("not a regular file".to_string()),
                });
                continue;
            }

            if metadata.len() > max_bytes as u64 {
                skipped_source_count += 1;
                source_refs.push(StyleSourceRef {
                    display_name,
                    canonical_path: canonical_path_str,
                    sha256: "".to_string(),
                    word_count: 0,
                    included: false,
                    skip_reason: Some("file size exceeds limit".to_string()),
                });
                continue;
            }

            let bytes = match fs::read(path) {
                Ok(b) => b,
                Err(_) => {
                    skipped_source_count += 1;
                    source_refs.push(StyleSourceRef {
                        display_name,
                        canonical_path: canonical_path_str,
                        sha256: "".to_string(),
                        word_count: 0,
                        included: false,
                        skip_reason: Some("failed to read content".to_string()),
                    });
                    continue;
                }
            };
            let sha256_hash = format!("{:x}", sha2::Sha256::digest(&bytes));

            let text = match String::from_utf8(bytes) {
                Ok(t) => t,
                Err(_) => {
                    skipped_source_count += 1;
                    source_refs.push(StyleSourceRef {
                        display_name,
                        canonical_path: canonical_path_str,
                        sha256: sha256_hash,
                        word_count: 0,
                        included: false,
                        skip_reason: Some("not valid UTF-8".to_string()),
                    });
                    continue;
                }
            };
            let word_count = text.split_whitespace().count();

            if total_words + word_count > max_words {
                skipped_source_count += 1;
                source_refs.push(StyleSourceRef {
                    display_name,
                    canonical_path: canonical_path_str,
                    sha256: sha256_hash,
                    word_count,
                    included: false,
                    skip_reason: Some("exceeds max_total_words limit".to_string()),
                });
                continue;
            }

            let normalized = style_helper::normalize_markdown(&text);
            let elements = style_helper::parse_elements(&normalized);

            all_elements.extend(elements.clone());
            source_elements.push((display_name.clone(), elements));
            total_words += word_count;
            total_characters += text.len();
            analyzed_source_count += 1;

            source_refs.push(StyleSourceRef {
                display_name,
                canonical_path: canonical_path_str,
                sha256: sha256_hash,
                word_count,
                included: true,
                skip_reason: None,
            });
        }

        if analyzed_source_count == 0 {
            return Err(anyhow!("no analyzable Markdown files found"));
        }

        // 5. Chunk elements and compute metrics
        let chunks = source_elements
            .iter()
            .flat_map(|(display_name, elements)| {
                style_helper::chunk_elements(elements, display_name, total_words)
            })
            .collect::<Vec<_>>();
        let chunk_count = chunks.len();
        let metrics = style_helper::compute_metrics(&all_elements);

        let mut warnings = Vec::new();
        if total_words < 3000 {
            warnings.push("The corpus is thin (under 3,000 words). Style metrics and synthesis may be less reliable.".to_string());
        }
        if metrics.dialogue_line_ratio == 0.0 {
            warnings.push("No dialogue detected in the corpus. If this project contains dialogue, the style profile will not reflect it.".to_string());
        }

        let corpus_summary = StyleCorpusSummary {
            source_count: md_files.len(),
            analyzed_source_count,
            skipped_source_count,
            total_words,
            total_characters,
            chunk_count,
            source_refs,
            warnings,
        };

        // 6. Run style synthesis via routed model
        let metrics_only = input.metrics_only.unwrap_or(false);
        let max_synthesis_words = input
            .source_sample_word_budget
            .unwrap_or(MAX_STYLE_SYNTHESIS_SOURCE_WORDS);

        let mut prompt_chunks = Vec::new();
        let mut prompt_source_words = 0usize;
        if !metrics_only {
            for (i, c) in chunks.iter().enumerate() {
                if prompt_source_words >= max_synthesis_words {
                    break;
                }
                let remaining_words = max_synthesis_words - prompt_source_words;
                let text = trim_to_word_limit(&c.text, remaining_words);
                prompt_source_words += style_helper::count_words(&text);
                prompt_chunks.push(format!(
                    "--- Chunk {} (Source: {}) ---\n{}",
                    i + 1,
                    c.source_display_name,
                    text
                ));
            }
        }
        let chunks_str = prompt_chunks.join("\n\n");

        let corpus_section = if metrics_only {
            "[METRICS ONLY MODE]\n\
             This analysis is running in metrics-only/privacy-preserving mode. No raw source prose is provided. \
             Synthesize the style profile and do/avoid guidance using ONLY the deterministic statistics and metrics provided above."
                .to_string()
        } else {
            format!(
                "[CORPUS CHUNKS]\n\
                 Here are representative source samples for style analysis, capped to avoid unbounded prompts. \
                 Use them only to derive abstract guidance; do not quote or reuse source phrasing in output.\n\n\
                 {}",
                chunks_str
            )
        };

        let prompt_text = format!(
            "Analyze the prose style of this user-provided local Markdown corpus.\n\
             Generate abstract prose guidance, NOT author imitation or cloning.\n\
             Do not copy or quote long passages of the source text.\n\n\
             [DETERMINISTIC STYLE METRICS]\n\
             - Word count: {}\n\
             - Average sentence length: {:.2} words (median: {:.2}, p90: {:.2})\n\
             - Average paragraph length: {:.2} words (median: {:.2})\n\
             - Dialogue line ratio: {:.2} (dialogue word ratio: {:.2})\n\
             - Punctuation rates per 1k words:\n\
               - Question marks: {:.2}\n\
               - Exclamation marks: {:.2}\n\
               - Semicolons: {:.2}\n\
               - Em dashes: {:.2}\n\
               - Ellipses: {:.2}\n\
             - Pronoun rates per 1k words:\n\
               - First-person: {:.2}\n\
               - Third-person: {:.2}\n\
             - Top repeated functional markers: {}\n\n\
             {}\n\n\
             Output a single JSON object strictly matching the StyleProfileGuidance schema.",
            total_words,
            metrics.average_sentence_words,
            metrics.median_sentence_words,
            metrics.p90_sentence_words,
            metrics.average_paragraph_words,
            metrics.median_paragraph_words,
            metrics.dialogue_line_ratio,
            metrics.dialogue_word_ratio,
            metrics.question_mark_rate_per_1k_words,
            metrics.exclamation_rate_per_1k_words,
            metrics.semicolon_rate_per_1k_words,
            metrics.em_dash_rate_per_1k_words,
            metrics.ellipsis_rate_per_1k_words,
            metrics.first_person_pronoun_rate_per_1k_words,
            metrics.third_person_pronoun_rate_per_1k_words,
            metrics.top_functional_markers.join(", "),
            corpus_section
        );

        let req = crate::ai::ModelRequest {
            route: STYLE_ANALYZE_ROUTE.to_string(),
            prompt: prompt_text.clone(),
            rating: None,
            context: Some(crate::ai::RequestContext {
                project_id: Some(input.project_id.clone()),
                book_id: None,
                chapter_id: None,
                scene_id: None,
            }),
        };

        let start_time = std::time::Instant::now();
        let response_res = self.repository().model_router().complete(&req).await;
        let latency = start_time.elapsed().as_millis() as u64;

        let status;
        let mut final_guidance = StyleProfileGuidance::default();
        let mut receipt = None;

        if let Ok(response) = response_res {
            receipt = Some(StyleProfileModelReceipt {
                model_route: STYLE_ANALYZE_ROUTE.to_string(),
                model_name: response.model_name.clone(),
                input_tokens: None,
                output_tokens: None,
                latency_ms: Some(latency),
            });

            let cleaned = clean_json_response(&response.output);
            let mut guidance_res: Result<StyleProfileGuidance, _> = serde_json::from_str(cleaned);

            if guidance_res.is_err() {
                // Run repair prompt once
                let repair_prompt = format!(
                    "Your prior response was invalid JSON or did not match the StyleProfileGuidance schema. Here is the error: {}\n\n\
                     Prior response:\n{}\n\n\
                     Please repair the JSON and output ONLY a valid JSON object matching the schema. No markdown wrapping.",
                    guidance_res.as_ref().err().unwrap(),
                    response.output
                );
                let repair_req = crate::ai::ModelRequest {
                    route: STYLE_ANALYZE_ROUTE.to_string(),
                    prompt: repair_prompt,
                    rating: None,
                    context: Some(crate::ai::RequestContext {
                        project_id: Some(input.project_id.clone()),
                        book_id: None,
                        chapter_id: None,
                        scene_id: None,
                    }),
                };
                if let Ok(repair_response) =
                    self.repository().model_router().complete(&repair_req).await
                {
                    let cleaned_repair = clean_json_response(&repair_response.output);
                    guidance_res = serde_json::from_str(cleaned_repair);
                }
            }

            match guidance_res {
                Ok(g) => {
                    status = StyleProfileStatus::Ready;
                    final_guidance = g;
                }
                Err(_) => {
                    status = StyleProfileStatus::NeedsReview;
                }
            }
        } else {
            status = StyleProfileStatus::NeedsReview;
        }

        let profile_id = format!("style_profile:{}", ulid::Ulid::new());
        let now_str = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let source_policy = StyleProfileSourcePolicy {
            local_user_provided: true,
            source_text_persisted: false,
            max_excerpt_words: 0,
            allowed_roots: allowed_roots
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
        };

        let profile = StyleProfileCard {
            profile_id,
            project_id: input.project_id.clone(),
            name: input.profile_name.clone(),
            status,
            created_at: now_str.clone(),
            updated_at: now_str,
            corpus: corpus_summary,
            metrics,
            guidance: final_guidance,
            source_policy,
            model_receipt: receipt,
        };

        // 7. Persist profile
        self.repository().insert_style_profile(&profile).await?;

        // 8. Optionally apply profile
        let mut applied = false;
        let mut application = None;
        if input.apply.unwrap_or(false) {
            let app_out = self
                .apply_style_profile(ApplyStyleProfileInput {
                    project_id: input.project_id.clone(),
                    profile_id: profile.profile_id.clone(),
                    mode: input
                        .application_mode
                        .unwrap_or(StyleProfileApplyMode::Merge),
                })
                .await?;
            applied = true;
            application = Some(app_out);
        }

        Ok(CreateStyleProfileFromMarkdownOutput {
            profile,
            applied,
            application,
        })
    }

    pub async fn list_style_profiles(
        &self,
        input: ListStyleProfilesInput,
    ) -> Result<ListStyleProfilesOutput> {
        self.repository().get_project(&input.project_id).await?;
        let profiles = self
            .repository()
            .list_style_profiles(&input.project_id)
            .await?;
        Ok(ListStyleProfilesOutput { profiles })
    }

    pub async fn get_style_profile(
        &self,
        input: GetStyleProfileInput,
    ) -> Result<GetStyleProfileOutput> {
        let profile = self
            .repository()
            .get_style_profile(&input.project_id, &input.profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found"))?;
        Ok(GetStyleProfileOutput { profile })
    }

    pub async fn apply_style_profile(
        &self,
        input: ApplyStyleProfileInput,
    ) -> Result<ApplyStyleProfileOutput> {
        // 1. Fetch style profile
        let profile = self
            .repository()
            .get_style_profile(&input.project_id, &input.profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found"))?;
        if profile.status != StyleProfileStatus::Ready {
            return Err(anyhow!(
                "style profile is not ready to apply: {:?}",
                profile.status
            ));
        }
        if !has_application_guidance(&profile.guidance) {
            return Err(anyhow!(
                "style profile has no application guidance to apply: {}",
                profile.profile_id
            ));
        }

        // Fetch project state
        let project = self.repository().get_project(&input.project_id).await?;
        let current_narrator_voice = project
            .narrator_voice
            .clone()
            .map(|v| v.into_core())
            .unwrap_or_default();
        let before_style_notes = project.reader_contract.style_notes.clone();

        let branch_id = project.active_branch_id.clone();
        let existing_world_rules = if let Some(ref bid) = branch_id {
            self.repository()
                .list_world_rules_by_project_and_branch(&input.project_id, bid)
                .await?
        } else {
            Vec::new()
        };

        // Use helper
        let (
            after_narrator_voice,
            added_style_notes,
            removed_style_notes,
            after_style_notes,
            style_rule_action,
            _cache_ids,
        ) = build_apply_style_profile_diff(
            &profile,
            &current_narrator_voice,
            &before_style_notes,
            input.mode,
            &existing_world_rules,
            branch_id.as_deref(),
        );

        // Perform narrator voice update
        self.repository()
            .set_narrator_voice(&input.project_id, after_narrator_voice.clone())
            .await?;

        // Perform reader contract update
        let mut reader_contract = project.reader_contract.into_core();
        reader_contract.style_notes = after_style_notes;
        self.repository()
            .update_project_reader_contract(&input.project_id, &reader_contract)
            .await?;

        // Perform world rule update
        let mut style_rule_id = None;
        let db_rule_action_str = match style_rule_action.clone() {
            spindle_core::style::StyleWorldRuleAction::Create {
                rule_name,
                description,
            } => {
                let rule_out = self
                    .create_world_rule(CreateWorldRuleInput {
                        project_id: input.project_id.clone(),
                        rule_name,
                        rule_type: "style".to_string(),
                        description,
                        scan_pattern: None,
                        relevance_tags: Vec::new(),
                        established_in: None,
                    })
                    .await?;
                style_rule_id = Some(rule_out.world_rule_id);
                "created".to_string()
            }
            spindle_core::style::StyleWorldRuleAction::Update {
                rule_id,
                new_description,
                ..
            } => {
                let changes = serde_json::json!({
                    "description": new_description
                });
                self.update_world_rule(UpdateWorldRuleInput {
                    world_rule_id: rule_id.clone(),
                    changes,
                })
                .await?;
                style_rule_id = Some(rule_id);
                "updated".to_string()
            }
            spindle_core::style::StyleWorldRuleAction::NoOp => {
                // If it exists but didn't change description, retrieve rule ID
                if let Some(_bid) = &branch_id {
                    let expected_name = format!("Style Profile: {}", profile.name);
                    style_rule_id = existing_world_rules
                        .iter()
                        .find(|r| r.rule_name == profile.name || r.rule_name == expected_name)
                        .map(|r| r.id.clone());
                }
                "no_op".to_string()
            }
        };

        // Invalidate caches
        let invalidated = self
            .resolve_phase_four_caches_for_project(
                &input.project_id,
                &[
                    PhaseFourCacheId::StyleCompliance,
                    PhaseFourCacheId::WorldRuleSemanticDrift,
                ],
            )
            .await?;

        // Write Audit Record
        let app_id = format!("style_profile_app:{}", ulid::Ulid::new());
        let applied_at = chrono::Utc::now().to_rfc3339();

        let prev_rule_desc = match &style_rule_action {
            spindle_core::style::StyleWorldRuleAction::Update {
                previous_description,
                ..
            } => Some(previous_description.clone()),
            _ => None,
        };

        let audit_record = spindle_core::style::StyleProfileApplicationRecord {
            id: app_id,
            project_id: input.project_id.clone(),
            profile_id: input.profile_id.clone(),
            applied_at,
            apply_mode: input.mode,
            before_narrator_voice: current_narrator_voice,
            after_narrator_voice,
            before_style_notes,
            after_style_notes: reader_contract.style_notes.clone(),
            added_style_notes,
            removed_style_notes,
            style_rule_id: style_rule_id.clone(),
            style_rule_action: db_rule_action_str,
            style_rule_previous_description: prev_rule_desc,
            invalidated_validator_count: invalidated,
            rolled_back_at: None,
            rollback_status: "not_rolled_back".to_string(),
        };

        self.repository()
            .insert_style_profile_application(&audit_record)
            .await?;

        Ok(ApplyStyleProfileOutput {
            project_id: input.project_id,
            profile_id: input.profile_id,
            narrator_voice: audit_record.after_narrator_voice,
            reader_contract_style_notes: audit_record.after_style_notes,
            style_rule_id,
            invalidated_validator_findings: invalidated,
        })
    }

    pub async fn preview_apply_style_profile(
        &self,
        input: spindle_core::style::PreviewApplyStyleProfileInput,
    ) -> Result<spindle_core::style::PreviewApplyStyleProfileOutput> {
        // 1. Fetch style profile
        let profile = self
            .repository()
            .get_style_profile(&input.project_id, &input.profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found"))?;
        if profile.status != StyleProfileStatus::Ready {
            return Err(anyhow!("style profile is not ready: {:?}", profile.status));
        }
        if !has_application_guidance(&profile.guidance) {
            return Err(anyhow!(
                "style profile has no application guidance to preview: {}",
                profile.profile_id
            ));
        }

        // Fetch project state
        let project = self.repository().get_project(&input.project_id).await?;
        let current_narrator_voice = project
            .narrator_voice
            .clone()
            .map(|v| v.into_core())
            .unwrap_or_default();
        let before_style_notes = project.reader_contract.style_notes.clone();

        let branch_id = project.active_branch_id.clone();
        let existing_world_rules = if let Some(ref bid) = branch_id {
            self.repository()
                .list_world_rules_by_project_and_branch(&input.project_id, bid)
                .await?
        } else {
            Vec::new()
        };

        // Use helper
        let (
            after_narrator_voice,
            added_style_notes,
            removed_style_notes,
            _after_style_notes,
            style_rule_action,
            validator_cache_ids,
        ) = build_apply_style_profile_diff(
            &profile,
            &current_narrator_voice,
            &before_style_notes,
            input.mode,
            &existing_world_rules,
            branch_id.as_deref(),
        );

        Ok(spindle_core::style::PreviewApplyStyleProfileOutput {
            project_id: input.project_id,
            profile_id: input.profile_id,
            before_narrator_voice: current_narrator_voice,
            after_narrator_voice,
            added_style_notes,
            removed_style_notes,
            style_rule_action,
            invalidated_validator_cache_ids: validator_cache_ids,
        })
    }

    pub async fn list_style_profile_applications(
        &self,
        input: spindle_core::style::ListStyleProfileApplicationsInput,
    ) -> Result<spindle_core::style::ListStyleProfileApplicationsOutput> {
        let applications = self
            .repository()
            .list_style_profile_applications(&input.project_id)
            .await?;
        Ok(spindle_core::style::ListStyleProfileApplicationsOutput { applications })
    }

    pub async fn rollback_style_profile_application(
        &self,
        input: spindle_core::style::RollbackStyleProfileApplicationInput,
    ) -> Result<spindle_core::style::RollbackStyleProfileApplicationOutput> {
        // 1. Fetch audit record
        let app = self
            .repository()
            .get_style_profile_application(&input.project_id, &input.application_id)
            .await?
            .ok_or_else(|| anyhow!("style profile application audit record not found"))?;

        if app.rollback_status == "rolled_back" {
            return Err(anyhow!("style profile application is already rolled back"));
        }

        // 2. Restore captured pre-apply narrator voice
        self.repository()
            .set_narrator_voice(&input.project_id, app.before_narrator_voice.clone())
            .await?;

        // 3. Restore captured pre-apply style notes
        let project = self.repository().get_project(&input.project_id).await?;
        let mut reader_contract = project.reader_contract.into_core();
        reader_contract.style_notes = app.before_style_notes.clone();
        self.repository()
            .update_project_reader_contract(&input.project_id, &reader_contract)
            .await?;

        // 4. Handle style world rule rollback conservatively
        let mut rule_action_done = "no_op".to_string();
        if let Some(rule_id) = &app.style_rule_id {
            match app.style_rule_action.as_str() {
                "created" => {
                    let deleted = self
                        .repository()
                        .delete_world_rule(&input.project_id, rule_id)
                        .await?;
                    if deleted {
                        rule_action_done = "deleted".to_string();
                    }
                }
                "updated" => {
                    if let Some(prev_desc) = &app.style_rule_previous_description {
                        let changes = serde_json::json!({
                            "description": prev_desc
                        });
                        self.update_world_rule(UpdateWorldRuleInput {
                            world_rule_id: rule_id.clone(),
                            changes,
                        })
                        .await?;
                        rule_action_done = "restored".to_string();
                    }
                }
                _ => {}
            }
        }

        // 5. Invalidate caches
        let invalidated = self
            .resolve_phase_four_caches_for_project(
                &input.project_id,
                &[
                    PhaseFourCacheId::StyleCompliance,
                    PhaseFourCacheId::WorldRuleSemanticDrift,
                ],
            )
            .await?;

        // 6. Record rollback status in the application audit row
        let rolled_back_at = chrono::Utc::now().to_rfc3339();
        self.repository()
            .update_style_profile_application_rollback(&app.id, &rolled_back_at, "rolled_back")
            .await?;

        Ok(spindle_core::style::RollbackStyleProfileApplicationOutput {
            project_id: input.project_id,
            application_id: app.id,
            rolled_back_at,
            narrator_voice: app.before_narrator_voice,
            reader_contract_style_notes: app.before_style_notes,
            style_rule_action: rule_action_done,
            invalidated_validator_findings: invalidated,
        })
    }

    pub async fn check_style_against_profile(
        &self,
        input: spindle_core::style::CheckStyleAgainstProfileInput,
    ) -> Result<spindle_core::style::CheckStyleAgainstProfileOutput> {
        // 1. Determine profile_id
        let profile_id = match input.profile_id {
            Some(pid) => pid,
            None => {
                self.repository()
                    .get_most_recently_applied_profile_id(&input.project_id)
                    .await?
                    .ok_or_else(|| anyhow!("No profile_id specified and no profile has been applied to this project yet"))?
            }
        };

        // 2. Fetch profile card
        let profile = self
            .repository()
            .get_style_profile(&input.project_id, &profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found: {}", profile_id))?;

        // 3. Fetch/retrieve prose text
        let draft_prose = match (&input.scene_id, &input.raw_text) {
            (Some(_), Some(_)) => {
                return Err(anyhow!("provide either scene_id or raw_text, not both"));
            }
            (Some(scene_id), None) => {
                let scene = self.repository().get_scene(scene_id).await?;
                if scene.project_id != input.project_id {
                    return Err(anyhow!("scene does not belong to project: {}", scene_id));
                }
                scene.full_text
            }
            (None, Some(text)) => text.clone(),
            (None, None) => return Err(anyhow!("Must provide either scene_id or raw_text")),
        };

        // 4. Compute metrics on the draft prose
        let normalized = style_helper::normalize_markdown(&draft_prose);
        let elements = style_helper::parse_elements(&normalized);
        let draft_metrics = style_helper::compute_metrics(&elements);

        let mut findings = Vec::new();

        // 5. Compare metrics deterministically
        // a. Sentence length mismatch
        if profile.metrics.average_sentence_words > 0.0
            && draft_metrics.average_sentence_words > 0.0
        {
            let ratio =
                draft_metrics.average_sentence_words / profile.metrics.average_sentence_words;
            if ratio > 1.3 || ratio < 0.7 {
                findings.push(spindle_core::style::StyleDriftFinding {
                    severity: "warning".to_string(),
                    category: "sentence_length".to_string(),
                    evidence_summary: format!(
                        "Draft average sentence length is {:.1} words, but the style profile average is {:.1} words.",
                        draft_metrics.average_sentence_words,
                        profile.metrics.average_sentence_words
                    ),
                    suggested_correction: Some(if ratio > 1.3 {
                        "Break up long sentences into shorter, punchier clauses."
                    } else {
                        "Combine short, choppy sentences to flow more naturally."
                    }.to_string()),
                });
            }
        }

        // b. Paragraph length mismatch
        if profile.metrics.average_paragraph_words > 0.0
            && draft_metrics.average_paragraph_words > 0.0
        {
            let ratio =
                draft_metrics.average_paragraph_words / profile.metrics.average_paragraph_words;
            if ratio > 1.5 || ratio < 0.6 {
                findings.push(spindle_core::style::StyleDriftFinding {
                    severity: "warning".to_string(),
                    category: "paragraph_length".to_string(),
                    evidence_summary: format!(
                        "Draft average paragraph length is {:.1} words, but the style profile average is {:.1} words.",
                        draft_metrics.average_paragraph_words,
                        profile.metrics.average_paragraph_words
                    ),
                    suggested_correction: Some(if ratio > 1.5 {
                        "Insert paragraph breaks to make blocks of text shorter."
                    } else {
                        "Merge short paragraphs into longer cohesive thematic blocks."
                    }.to_string()),
                });
            }
        }

        // c. Dialogue ratio mismatch
        if profile.metrics.dialogue_word_ratio > 0.0 && draft_metrics.dialogue_word_ratio > 0.0 {
            let diff =
                (draft_metrics.dialogue_word_ratio - profile.metrics.dialogue_word_ratio).abs();
            if diff > 0.2 {
                findings.push(spindle_core::style::StyleDriftFinding {
                    severity: "warning".to_string(),
                    category: "dialogue_ratio".to_string(),
                    evidence_summary: format!(
                        "Draft dialogue word ratio is {:.1}%, but the style profile average is {:.1}%.",
                        draft_metrics.dialogue_word_ratio * 100.0,
                        profile.metrics.dialogue_word_ratio * 100.0
                    ),
                    suggested_correction: Some(if draft_metrics.dialogue_word_ratio < profile.metrics.dialogue_word_ratio {
                        "Add more character dialogue to break up narrative blocks."
                    } else {
                        "Reduce dialogue frequency in favor of interiority or action narration."
                    }.to_string()),
                });
            }
        }

        // 6. Run the existing scanner
        let style_rule = spindle_core::style::StyleRule {
            rule_name: format!("Style Profile: {}", profile.name),
            description: profile.guidance.prompt_snippet.clone(),
        };
        let directive = spindle_core::style::StyleDirective::assemble(
            profile.guidance.pov.clone().unwrap_or_default(),
            "derived style profile".to_string(),
            profile.guidance.summary.clone(),
            profile.guidance.do_rules.clone(),
            profile.guidance.avoid_rules.clone(),
            vec![style_rule],
            Some(profile.guidance.narrator_voice.clone()),
        );

        let scan_input = spindle_core::style::StyleScanInput {
            prose: &draft_prose,
            declared_tone: None,
            is_chapter_end: false,
        };
        let scanner_hits = directive.scan(&scan_input);
        for hit in scanner_hits {
            findings.push(spindle_core::style::StyleDriftFinding {
                severity: match hit.severity {
                    spindle_core::style::StyleDriftSeverity::Warning => "warning".to_string(),
                    spindle_core::style::StyleDriftSeverity::Info => "info".to_string(),
                },
                category: "scanner_heuristic".to_string(),
                evidence_summary: hit.message.clone(),
                suggested_correction: None,
            });
        }

        Ok(spindle_core::style::CheckStyleAgainstProfileOutput {
            project_id: input.project_id,
            profile_id,
            findings,
        })
    }
}

// ── Shared Apply Helpers ───────────────────────────────────────────

fn is_generated_style_note(note: &str) -> bool {
    note.contains(" (Style Profile: style_profile:")
}

fn make_generated_style_note(text: &str, profile_id: &str, profile_name: &str) -> String {
    format!(
        "{} (Style Profile: {}/{})",
        text.trim(),
        profile_id,
        profile_name
    )
}

fn build_apply_style_profile_diff(
    profile: &StyleProfileCard,
    _current_narrator_voice: &spindle_core::style::NarratorVoice,
    before_style_notes: &[String],
    mode: StyleProfileApplyMode,
    existing_world_rules: &[crate::sqlite::records::WorldRule],
    active_branch_id: Option<&str>,
) -> (
    spindle_core::style::NarratorVoice, // after_narrator_voice
    Vec<String>,                        // added_style_notes
    Vec<String>,                        // removed_style_notes
    Vec<String>,                        // after_style_notes
    spindle_core::style::StyleWorldRuleAction,
    Vec<String>, // validator_cache_ids
) {
    let after_narrator_voice = profile.guidance.narrator_voice.clone();

    let mut notes_to_add = Vec::new();
    if !profile.guidance.summary.trim().is_empty() {
        notes_to_add.push(make_generated_style_note(
            &profile.guidance.summary,
            &profile.profile_id,
            &profile.name,
        ));
    }
    for rule in &profile.guidance.do_rules {
        if !rule.trim().is_empty() {
            notes_to_add.push(make_generated_style_note(
                &format!("Do: {}", rule),
                &profile.profile_id,
                &profile.name,
            ));
        }
    }
    for rule in &profile.guidance.avoid_rules {
        if !rule.trim().is_empty() {
            notes_to_add.push(make_generated_style_note(
                &format!("Avoid: {}", rule),
                &profile.profile_id,
                &profile.name,
            ));
        }
    }

    let (added_style_notes, removed_style_notes, after_style_notes) = match mode {
        StyleProfileApplyMode::Merge => {
            let mut added = Vec::new();
            let mut after = before_style_notes.to_vec();
            let mut existing_set: std::collections::HashSet<String> = before_style_notes
                .iter()
                .map(|n| n.trim().to_lowercase())
                .collect();
            for note in notes_to_add {
                let note_key = note.trim().to_lowercase();
                if existing_set.insert(note_key) {
                    added.push(note.clone());
                    after.push(note);
                }
            }
            (added, Vec::new(), after)
        }
        StyleProfileApplyMode::ReplaceGeneratedStyleNotes => {
            let mut removed = Vec::new();
            let mut kept_user_notes = Vec::new();
            for note in before_style_notes {
                if is_generated_style_note(note) {
                    removed.push(note.clone());
                } else {
                    kept_user_notes.push(note.clone());
                }
            }

            let mut added = Vec::new();
            let mut after = kept_user_notes.clone();
            let mut user_set: std::collections::HashSet<String> = kept_user_notes
                .iter()
                .map(|n| n.trim().to_lowercase())
                .collect();
            for note in notes_to_add {
                let note_key = note.trim().to_lowercase();
                if user_set.insert(note_key) {
                    added.push(note.clone());
                    after.push(note);
                }
            }
            (added, removed, after)
        }
    };

    let mut style_rule_action = spindle_core::style::StyleWorldRuleAction::NoOp;
    if active_branch_id.is_some() && !profile.guidance.prompt_snippet.trim().is_empty() {
        let expected_name = format!("Style Profile: {}", profile.name);
        let existing = existing_world_rules
            .iter()
            .find(|r| r.rule_name == profile.name || r.rule_name == expected_name);

        if let Some(r) = existing {
            if r.description.trim() != profile.guidance.prompt_snippet.trim() {
                style_rule_action = spindle_core::style::StyleWorldRuleAction::Update {
                    rule_id: r.id.clone(),
                    rule_name: r.rule_name.clone(),
                    previous_description: r.description.clone(),
                    new_description: profile.guidance.prompt_snippet.clone(),
                };
            }
        } else {
            style_rule_action = spindle_core::style::StyleWorldRuleAction::Create {
                rule_name: expected_name,
                description: profile.guidance.prompt_snippet.clone(),
            };
        }
    }

    let validator_cache_ids = vec![
        "style_compliance".to_string(),
        "world_rule_semantic_drift".to_string(),
    ];

    (
        after_narrator_voice,
        added_style_notes,
        removed_style_notes,
        after_style_notes,
        style_rule_action,
        validator_cache_ids,
    )
}

fn has_application_guidance(guidance: &StyleProfileGuidance) -> bool {
    !guidance.narrator_voice.is_empty()
        || !guidance.summary.trim().is_empty()
        || !guidance.prompt_snippet.trim().is_empty()
        || guidance.do_rules.iter().any(|rule| !rule.trim().is_empty())
        || guidance
            .avoid_rules
            .iter()
            .any(|rule| !rule.trim().is_empty())
}
