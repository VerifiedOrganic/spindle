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

        // 5b. Compute quality report
        let mut quality_warnings = corpus_summary.warnings.clone();

        let fp = metrics.first_person_pronoun_rate_per_1k_words;
        let tp = metrics.third_person_pronoun_rate_per_1k_words;
        let pov_tense_confidence = if fp + tp > 0.0 {
            ((fp - tp).abs() / (fp + tp)).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let pov_tense_confidence = (0.5 + 0.5 * pov_tense_confidence).clamp(0.0, 1.0);

        let mut chunk_dialogue_ratios = Vec::new();
        let mut chunk_sentence_averages = Vec::new();
        for chunk in &chunks {
            let normalized = style_helper::normalize_markdown(&chunk.text);
            let elements = style_helper::parse_elements(&normalized);
            let cm = style_helper::compute_metrics(&elements);
            chunk_dialogue_ratios.push(cm.dialogue_word_ratio);
            chunk_sentence_averages.push(cm.average_sentence_words);
        }

        let chunk_consistency = if chunk_dialogue_ratios.len() > 1 {
            let avg_dialogue =
                chunk_dialogue_ratios.iter().sum::<f64>() / chunk_dialogue_ratios.len() as f64;
            let dev_dialogue = chunk_dialogue_ratios
                .iter()
                .map(|&x| (x - avg_dialogue).abs())
                .sum::<f64>()
                / chunk_dialogue_ratios.len() as f64;
            let dialogue_consistency = (1.0 - dev_dialogue * 2.0).clamp(0.0, 1.0);

            let avg_sentence =
                chunk_sentence_averages.iter().sum::<f64>() / chunk_sentence_averages.len() as f64;
            let dev_sentence = if avg_sentence > 0.0 {
                chunk_sentence_averages
                    .iter()
                    .map(|&x| (x - avg_sentence).abs())
                    .sum::<f64>()
                    / chunk_sentence_averages.len() as f64
                    / avg_sentence
            } else {
                0.0
            };
            let sentence_consistency = (1.0 - dev_sentence).clamp(0.0, 1.0);

            (dialogue_consistency * 0.5 + sentence_consistency * 0.5).clamp(0.0, 1.0)
        } else {
            1.0
        };

        let size_score = (total_words as f64 / 3000.0).clamp(0.0, 1.0);
        let confidence_score =
            (size_score * 0.4 + pov_tense_confidence * 0.3 + chunk_consistency * 0.3)
                .clamp(0.0, 1.0);

        let classification = if total_words < 3000 {
            spindle_core::style::StyleProfileQualityClassification::Thin
        } else if chunk_consistency < 0.6 {
            spindle_core::style::StyleProfileQualityClassification::Inconsistent
        } else {
            spindle_core::style::StyleProfileQualityClassification::Ready
        };

        if classification == spindle_core::style::StyleProfileQualityClassification::Thin {
            quality_warnings.push("The corpus is thin (under 3,000 words). Style metrics and synthesis may be less reliable.".to_string());
        }
        if classification == spindle_core::style::StyleProfileQualityClassification::Inconsistent {
            quality_warnings.push("The corpus is style-inconsistent across chunks. Derived rules might not represent the whole text uniformly.".to_string());
        }
        if pov_tense_confidence < 0.6 {
            quality_warnings.push("Low confidence in POV/tense consistency.".to_string());
        }
        quality_warnings.dedup();

        let quality = spindle_core::style::StyleProfileQualityReport {
            corpus_size_words: total_words,
            dialogue_coverage: metrics.dialogue_word_ratio,
            pov_tense_confidence,
            chunk_consistency,
            file_count: md_files.len(),
            warnings: quality_warnings,
            confidence_score,
            classification,
        };

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
            metrics_only,
            source_sample_word_budget: input.source_sample_word_budget,
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
            quality,
            archived_at: None,
        };

        let should_apply = input.apply.unwrap_or(false);
        if should_apply
            && profile.quality.classification
                != spindle_core::style::StyleProfileQualityClassification::Ready
            && !input.force_apply.unwrap_or(false)
        {
            return Err(anyhow!(
                "Style profile quality is too low to auto-apply (classification: {:?}). Use force_apply=true to override.",
                profile.quality.classification
            ));
        }

        // 7. Persist profile
        self.repository().insert_style_profile(&profile).await?;

        // 8. Optionally apply profile
        let mut applied = false;
        let mut application = None;
        if should_apply {
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
        ensure_style_profile_not_archived(&profile)?;
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

        self.repository()
            .set_active_style_profile_id(&input.project_id, Some(input.profile_id.clone()))
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
        ensure_style_profile_not_archived(&profile)?;
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

        // 7. Clear or restore active style profile ID
        if project.active_style_profile_id.as_ref() == Some(&app.profile_id) {
            let apps = self
                .repository()
                .list_style_profile_applications(&input.project_id)
                .await?;
            let mut previous_active = None;
            for a in apps {
                if a.id != app.id && a.rollback_status != "rolled_back" {
                    if let Ok(Some(prof)) = self
                        .repository()
                        .get_style_profile(&input.project_id, &a.profile_id)
                        .await
                    {
                        if prof.archived_at.is_none() {
                            previous_active = Some(a.profile_id);
                            break;
                        }
                    }
                }
            }
            self.repository()
                .set_active_style_profile_id(&input.project_id, previous_active)
                .await?;
        }

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

    fn check_style_against_prose(
        &self,
        profile: &spindle_core::style::StyleProfileCard,
        draft_prose: &str,
        scene_id: Option<String>,
    ) -> Vec<spindle_core::style::StyleDriftFinding> {
        let normalized = style_helper::normalize_markdown(draft_prose);
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
                let delta =
                    draft_metrics.average_sentence_words - profile.metrics.average_sentence_words;
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
                    scene_id: scene_id.clone(),
                    metric_name: Some("average_sentence_words".to_string()),
                    metric_delta: Some(delta),
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
                let delta =
                    draft_metrics.average_paragraph_words - profile.metrics.average_paragraph_words;
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
                    scene_id: scene_id.clone(),
                    metric_name: Some("average_paragraph_words".to_string()),
                    metric_delta: Some(delta),
                });
            }
        }

        // c. Dialogue ratio mismatch
        if profile.metrics.dialogue_word_ratio > 0.0 && draft_metrics.dialogue_word_ratio > 0.0 {
            let diff =
                (draft_metrics.dialogue_word_ratio - profile.metrics.dialogue_word_ratio).abs();
            if diff > 0.2 {
                let delta = draft_metrics.dialogue_word_ratio - profile.metrics.dialogue_word_ratio;
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
                    scene_id: scene_id.clone(),
                    metric_name: Some("dialogue_word_ratio".to_string()),
                    metric_delta: Some(delta),
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
            prose: draft_prose,
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
                scene_id: scene_id.clone(),
                metric_name: None,
                metric_delta: None,
            });
        }

        findings
    }

    pub async fn check_style_against_profile(
        &self,
        input: spindle_core::style::CheckStyleAgainstProfileInput,
    ) -> Result<spindle_core::style::CheckStyleAgainstProfileOutput> {
        // 1. Determine profile_id
        let profile_id = match input.profile_id {
            Some(pid) => pid,
            None => {
                let project = self.repository().get_project(&input.project_id).await?;
                project.active_style_profile_id
                    .ok_or_else(|| anyhow!("No profile_id specified and no active style profile is set for this project"))?
            }
        };

        // 2. Fetch profile card
        let profile = self
            .repository()
            .get_style_profile(&input.project_id, &profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found: {}", profile_id))?;
        ensure_style_profile_not_archived(&profile)?;

        let mut findings = Vec::new();

        // 3. Determine targets: chapter, scene, or raw text
        if let Some(chapter_id) = &input.chapter_id {
            if input.scene_id.is_some() || input.raw_text.is_some() {
                return Err(anyhow!(
                    "provide only one of chapter_id, scene_id, or raw_text"
                ));
            }
            let chapter = self.repository().get_chapter(chapter_id).await?;
            if chapter.project_id != input.project_id {
                return Err(anyhow!(
                    "chapter does not belong to project: {}",
                    chapter_id
                ));
            }
            let scenes = self.repository().list_scenes_by_chapter(chapter_id).await?;
            for scene in scenes {
                let scene_findings = self.check_style_against_prose(
                    &profile,
                    &scene.full_text,
                    Some(scene.id.clone()),
                );
                findings.extend(scene_findings);
            }
        } else {
            let (prose, scene_id) = match (&input.scene_id, &input.raw_text) {
                (Some(_), Some(_)) => {
                    return Err(anyhow!(
                        "provide only one of chapter_id, scene_id, or raw_text"
                    ));
                }
                (Some(sid), None) => {
                    let scene = self.repository().get_scene(sid).await?;
                    if scene.project_id != input.project_id {
                        return Err(anyhow!("scene does not belong to project: {}", sid));
                    }
                    (scene.full_text, Some(sid.clone()))
                }
                (None, Some(text)) => (text.clone(), None),
                (None, None) => {
                    return Err(anyhow!(
                        "Must provide either chapter_id, scene_id or raw_text"
                    ));
                }
            };
            let scene_findings = self.check_style_against_prose(&profile, &prose, scene_id);
            findings.extend(scene_findings);
        }

        // 4. Compute summary score
        let warning_count = findings.iter().filter(|f| f.severity == "warning").count();
        let error_count = findings.iter().filter(|f| f.severity == "error").count();

        let summary_score = if error_count > 0 || warning_count >= 3 {
            spindle_core::style::StyleDriftSummaryScore::StrongDrift
        } else if warning_count > 0 {
            spindle_core::style::StyleDriftSummaryScore::MildDrift
        } else {
            spindle_core::style::StyleDriftSummaryScore::Aligned
        };

        Ok(spindle_core::style::CheckStyleAgainstProfileOutput {
            project_id: input.project_id,
            profile_id,
            findings,
            summary_score,
        })
    }

    pub async fn plan_style_revision(
        &self,
        input: spindle_core::style::PlanStyleRevisionInput,
    ) -> Result<spindle_core::style::PlanStyleRevisionOutput> {
        // 1. Build CheckStyleAgainstProfileInput to reuse existing logic
        let check_input = spindle_core::style::CheckStyleAgainstProfileInput {
            project_id: input.project_id.clone(),
            profile_id: input.profile_id.clone(),
            scene_id: input.scene_id.clone(),
            raw_text: input.raw_text.clone(),
            chapter_id: input.chapter_id.clone(),
        };

        // This resolves active profile, validates scene/chapter ownership, rejects archived profile, and rejects ambiguous targets
        let check_output = self.check_style_against_profile(check_input).await?;

        // 2. Fetch full text and build target summary
        let mut target_prose = String::new();
        let target_summary = if let Some(cid) = &input.chapter_id {
            let chapter = self.repository().get_chapter(cid).await?;
            let scenes = self.repository().list_scenes_by_chapter(cid).await?;
            let mut total_words = 0;
            for s in &scenes {
                target_prose.push_str(&s.full_text);
                target_prose.push('\n');
                total_words += s.full_text.split_whitespace().count();
            }
            let title = chapter.title.as_deref().unwrap_or("Untitled");
            format!(
                "Chapter: {} ({}, {} scenes, total word count: {})",
                chapter.id,
                title,
                scenes.len(),
                total_words
            )
        } else if let Some(sid) = &input.scene_id {
            let scene = self.repository().get_scene(sid).await?;
            target_prose = scene.full_text.clone();
            let word_count = scene.full_text.split_whitespace().count();
            format!("Scene: {} (word count: {})", scene.id, word_count)
        } else if let Some(raw) = &input.raw_text {
            target_prose = raw.clone();
            let word_count = raw.split_whitespace().count();
            format!("Raw text target (word count: {})", word_count)
        } else {
            return Err(anyhow!(
                "Must provide either chapter_id, scene_id or raw_text"
            ));
        };

        // 3. Filter findings based on metrics_only
        let mut raw_findings = check_output.findings;
        if input.metrics_only.unwrap_or(false) {
            raw_findings.retain(|f| {
                f.category == "sentence_length"
                    || f.category == "paragraph_length"
                    || f.category == "dialogue_ratio"
            });
        }

        // 4. Recalculate drift summary score for the filtered findings
        let warning_count = raw_findings
            .iter()
            .filter(|f| f.severity == "warning")
            .count();
        let error_count = raw_findings
            .iter()
            .filter(|f| f.severity == "error")
            .count();

        let drift_summary_score = if error_count > 0 || warning_count >= 3 {
            spindle_core::style::StyleDriftSummaryScore::StrongDrift
        } else if warning_count > 0 {
            spindle_core::style::StyleDriftSummaryScore::MildDrift
        } else {
            spindle_core::style::StyleDriftSummaryScore::Aligned
        };

        // 5. Convert findings to DTO format
        let mut findings = Vec::new();
        for f in raw_findings {
            let severity = match f.severity.as_str() {
                "warning" => spindle_core::style::StyleRevisionSeverity::Warning,
                _ => spindle_core::style::StyleRevisionSeverity::Info,
            };
            findings.push(spindle_core::style::StyleRevisionPlanFinding {
                severity,
                category: f.category,
                evidence_summary: f.evidence_summary,
                suggested_correction: f.suggested_correction,
                scene_id: f.scene_id,
                metric_name: f.metric_name,
                metric_delta: f.metric_delta,
            });
        }

        // Sort findings: Warnings first, then Info
        findings.sort_by(|a, b| {
            let a_val = match a.severity {
                spindle_core::style::StyleRevisionSeverity::Warning => 0,
                spindle_core::style::StyleRevisionSeverity::Info => 1,
            };
            let b_val = match b.severity {
                spindle_core::style::StyleRevisionSeverity::Warning => 0,
                spindle_core::style::StyleRevisionSeverity::Info => 1,
            };
            a_val.cmp(&b_val)
        });

        // 6. Generate revision steps from sorted findings
        let mut steps = Vec::new();
        for (i, finding) in findings.iter().enumerate() {
            let instructions = finding
                .suggested_correction
                .clone()
                .unwrap_or_else(|| finding.evidence_summary.clone());

            let target_scope = if finding.scene_id.is_some() {
                spindle_core::style::StyleRevisionTargetScope::Scene
            } else if input.chapter_id.is_some() {
                spindle_core::style::StyleRevisionTargetScope::Chapter
            } else if input.scene_id.is_some() {
                spindle_core::style::StyleRevisionTargetScope::Scene
            } else {
                spindle_core::style::StyleRevisionTargetScope::RawText
            };

            let confidence = match finding.category.as_str() {
                "sentence_length" | "paragraph_length" | "dialogue_ratio" => {
                    spindle_core::style::StyleRevisionConfidence::High
                }
                _ => spindle_core::style::StyleRevisionConfidence::Medium,
            };

            steps.push(spindle_core::style::StyleRevisionPlanStep {
                order: i + 1,
                finding_category: finding.category.clone(),
                instructions,
                target_scope,
                target_id: finding.scene_id.clone(),
                confidence,
            });
        }

        // Apply max suggestions limit if present
        if let Some(max_sugg) = input.max_suggestions {
            if findings.len() > max_sugg {
                findings.truncate(max_sugg);
            }
            if steps.len() > max_sugg {
                steps.truncate(max_sugg);
            }
        }

        // 7. Generate rewrite examples if requested (and LLM route is allowed)
        let mut rewrite_examples = None;
        if input.include_rewrite_examples.unwrap_or(false) && !input.metrics_only.unwrap_or(false) {
            let profile = self
                .repository()
                .get_style_profile(&input.project_id, &check_output.profile_id)
                .await?
                .ok_or_else(|| anyhow!("style profile not found: {}", check_output.profile_id))?;

            let trimmed_prose = trim_to_word_limit(&target_prose, 4000);
            let max_sugg = input.max_suggestions.unwrap_or(3);
            let prompt = format!(
                "You are an expert editor. You have been given a style profile and a piece of prose that drifts from this style.\n\n\
                 Style Profile Name: {}\n\
                 - POV: {}\n\
                 - Tense: {}\n\
                 - Narrative Voice: {:?}\n\
                 - Do Rules: {:?}\n\
                 - Avoid Rules: {:?}\n\n\
                 Prose to Revise:\n\
                 {}\n\n\
                 Please provide at most {} short rewrite examples showing how to align the prose with the style profile.\n\
                 Each example must show an original snippet (max 2-3 sentences), the revised style-aligned version, and an explanation of the changes.\n\
                 Output MUST be a valid JSON array matching this schema:\n\
                 [\n\
                   {{\n\
                     \"original_prose\": \"string\",\n\
                     \"revised_prose\": \"string\",\n\
                     \"explanation\": \"string\"\n\
                   }}\n\
                 ]\n\
                 Do not wrap in markdown code blocks.",
                profile.name,
                profile.guidance.pov.clone().unwrap_or_default(),
                profile.guidance.tense.clone().unwrap_or_default(),
                profile.guidance.narrator_voice,
                profile.guidance.do_rules,
                profile.guidance.avoid_rules,
                trimmed_prose,
                max_sugg
            );

            let req = crate::ai::ModelRequest {
                route: "style_revise".to_string(),
                prompt,
                rating: None,
                context: Some(crate::ai::RequestContext {
                    project_id: Some(input.project_id.clone()),
                    book_id: None,
                    chapter_id: input.chapter_id.clone(),
                    scene_id: input.scene_id.clone(),
                }),
            };

            if let Ok(response) = self.repository().model_router().complete(&req).await {
                let cleaned = clean_json_response(&response.output);
                if let Ok(parsed) = serde_json::from_str::<
                    Vec<spindle_core::style::StyleRevisionPlanExample>,
                >(cleaned)
                {
                    rewrite_examples = Some(parsed);
                }
            }
        }

        Ok(spindle_core::style::PlanStyleRevisionOutput {
            project_id: input.project_id,
            profile_id: check_output.profile_id,
            target_summary,
            drift_summary_score,
            findings,
            steps,
            rewrite_examples,
            mutates_prose: false,
        })
    }

    pub async fn compare_style_profiles(
        &self,
        input: spindle_core::style::CompareStyleProfilesInput,
    ) -> Result<spindle_core::style::CompareStyleProfilesOutput> {
        let profile_a = self
            .repository()
            .get_style_profile(&input.project_id, &input.profile_id_a)
            .await?
            .ok_or_else(|| anyhow!("style profile A not found: {}", input.profile_id_a))?;
        ensure_style_profile_not_archived(&profile_a)?;
        let profile_b = self
            .repository()
            .get_style_profile(&input.project_id, &input.profile_id_b)
            .await?
            .ok_or_else(|| anyhow!("style profile B not found: {}", input.profile_id_b))?;
        ensure_style_profile_not_archived(&profile_b)?;

        // 1. Calculate metric deltas
        let ma = &profile_a.metrics;
        let mb = &profile_b.metrics;
        let metric_deltas = spindle_core::style::StyleCorpusMetricsDeltas {
            average_sentence_words_delta: mb.average_sentence_words - ma.average_sentence_words,
            median_sentence_words_delta: mb.median_sentence_words - ma.median_sentence_words,
            p90_sentence_words_delta: mb.p90_sentence_words - ma.p90_sentence_words,
            average_paragraph_words_delta: mb.average_paragraph_words - ma.average_paragraph_words,
            median_paragraph_words_delta: mb.median_paragraph_words - ma.median_paragraph_words,
            dialogue_line_ratio_delta: mb.dialogue_line_ratio - ma.dialogue_line_ratio,
            dialogue_word_ratio_delta: mb.dialogue_word_ratio - ma.dialogue_word_ratio,
            question_mark_rate_delta: mb.question_mark_rate_per_1k_words
                - ma.question_mark_rate_per_1k_words,
            exclamation_rate_delta: mb.exclamation_rate_per_1k_words
                - ma.exclamation_rate_per_1k_words,
            semicolon_rate_delta: mb.semicolon_rate_per_1k_words - ma.semicolon_rate_per_1k_words,
            em_dash_rate_delta: mb.em_dash_rate_per_1k_words - ma.em_dash_rate_per_1k_words,
            ellipsis_rate_delta: mb.ellipsis_rate_per_1k_words - ma.ellipsis_rate_per_1k_words,
            first_person_pronoun_rate_delta: mb.first_person_pronoun_rate_per_1k_words
                - ma.first_person_pronoun_rate_per_1k_words,
            third_person_pronoun_rate_delta: mb.third_person_pronoun_rate_per_1k_words
                - ma.third_person_pronoun_rate_per_1k_words,
        };

        // 2. Guidance differences
        let ga = &profile_a.guidance;
        let gb = &profile_b.guidance;

        let summary_changed = ga.summary != gb.summary;
        let pov_changed = ga.pov != gb.pov;
        let tense_changed = ga.tense != gb.tense;
        let narrator_distance_changed = ga.narrator_distance != gb.narrator_distance;
        let voice_changed = ga.narrator_voice != gb.narrator_voice;

        let mut do_rules_added = Vec::new();
        let mut do_rules_removed = Vec::new();
        for r in &gb.do_rules {
            if !ga.do_rules.contains(r) {
                do_rules_added.push(r.clone());
            }
        }
        for r in &ga.do_rules {
            if !gb.do_rules.contains(r) {
                do_rules_removed.push(r.clone());
            }
        }

        let mut avoid_rules_added = Vec::new();
        let mut avoid_rules_removed = Vec::new();
        for r in &gb.avoid_rules {
            if !ga.avoid_rules.contains(r) {
                avoid_rules_added.push(r.clone());
            }
        }
        for r in &ga.avoid_rules {
            if !gb.avoid_rules.contains(r) {
                avoid_rules_removed.push(r.clone());
            }
        }

        let guidance_differences = spindle_core::style::StyleProfileGuidanceDifferences {
            summary_changed,
            pov_changed,
            tense_changed,
            narrator_distance_changed,
            voice_changed,
            do_rules_added,
            do_rules_removed,
            avoid_rules_added,
            avoid_rules_removed,
        };

        // 3. Determine if material change is likely
        let mut change_reasons = Vec::new();
        if pov_changed {
            change_reasons.push(format!("POV changed from {:?} to {:?}", ga.pov, gb.pov));
        }
        if tense_changed {
            change_reasons.push(format!(
                "Tense changed from {:?} to {:?}",
                ga.tense, gb.tense
            ));
        }
        if metric_deltas.average_sentence_words_delta.abs() > 3.0 {
            change_reasons.push(format!(
                "Significant change in average sentence length: delta of {:.1} words",
                metric_deltas.average_sentence_words_delta
            ));
        }
        if metric_deltas.average_paragraph_words_delta.abs() > 10.0 {
            change_reasons.push(format!(
                "Significant change in average paragraph length: delta of {:.1} words",
                metric_deltas.average_paragraph_words_delta
            ));
        }
        if metric_deltas.dialogue_word_ratio_delta.abs() > 0.15 {
            change_reasons.push(format!(
                "Significant change in dialogue ratio: delta of {:.1}%",
                metric_deltas.dialogue_word_ratio_delta * 100.0
            ));
        }

        let likely_material_change = !change_reasons.is_empty();

        Ok(spindle_core::style::CompareStyleProfilesOutput {
            project_id: input.project_id,
            profile_id_a: input.profile_id_a,
            profile_id_b: input.profile_id_b,
            metric_deltas,
            guidance_differences,
            likely_material_change,
            change_reasons,
        })
    }

    pub async fn archive_style_profile(
        &self,
        input: spindle_core::style::ArchiveStyleProfileInput,
    ) -> Result<spindle_core::style::ArchiveStyleProfileOutput> {
        let project = self.repository().get_project(&input.project_id).await?;
        if let Some(active_id) = &project.active_style_profile_id {
            if active_id == &input.profile_id && !input.force.unwrap_or(false) {
                return Err(anyhow!(
                    "Cannot archive the active style profile unless force=true is provided"
                ));
            }
        }

        let archived_at = self
            .repository()
            .archive_style_profile(&input.project_id, &input.profile_id)
            .await?;

        if let Some(active_id) = &project.active_style_profile_id {
            if active_id == &input.profile_id {
                self.repository()
                    .set_active_style_profile_id(&input.project_id, None)
                    .await?;
            }
        }

        Ok(spindle_core::style::ArchiveStyleProfileOutput {
            project_id: input.project_id,
            profile_id: input.profile_id,
            archived_at,
        })
    }
}

// ── Shared Apply Helpers ───────────────────────────────────────────

fn is_generated_style_note(note: &str) -> bool {
    note.contains(" (Style Profile: style_profile:")
}

fn ensure_style_profile_not_archived(profile: &StyleProfileCard) -> Result<()> {
    if profile.archived_at.is_some() {
        return Err(anyhow!(
            "style profile is archived and cannot be used: {}",
            profile.profile_id
        ));
    }
    Ok(())
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
