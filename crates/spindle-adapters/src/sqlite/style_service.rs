use crate::sqlite::SqliteSpindleService;
use crate::sqlite::service::PhaseFourCacheId;
use crate::sqlite::style_helper;
use anyhow::{Result, anyhow};
use sha2::Digest;
use spindle_core::models::{CreateWorldRuleInput, UpdateWorldRuleInput};
use spindle_core::style::{
    ApplyStyleProfileInput, ApplyStyleProfileOutput, CheckStyleProfileSourcesInput,
    CheckStyleProfileSourcesOutput, CreateStyleProfileFromMarkdownInput,
    CreateStyleProfileFromMarkdownOutput, GetStyleProfileInput, GetStyleProfileOutput,
    ListStyleProfilesInput, ListStyleProfilesOutput, PreviewRefreshStyleProfileInput,
    PreviewRefreshStyleProfileOutput, RefreshStyleProfileInput, RefreshStyleProfileOutput,
    StyleCorpusSummary, StyleProfileApplyMode, StyleProfileCard, StyleProfileGuidance,
    StyleProfileModelReceipt, StyleProfileSourcePolicy, StyleProfileStatus, StyleSourceRef,
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
    pub async fn generate_candidate_style_profile(
        &self,
        project_id: &str,
        name: &str,
        policy: &StyleProfileSourcePolicy,
        override_metrics_only: Option<bool>,
    ) -> Result<StyleProfileCard> {
        // 1. Validate project exists
        let _project = self.repository().get_project(project_id).await?;

        // 2. Resolve allowed roots
        let data_dir = self.repository().data_dir().to_path_buf();
        let mut allowed_roots = vec![data_dir.clone()];
        if let Some(parent) = data_dir.parent() {
            allowed_roots.push(parent.to_path_buf());
        }

        // 3. Resolve source paths
        let source_paths = if policy.source_paths.is_empty() {
            return Err(anyhow!("no source paths defined in policy"));
        } else {
            policy.source_paths.clone()
        };

        // Collect Markdown files with safety boundary check
        let max_files = policy.max_files.unwrap_or(100);
        let max_bytes = policy.max_bytes_per_file.unwrap_or(10 * 1024 * 1024); // 10MB
        let max_words = policy.max_total_words.unwrap_or(200_000);

        let recursive = policy.recursive.unwrap_or(false);
        let include_globs = policy.include_globs.as_deref();
        let exclude_globs = policy.exclude_globs.as_deref();

        let mut missing_source_roots = Vec::new();
        let mut valid_source_paths = Vec::new();
        for path_str in &source_paths {
            let path = std::path::Path::new(path_str);
            if !path.exists() {
                missing_source_roots.push(format!("Source path '{}' does not exist", path_str));
                continue;
            }
            match style_helper::resolve_and_verify_path(path_str, &allowed_roots) {
                Ok(_) => {
                    valid_source_paths.push(path_str.clone());
                }
                Err(e) => {
                    if e.to_string().contains("outside allowed roots") {
                        return Err(e);
                    } else {
                        missing_source_roots.push(format!(
                            "Source path '{}' could not be verified: {}",
                            path_str, e
                        ));
                    }
                }
            }
        }

        let md_files = if valid_source_paths.is_empty() {
            return Err(anyhow!("no analyzable Markdown files found"));
        } else {
            style_helper::collect_markdown_files(
                &valid_source_paths,
                recursive,
                &allowed_roots,
                max_files,
                include_globs,
                exclude_globs,
            )?
        };

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
                        file_size: None,
                        modified_at: None,
                        glob_policy_metadata: None,
                        captured_at: Some(chrono::Utc::now().to_rfc3339()),
                    });
                    continue;
                }
            };

            let file_size = metadata.len();
            let modified_at = metadata.modified().ok().map(|t| {
                let datetime: chrono::DateTime<chrono::Utc> = t.into();
                datetime.to_rfc3339()
            });

            if !metadata.is_file() {
                skipped_source_count += 1;
                source_refs.push(StyleSourceRef {
                    display_name,
                    canonical_path: canonical_path_str,
                    sha256: "".to_string(),
                    word_count: 0,
                    included: false,
                    skip_reason: Some("not a regular file".to_string()),
                    file_size: Some(file_size),
                    modified_at,
                    glob_policy_metadata: None,
                    captured_at: Some(chrono::Utc::now().to_rfc3339()),
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
                    file_size: Some(file_size),
                    modified_at,
                    glob_policy_metadata: None,
                    captured_at: Some(chrono::Utc::now().to_rfc3339()),
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
                        file_size: Some(file_size),
                        modified_at,
                        glob_policy_metadata: None,
                        captured_at: Some(chrono::Utc::now().to_rfc3339()),
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
                        file_size: Some(file_size),
                        modified_at,
                        glob_policy_metadata: None,
                        captured_at: Some(chrono::Utc::now().to_rfc3339()),
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
                    file_size: Some(file_size),
                    modified_at,
                    glob_policy_metadata: None,
                    captured_at: Some(chrono::Utc::now().to_rfc3339()),
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
                file_size: Some(file_size),
                modified_at,
                glob_policy_metadata: Some(
                    serde_json::json!({
                        "recursive": recursive,
                        "include_globs": include_globs,
                        "exclude_globs": exclude_globs,
                    })
                    .to_string(),
                ),
                captured_at: Some(chrono::Utc::now().to_rfc3339()),
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
        let final_metrics_only = override_metrics_only.unwrap_or(policy.metrics_only);
        let max_synthesis_words = policy
            .source_sample_word_budget
            .unwrap_or(MAX_STYLE_SYNTHESIS_SOURCE_WORDS);

        let mut prompt_chunks = Vec::new();
        let mut prompt_source_words = 0usize;
        if !final_metrics_only {
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

        let corpus_section = if final_metrics_only {
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
                project_id: Some(project_id.to_string()),
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
                        project_id: Some(project_id.to_string()),
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

        let final_policy = StyleProfileSourcePolicy {
            local_user_provided: true,
            source_text_persisted: false,
            max_excerpt_words: 0,
            allowed_roots: allowed_roots
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            metrics_only: final_metrics_only,
            source_sample_word_budget: policy.source_sample_word_budget,
            source_paths,
            recursive: Some(recursive),
            include_globs: policy.include_globs.clone(),
            exclude_globs: policy.exclude_globs.clone(),
            max_files: Some(max_files),
            max_bytes_per_file: Some(max_bytes),
            max_total_words: Some(max_words),
        };

        Ok(StyleProfileCard {
            profile_id,
            project_id: project_id.to_string(),
            name: name.to_string(),
            status,
            created_at: now_str.clone(),
            updated_at: now_str,
            corpus: corpus_summary,
            metrics,
            guidance: final_guidance,
            source_policy: final_policy,
            model_receipt: receipt,
            quality,
            archived_at: None,
            parent_profile_id: None,
            refreshed_from_profile_id: None,
            version_number: None,
            refreshed_at: None,
        })
    }

    pub async fn create_style_profile_from_markdown(
        &self,
        input: CreateStyleProfileFromMarkdownInput,
    ) -> Result<CreateStyleProfileFromMarkdownOutput> {
        let _project = self.repository().get_project(&input.project_id).await?;

        let data_dir = self.repository().data_dir().to_path_buf();
        let mut allowed_roots = vec![data_dir.clone()];
        if let Some(parent) = data_dir.parent() {
            allowed_roots.push(parent.to_path_buf());
        }

        let policy = StyleProfileSourcePolicy {
            local_user_provided: true,
            source_text_persisted: false,
            max_excerpt_words: 0,
            allowed_roots: allowed_roots
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            metrics_only: input.metrics_only.unwrap_or(false),
            source_sample_word_budget: input.source_sample_word_budget,
            source_paths: input.source_paths.clone(),
            recursive: input.recursive,
            include_globs: input.include_globs.clone(),
            exclude_globs: input.exclude_globs.clone(),
            max_files: input.max_files,
            max_bytes_per_file: input.max_bytes_per_file,
            max_total_words: input.max_total_words,
        };

        let profile = self
            .generate_candidate_style_profile(&input.project_id, &input.profile_name, &policy, None)
            .await?;

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

        // Persist profile
        self.repository().insert_style_profile(&profile).await?;

        // Optionally apply profile
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

    pub async fn check_style_profile_sources(
        &self,
        input: CheckStyleProfileSourcesInput,
    ) -> Result<CheckStyleProfileSourcesOutput> {
        let profile = self
            .repository()
            .get_style_profile(&input.project_id, &input.profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found"))?;

        if profile.archived_at.is_some() && !input.include_archived.unwrap_or(false) {
            return Err(anyhow!(
                "Cannot check archived style profile unless include_archived is true"
            ));
        }

        let data_dir = self.repository().data_dir().to_path_buf();
        let mut allowed_roots = vec![data_dir.clone()];
        if let Some(parent) = data_dir.parent() {
            allowed_roots.push(parent.to_path_buf());
        }

        let source_paths = if profile.source_policy.source_paths.is_empty() {
            // Fallback to original source_refs paths
            profile
                .corpus
                .source_refs
                .iter()
                .filter(|r| r.included)
                .map(|r| r.canonical_path.clone())
                .collect::<Vec<_>>()
        } else {
            profile.source_policy.source_paths.clone()
        };

        let recursive = profile.source_policy.recursive.unwrap_or(false);
        let include_globs = profile.source_policy.include_globs.as_deref();
        let exclude_globs = profile.source_policy.exclude_globs.as_deref();
        let max_files = profile.source_policy.max_files.unwrap_or(100);

        let mut missing_source_roots = Vec::new();
        let mut valid_source_paths = Vec::new();
        for path_str in &source_paths {
            let path = std::path::Path::new(path_str);
            if !path.exists() {
                missing_source_roots.push(format!("Source path '{}' does not exist", path_str));
                continue;
            }
            match style_helper::resolve_and_verify_path(path_str, &allowed_roots) {
                Ok(_) => {
                    valid_source_paths.push(path_str.clone());
                }
                Err(e) => {
                    if e.to_string().contains("outside allowed roots") {
                        return Err(e);
                    } else {
                        missing_source_roots.push(format!(
                            "Source path '{}' could not be verified: {}",
                            path_str, e
                        ));
                    }
                }
            }
        }

        let current_files = if valid_source_paths.is_empty() {
            Vec::new()
        } else {
            style_helper::collect_markdown_files(
                &valid_source_paths,
                recursive,
                &allowed_roots,
                max_files,
                include_globs,
                exclude_globs,
            )?
        };

        let mut current_refs = Vec::new();
        for path in &current_files {
            let display_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            let canonical_path_str = path.to_string_lossy().to_string();
            let metadata = match fs::metadata(path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !metadata.is_file() {
                continue;
            }
            let file_size = metadata.len();
            let modified_at = metadata.modified().ok().map(|t| {
                let datetime: chrono::DateTime<chrono::Utc> = t.into();
                datetime.to_rfc3339()
            });
            let bytes = match fs::read(path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let sha256_hash = format!("{:x}", sha2::Sha256::digest(&bytes));

            let text = String::from_utf8_lossy(&bytes);
            let word_count = text.split_whitespace().count();

            current_refs.push(StyleSourceRef {
                display_name,
                canonical_path: canonical_path_str,
                sha256: sha256_hash,
                word_count,
                included: true,
                skip_reason: None,
                file_size: Some(file_size),
                modified_at,
                glob_policy_metadata: Some(
                    serde_json::json!({
                        "recursive": recursive,
                        "include_globs": include_globs,
                        "exclude_globs": exclude_globs,
                    })
                    .to_string(),
                ),
                captured_at: Some(chrono::Utc::now().to_rfc3339()),
            });
        }

        let mut added_files = Vec::new();
        let mut removed_files = Vec::new();
        let mut changed_files = Vec::new();
        let mut unchanged_count = 0;

        let original_map: std::collections::HashMap<String, StyleSourceRef> = profile
            .corpus
            .source_refs
            .iter()
            .map(|r| (r.canonical_path.clone(), r.clone()))
            .collect();

        let current_map: std::collections::HashMap<String, StyleSourceRef> = current_refs
            .iter()
            .map(|r| (r.canonical_path.clone(), r.clone()))
            .collect();

        for path in current_map.keys() {
            if !original_map.contains_key(path) {
                added_files.push(path.clone());
            }
        }

        for (path, ref_orig) in &original_map {
            if ref_orig.included && !current_map.contains_key(path) {
                removed_files.push(path.clone());
            }
        }

        for (path, ref_curr) in &current_map {
            if let Some(ref_orig) = original_map.get(path) {
                // A previously-excluded file that is now present, or one whose
                // content hash changed, both count as a change.
                if !ref_orig.included || ref_curr.sha256 != ref_orig.sha256 {
                    changed_files.push(path.clone());
                } else {
                    unchanged_count += 1;
                }
            }
        }

        let stale =
            !added_files.is_empty() || !removed_files.is_empty() || !changed_files.is_empty();
        let can_refresh = !current_refs.is_empty();

        Ok(CheckStyleProfileSourcesOutput {
            profile_id: input.profile_id.clone(),
            stale,
            added_files,
            removed_files,
            changed_files,
            unchanged_count,
            missing_source_roots,
            can_refresh,
        })
    }

    pub async fn preview_refresh_style_profile(
        &self,
        input: PreviewRefreshStyleProfileInput,
    ) -> Result<PreviewRefreshStyleProfileOutput> {
        let old_profile = self
            .repository()
            .get_style_profile(&input.project_id, &input.profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found"))?;

        if old_profile.archived_at.is_some() {
            return Err(anyhow!("Cannot preview refresh an archived style profile"));
        }

        let candidate = self
            .generate_candidate_style_profile(
                &input.project_id,
                &old_profile.name,
                &old_profile.source_policy,
                input.metrics_only,
            )
            .await?;

        let metric_deltas = compute_metrics_deltas(&old_profile.metrics, &candidate.metrics);

        let mut change_reasons = Vec::new();
        if candidate.guidance.pov != old_profile.guidance.pov {
            change_reasons.push(format!(
                "POV changed from {:?} to {:?}",
                old_profile.guidance.pov, candidate.guidance.pov
            ));
        }
        if candidate.guidance.tense != old_profile.guidance.tense {
            change_reasons.push(format!(
                "Tense changed from {:?} to {:?}",
                old_profile.guidance.tense, candidate.guidance.tense
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
        let material_change = !change_reasons.is_empty();

        let mut apply_safety = Vec::new();
        if candidate.quality.classification
            != spindle_core::style::StyleProfileQualityClassification::Ready
        {
            apply_safety.push(format!(
                "Candidate profile quality is {:?}. Auto-applying this profile will be BLOCKED unless force_apply=true is used.",
                candidate.quality.classification
            ));
        }
        if candidate.quality.confidence_score < 0.6 {
            apply_safety.push(format!(
                "Candidate confidence score is low ({:.2}). Rebuilt profile might be unstable.",
                candidate.quality.confidence_score
            ));
        }
        if candidate.guidance.pov != old_profile.guidance.pov {
            apply_safety.push(format!(
                "POV changes from {:?} to {:?}. Applying this profile will modify narrator rules and reader contract.",
                old_profile.guidance.pov, candidate.guidance.pov
            ));
        }
        if candidate.guidance.tense != old_profile.guidance.tense {
            apply_safety.push(format!(
                "Tense changes from {:?} to {:?}. Applying this profile will modify narrator rules and reader contract.",
                old_profile.guidance.tense, candidate.guidance.tense
            ));
        }

        Ok(PreviewRefreshStyleProfileOutput {
            old_profile_summary: old_profile,
            candidate_profile_summary: candidate.clone(),
            quality_report: candidate.quality,
            metric_deltas,
            material_change,
            apply_safety,
        })
    }

    pub async fn refresh_style_profile(
        &self,
        input: RefreshStyleProfileInput,
    ) -> Result<RefreshStyleProfileOutput> {
        let old_profile = self
            .repository()
            .get_style_profile(&input.project_id, &input.profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found"))?;

        if old_profile.archived_at.is_some() {
            return Err(anyhow!("Cannot refresh an archived style profile"));
        }

        let mut candidate = self
            .generate_candidate_style_profile(
                &input.project_id,
                &old_profile.name,
                &old_profile.source_policy,
                input.metrics_only,
            )
            .await?;

        let parent_id = old_profile
            .parent_profile_id
            .clone()
            .unwrap_or_else(|| old_profile.profile_id.clone());
        candidate.parent_profile_id = Some(parent_id);
        candidate.refreshed_from_profile_id = Some(old_profile.profile_id.clone());
        let old_version = old_profile.version_number.unwrap_or(1);
        candidate.version_number = Some(old_version + 1);
        let now_str = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        candidate.refreshed_at = Some(now_str);

        let should_apply = input.apply_after_refresh.unwrap_or(false);
        if should_apply
            && candidate.quality.classification
                != spindle_core::style::StyleProfileQualityClassification::Ready
            && !input.force_apply.unwrap_or(false)
        {
            return Err(anyhow!(
                "Style profile quality is too low to auto-apply (classification: {:?}). Use force_apply=true to override.",
                candidate.quality.classification
            ));
        }

        // Persist new profile
        self.repository().insert_style_profile(&candidate).await?;

        let mut applied = false;
        let mut application = None;
        if should_apply {
            let app_out = self
                .apply_style_profile(ApplyStyleProfileInput {
                    project_id: input.project_id.clone(),
                    profile_id: candidate.profile_id.clone(),
                    mode: StyleProfileApplyMode::Merge,
                })
                .await?;
            applied = true;
            application = Some(app_out);
        }

        Ok(RefreshStyleProfileOutput {
            new_profile: candidate,
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
                if a.id != app.id && a.rollback_status != "rolled_back"
                    && let Ok(Some(prof)) = self
                        .repository()
                        .get_style_profile(&input.project_id, &a.profile_id)
                        .await
                        && prof.archived_at.is_none() {
                            previous_active = Some(a.profile_id);
                            break;
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
            if !(0.7..=1.3).contains(&ratio) {
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
            if !(0.6..=1.5).contains(&ratio) {
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
        let include_rewrite_examples = input.include_rewrite_examples.unwrap_or(false);
        let max_rewrite_examples = input.max_suggestions.unwrap_or(3);
        let mut rewrite_examples = None;
        if include_rewrite_examples
            && !input.metrics_only.unwrap_or(false)
            && max_rewrite_examples == 0
        {
            rewrite_examples = Some(Vec::new());
        } else if include_rewrite_examples && !input.metrics_only.unwrap_or(false) {
            let profile = self
                .repository()
                .get_style_profile(&input.project_id, &check_output.profile_id)
                .await?
                .ok_or_else(|| anyhow!("style profile not found: {}", check_output.profile_id))?;

            let trimmed_prose = trim_to_word_limit(&target_prose, 4000);
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
                max_rewrite_examples
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
                if let Ok(mut parsed) = serde_json::from_str::<
                    Vec<spindle_core::style::StyleRevisionPlanExample>,
                >(cleaned)
                {
                    parsed.truncate(max_rewrite_examples);
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
        if let Some(active_id) = &project.active_style_profile_id
            && active_id == &input.profile_id && !input.force.unwrap_or(false) {
                return Err(anyhow!(
                    "Cannot archive the active style profile unless force=true is provided"
                ));
            }

        let archived_at = self
            .repository()
            .archive_style_profile(&input.project_id, &input.profile_id)
            .await?;

        if let Some(active_id) = &project.active_style_profile_id
            && active_id == &input.profile_id {
                self.repository()
                    .set_active_style_profile_id(&input.project_id, None)
                    .await?;
            }

        Ok(spindle_core::style::ArchiveStyleProfileOutput {
            project_id: input.project_id,
            profile_id: input.profile_id,
            archived_at,
        })
    }

    pub async fn preview_style_revision_patch(
        &self,
        input: spindle_core::style::PreviewStyleRevisionPatchInput,
    ) -> Result<spindle_core::style::PreviewStyleRevisionPatchOutput> {
        // 1. Accept exactly one target: scene_id or chapter_id
        match (&input.scene_id, &input.chapter_id) {
            (Some(_), Some(_)) => {
                return Err(anyhow!("Provide only one of scene_id or chapter_id"));
            }
            (None, None) => return Err(anyhow!("Must provide either scene_id or chapter_id")),
            _ => {}
        }

        // 2. Reuse plan_style_revision's validation by invoking plan_style_revision
        // This resolves active profile, validates scene/chapter project ownership, and rejects archived profiles
        let plan_input = spindle_core::style::PlanStyleRevisionInput {
            project_id: input.project_id.clone(),
            profile_id: input.profile_id.clone(),
            raw_text: None,
            scene_id: input.scene_id.clone(),
            chapter_id: input.chapter_id.clone(),
            max_suggestions: input.max_suggestions,
            metrics_only: Some(false),
            include_rewrite_examples: Some(false),
        };
        let plan_output = self.plan_style_revision(plan_input).await?;
        let resolved_profile_id = plan_output.profile_id.clone();

        // 3. Retrieve target scenes
        let mut scenes = Vec::new();
        if let Some(sid) = &input.scene_id {
            let scene = self.repository().get_scene(sid).await?;
            scenes.push(scene);
        } else if let Some(cid) = &input.chapter_id {
            let chapter_scenes = self.repository().list_scenes_by_chapter(cid).await?;
            scenes.extend(chapter_scenes);
        }

        // Fetch style profile
        let profile = self
            .repository()
            .get_style_profile(&input.project_id, &resolved_profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found: {}", resolved_profile_id))?;

        let mut scene_patches = Vec::new();
        let mut last_receipt = None;

        for scene in scenes {
            // Cap prose sent to the model with a configurable word budget (default 4000)
            let word_budget = 4000;
            let trimmed_prose = trim_to_word_limit(&scene.full_text, word_budget);

            let mut prompt = format!(
                "You are an expert editor. Revise the following prose to align with the style profile.\n\n\
                 Style Profile Name: {}\n\
                 - POV: {}\n\
                 - Tense: {}\n\
                 - Narrative Voice: {:?}\n\
                 - Do Rules: {:?}\n\
                 - Avoid Rules: {:?}\n\n\
                 Prose to Revise:\n\
                 {}\n\n",
                profile.name,
                profile.guidance.pov.clone().unwrap_or_default(),
                profile.guidance.tense.clone().unwrap_or_default(),
                profile.guidance.narrator_voice,
                profile.guidance.do_rules,
                profile.guidance.avoid_rules,
                trimmed_prose
            );

            if let Some(inst) = &input.instructions {
                prompt.push_str(&format!("Instructions: {}\n\n", inst));
            } else {
                prompt.push_str(
                    "Instructions: Revise the prose to match the style profile guidance.\n\n",
                );
            }

            prompt.push_str("Return ONLY the revised text. Do not include any explanations, markdown formatting, or tags.");

            let req = crate::ai::ModelRequest {
                route: "style_revise".to_string(),
                prompt,
                rating: None,
                context: Some(crate::ai::RequestContext {
                    project_id: Some(input.project_id.clone()),
                    book_id: None,
                    chapter_id: input.chapter_id.clone(),
                    scene_id: Some(scene.id.clone()),
                }),
            };

            let response = self.repository().model_router().complete(&req).await?;
            let revised_text = response.output.trim().to_string();

            let receipt = StyleProfileModelReceipt {
                model_route: "style_revise".to_string(),
                model_name: response.model_name,
                input_tokens: None,
                output_tokens: None,
                latency_ms: None,
            };
            last_receipt = Some(receipt);

            // Compute word counts
            let original_word_count = scene.full_text.split_whitespace().count();
            let revised_word_count = revised_text.split_whitespace().count();

            // Compute hashes
            let before_hash = hash_text(&scene.full_text);
            let after_hash = hash_text(&revised_text);

            // Compute unified diff and structured hunks
            let (unified_diff, hunks) = generate_diff_and_hunks(&scene.full_text, &revised_text);

            scene_patches.push(spindle_core::style::StyleRevisionPatchScene {
                scene_id: scene.id.clone(),
                original_word_count,
                revised_word_count,
                before_hash,
                after_hash,
                unified_diff,
                hunks: Some(hunks),
                revised_text,
            });
        }

        let evaluation = if input.run_evaluation == Some(true) {
            let eval_input = spindle_core::style::EvaluateStyleRevisionPatchInput {
                project_id: input.project_id.clone(),
                profile_id: resolved_profile_id.clone(),
                scenes: scene_patches.clone(),
                run_validator_preflight: input.run_validator_preflight,
                minimum_improvement_score: input.minimum_improvement_score,
            };
            Some(self.evaluate_style_revision_patch(eval_input).await?)
        } else {
            None
        };

        Ok(spindle_core::style::PreviewStyleRevisionPatchOutput {
            project_id: input.project_id,
            profile_id: resolved_profile_id,
            scenes: scene_patches,
            model_receipt: last_receipt,
            evaluation,
        })
    }

    pub async fn evaluate_style_revision_patch(
        &self,
        input: spindle_core::style::EvaluateStyleRevisionPatchInput,
    ) -> Result<spindle_core::style::EvaluateStyleRevisionPatchOutput> {
        // 1. Fetch profile card and reject if archived
        let profile = self
            .repository()
            .get_style_profile(&input.project_id, &input.profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found: {}", input.profile_id))?;
        ensure_style_profile_not_archived(&profile)?;

        // 2. Resolve active branch ID
        let active_branch_id = self
            .repository()
            .active_branch_id_public(&input.project_id)
            .await?;

        if input.scenes.is_empty() {
            return Err(anyhow!(
                "style revision patch evaluation must include at least one scene"
            ));
        }

        let mut total_before_warnings = 0;
        let mut total_after_warnings = 0;
        let mut total_before_errors = 0;
        let mut total_after_errors = 0;
        let mut total_improvement = 0.0;
        let mut scene_evals = Vec::new();
        let mut aggregate_risks = Vec::new();
        let mut seen_scene_ids = std::collections::HashSet::new();

        // 3. Evaluate each scene patch
        for scene_patch in &input.scenes {
            if !seen_scene_ids.insert(scene_patch.scene_id.clone()) {
                return Err(anyhow!(
                    "duplicate scene in style revision patch evaluation: {}",
                    scene_patch.scene_id
                ));
            }
            let existing_scene = self.repository().get_scene(&scene_patch.scene_id).await?;
            if existing_scene.project_id != input.project_id {
                return Err(anyhow!(
                    "Scene {} does not belong to project {}",
                    scene_patch.scene_id,
                    input.project_id
                ));
            }
            if existing_scene.branch_id != active_branch_id {
                return Err(anyhow!(
                    "Scene {} does not belong to active branch {}",
                    scene_patch.scene_id,
                    active_branch_id
                ));
            }

            // Verify before_hash matches current scene text
            let current_hash = hash_text(&existing_scene.full_text);
            if current_hash != scene_patch.before_hash {
                return Err(anyhow!(
                    "Scene {} text has changed; patch is stale",
                    scene_patch.scene_id
                ));
            }

            // Verify after_hash matches revised_text
            let computed_after_hash = hash_text(&scene_patch.revised_text);
            if computed_after_hash != scene_patch.after_hash {
                return Err(anyhow!(
                    "style revision patch after_hash does not match revised text for scene {}",
                    scene_patch.scene_id
                ));
            }

            // Run style drift checks
            let before_drift_findings = self.check_style_against_prose(
                &profile,
                &existing_scene.full_text,
                Some(scene_patch.scene_id.clone()),
            );
            let after_drift_findings = self.check_style_against_prose(
                &profile,
                &scene_patch.revised_text,
                Some(scene_patch.scene_id.clone()),
            );

            let before_warnings = before_drift_findings
                .iter()
                .filter(|f| f.severity == "warning")
                .count();
            let before_errors = before_drift_findings
                .iter()
                .filter(|f| f.severity == "error")
                .count();
            let after_warnings = after_drift_findings
                .iter()
                .filter(|f| f.severity == "warning")
                .count();
            let after_errors = after_drift_findings
                .iter()
                .filter(|f| f.severity == "error")
                .count();

            let before_score_val = before_warnings + 3 * before_errors;
            let after_score_val = after_warnings + 3 * after_errors;
            let improvement_score = (before_score_val as f64) - (after_score_val as f64);

            let status = if improvement_score > 0.0 {
                spindle_core::style::StyleRevisionPatchStatus::Improved
            } else if improvement_score < 0.0 {
                spindle_core::style::StyleRevisionPatchStatus::Regressed
            } else {
                spindle_core::style::StyleRevisionPatchStatus::Neutral
            };

            let mut risks = Vec::new();
            if after_score_val > before_score_val {
                risks.push(spindle_core::style::StyleRevisionPatchRisk {
                    risk_type: "increased_style_drift".to_string(),
                    severity: "warning".to_string(),
                    description: format!(
                        "Style drift findings increased from (warnings: {}, errors: {}) to (warnings: {}, errors: {}).",
                        before_warnings, before_errors, after_warnings, after_errors
                    ),
                });
            }

            let original_word_count = existing_scene.full_text.split_whitespace().count();
            let revised_word_count = scene_patch.revised_text.split_whitespace().count();
            let orig_words = original_word_count as f64;
            let rev_words = revised_word_count as f64;
            if orig_words > 0.0 {
                let ratio = (rev_words - orig_words).abs() / orig_words;
                if ratio >= 0.30 {
                    risks.push(spindle_core::style::StyleRevisionPatchRisk {
                        risk_type: "large_word_count_swing".to_string(),
                        severity: "warning".to_string(),
                        description: format!(
                            "Prose word count changed significantly by {:.1}% (from {} to {} words).",
                            ratio * 100.0,
                            original_word_count,
                            revised_word_count
                        ),
                    });
                }
            }

            if scene_patch.revised_text.trim().is_empty() {
                risks.push(spindle_core::style::StyleRevisionPatchRisk {
                    risk_type: "empty_revised_prose".to_string(),
                    severity: "error".to_string(),
                    description: "Revised prose is completely empty.".to_string(),
                });
            } else if revised_word_count < 5 || scene_patch.revised_text.trim().chars().count() < 15
            {
                risks.push(spindle_core::style::StyleRevisionPatchRisk {
                    risk_type: "near_empty_revised_prose".to_string(),
                    severity: "warning".to_string(),
                    description: format!(
                        "Revised prose is extremely short ({} words, {} characters).",
                        revised_word_count,
                        scene_patch.revised_text.trim().chars().count()
                    ),
                });
            }

            let (before_rating, _) =
                crate::sqlite::import_service::detect_content_rating(&existing_scene.full_text);
            let (after_rating, _) =
                crate::sqlite::import_service::detect_content_rating(&scene_patch.revised_text);
            if before_rating != after_rating {
                risks.push(spindle_core::style::StyleRevisionPatchRisk {
                    risk_type: "content_rating_change".to_string(),
                    severity: "warning".to_string(),
                    description: format!(
                        "Detected content rating change from {:?} to {:?}.",
                        before_rating, after_rating
                    ),
                });
            }

            if crate::sqlite::import_service::contains_explicit_sexual_prose(
                &scene_patch.revised_text,
            ) {
                risks.push(spindle_core::style::StyleRevisionPatchRisk {
                    risk_type: "unsafe_explicit_save_constraint".to_string(),
                    severity: "error".to_string(),
                    description: "Revised prose contains explicit sexual prose but has no generation_id, which will block saving.".to_string(),
                });
            }

            // Run validator preflight
            if input.run_validator_preflight == Some(true) {
                let mut preflight_context = self
                    .build_phase_four_validator_context(
                        &input.project_id,
                        &active_branch_id,
                        std::slice::from_ref(&existing_scene),
                    )
                    .await?;
                if let Some(snap) = preflight_context.scenes.iter_mut().next() {
                    snap.full_text = scene_patch.revised_text.clone();
                }
                let registry = crate::sqlite::validators::phase_four_validator_registry();
                if let Some(snap) = preflight_context.scenes.first() {
                    match registry.validate_scene(snap, &preflight_context) {
                        Ok(findings) => {
                            for finding in findings {
                                let sev = match finding.severity {
                                    spindle_core::validators::ValidatorSeverity::Error => "error",
                                    spindle_core::validators::ValidatorSeverity::Warning => {
                                        "warning"
                                    }
                                    spindle_core::validators::ValidatorSeverity::Info => "info",
                                };
                                if sev == "error" || sev == "warning" {
                                    risks.push(spindle_core::style::StyleRevisionPatchRisk {
                                        risk_type: format!("preflight:{}", finding.check_type),
                                        severity: sev.to_string(),
                                        description: finding.message,
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            risks.push(spindle_core::style::StyleRevisionPatchRisk {
                                risk_type: "preflight_error".to_string(),
                                severity: "error".to_string(),
                                description: format!("Validator preflight failed to run: {}", e),
                            });
                        }
                    }
                }
            }

            total_before_warnings += before_warnings;
            total_after_warnings += after_warnings;
            total_before_errors += before_errors;
            total_after_errors += after_errors;
            total_improvement += improvement_score;

            aggregate_risks.extend(risks.clone());

            scene_evals.push(spindle_core::style::StyleRevisionPatchEvaluation {
                scene_id: scene_patch.scene_id.clone(),
                score: spindle_core::style::StyleRevisionPatchScore {
                    before_warnings,
                    after_warnings,
                    before_errors,
                    after_errors,
                    improvement_score,
                },
                status,
                risks,
            });
        }

        let aggregate_score = spindle_core::style::StyleRevisionPatchScore {
            before_warnings: total_before_warnings,
            after_warnings: total_after_warnings,
            before_errors: total_before_errors,
            after_errors: total_after_errors,
            improvement_score: total_improvement,
        };

        let aggregate_status = if total_improvement > 0.0 {
            spindle_core::style::StyleRevisionPatchStatus::Improved
        } else if total_improvement < 0.0 {
            spindle_core::style::StyleRevisionPatchStatus::Regressed
        } else {
            spindle_core::style::StyleRevisionPatchStatus::Neutral
        };

        if let Some(min_score) = input.minimum_improvement_score
            && total_improvement < min_score {
                aggregate_risks.push(spindle_core::style::StyleRevisionPatchRisk {
                    risk_type: "minimum_improvement_score_failed".to_string(),
                    severity: "error".to_string(),
                    description: format!(
                        "Aggregate improvement score ({:.2}) is less than the required minimum threshold ({:.2}).",
                        total_improvement, min_score
                    ),
                });
            }

        Ok(spindle_core::style::EvaluateStyleRevisionPatchOutput {
            project_id: input.project_id,
            profile_id: input.profile_id,
            scenes: scene_evals,
            aggregate_score,
            status: aggregate_status,
            risks: aggregate_risks,
        })
    }

    pub async fn apply_style_revision_patch(
        &self,
        input: spindle_core::style::ApplyStyleRevisionPatchInput,
    ) -> Result<spindle_core::style::ApplyStyleRevisionPatchOutput> {
        // 1. Fetch profile card and reject if archived
        let profile = self
            .repository()
            .get_style_profile(&input.project_id, &input.profile_id)
            .await?
            .ok_or_else(|| anyhow!("style profile not found: {}", input.profile_id))?;
        ensure_style_profile_not_archived(&profile)?;

        if input.scenes.is_empty() {
            return Err(anyhow!(
                "style revision patch must include at least one scene"
            ));
        }

        // Apply integration check:
        if input.require_positive_evaluation == Some(true) {
            let eval_input = spindle_core::style::EvaluateStyleRevisionPatchInput {
                project_id: input.project_id.clone(),
                profile_id: input.profile_id.clone(),
                scenes: input.scenes.clone(),
                run_validator_preflight: Some(false),
                minimum_improvement_score: input.minimum_improvement_score,
            };
            let eval_output = self.evaluate_style_revision_patch(eval_input).await?;
            if eval_output.status == spindle_core::style::StyleRevisionPatchStatus::Regressed {
                return Err(anyhow!("style revision patch evaluation regressed"));
            }
            if let Some(min_score) = input.minimum_improvement_score
                && eval_output.aggregate_score.improvement_score < min_score {
                    return Err(anyhow!(
                        "style revision patch evaluation failed minimum threshold: score {:.2} < required {:.2}",
                        eval_output.aggregate_score.improvement_score,
                        min_score
                    ));
                }
        }

        // 2. Validate all scene targets for project ownership and stale hashes
        let active_branch_id = self
            .repository()
            .active_branch_id_public(&input.project_id)
            .await?;
        let mut seen_scene_ids = std::collections::HashSet::new();
        let mut scenes_to_save = Vec::new();
        let mut before_hashes = Vec::new();
        let mut after_hashes = Vec::new();
        let mut target_ids = Vec::new();

        for scene_patch in &input.scenes {
            if !seen_scene_ids.insert(scene_patch.scene_id.clone()) {
                return Err(anyhow!(
                    "duplicate scene in style revision patch: {}",
                    scene_patch.scene_id
                ));
            }
            if scene_patch.revised_text.trim().is_empty() {
                return Err(anyhow!(
                    "style revision patch revised text is empty for scene {}",
                    scene_patch.scene_id
                ));
            }
            let computed_after_hash = hash_text(&scene_patch.revised_text);
            if computed_after_hash != scene_patch.after_hash {
                return Err(anyhow!(
                    "style revision patch after_hash does not match revised text for scene {}",
                    scene_patch.scene_id
                ));
            }

            let existing_scene = self.repository().get_scene(&scene_patch.scene_id).await?;
            if existing_scene.project_id != input.project_id {
                return Err(anyhow!(
                    "Scene {} does not belong to project {}",
                    scene_patch.scene_id,
                    input.project_id
                ));
            }
            if existing_scene.branch_id != active_branch_id {
                return Err(anyhow!(
                    "Scene {} does not belong to active branch {}",
                    scene_patch.scene_id,
                    active_branch_id
                ));
            }

            // Check stale hash
            let current_hash = hash_text(&existing_scene.full_text);
            if current_hash != scene_patch.before_hash {
                return Err(anyhow!(
                    "Scene {} text has changed; patch is stale",
                    scene_patch.scene_id
                ));
            }

            scenes_to_save.push((existing_scene, scene_patch.revised_text.clone()));
            before_hashes.push(scene_patch.before_hash.clone());
            after_hashes.push(scene_patch.after_hash.clone());
            target_ids.push(scene_patch.scene_id.clone());
        }

        // 3. Save scene drafts through existing save_scene_draft path
        let mut applied_scene_ids = Vec::new();
        for (existing_scene, revised_text) in scenes_to_save {
            let content_rating = match existing_scene.content_rating.as_str() {
                "General" | "general" => spindle_core::models::ContentRating::General,
                "Teen" | "teen" => spindle_core::models::ContentRating::Teen,
                "Mature" | "mature" => spindle_core::models::ContentRating::Mature,
                "Explicit" | "explicit" => spindle_core::models::ContentRating::Explicit,
                other => return Err(anyhow!("unknown content_rating in scene: {}", other)),
            };

            let save_input = spindle_core::models::SaveSceneDraftInput {
                project_id: input.project_id.clone(),
                book_number: existing_scene.book_number,
                chapter_number: existing_scene.chapter_number,
                chapter_id: Some(existing_scene.chapter_id.clone()),
                scene_order: existing_scene.scene_order,
                full_text: revised_text,
                summary: existing_scene.summary.clone(),
                content_rating,
                tone: existing_scene.tone.clone(),
                generation_id: None,
                source_path: None,
                research_source_ids: Vec::new(),
                research_note_ids: Vec::new(),
                research_claim_ids: Vec::new(),
                research_query_pack_input: None,
                research_context_hash: None,
            };

            self.save_scene_draft(save_input).await?;
            applied_scene_ids.push(existing_scene.id);
        }

        // 4. Invalidate style-sensitive validator caches
        self.resolve_phase_four_caches_for_project(
            &input.project_id,
            &[
                PhaseFourCacheId::StyleCompliance,
                PhaseFourCacheId::WorldRuleSemanticDrift,
            ],
        )
        .await?;

        // 5. Record an audit row
        let audit_id = format!("style_patch_audit:{}", ulid::Ulid::new());
        let applied_at = chrono::Utc::now().to_rfc3339();

        let audit_record = spindle_core::style::StyleRevisionPatchAuditRecord {
            id: audit_id.clone(),
            project_id: input.project_id.clone(),
            profile_id: input.profile_id.clone(),
            applied_at,
            target_ids,
            before_hashes,
            after_hashes,
            model_receipt: input.model_receipt.clone(),
            rolled_back_at: None,
            rollback_status: "not_rolled_back".to_string(),
        };

        self.repository()
            .insert_style_revision_patch_audit(&audit_record)
            .await?;

        Ok(spindle_core::style::ApplyStyleRevisionPatchOutput {
            project_id: input.project_id,
            applied_scene_ids,
            audit_id,
        })
    }

    pub async fn list_style_revision_patch_audits(
        &self,
        input: spindle_core::style::ListStyleRevisionPatchAuditsInput,
    ) -> Result<spindle_core::style::ListStyleRevisionPatchAuditsOutput> {
        let audits = self
            .repository()
            .list_style_revision_patch_audits(&input.project_id)
            .await?;
        Ok(spindle_core::style::ListStyleRevisionPatchAuditsOutput { audits })
    }

    pub async fn rollback_style_revision_patch(
        &self,
        input: spindle_core::style::RollbackStyleRevisionPatchInput,
    ) -> Result<spindle_core::style::RollbackStyleRevisionPatchOutput> {
        // 1. Load the audit row
        let audit = self
            .repository()
            .get_style_revision_patch_audit(&input.project_id, &input.audit_id)
            .await?
            .ok_or_else(|| anyhow!("style revision patch audit not found: {}", input.audit_id))?;

        // 2. Reject if already rolled back
        if audit.rollback_status == "rolled_back" {
            return Err(anyhow!(
                "style revision patch audit {} has already been rolled back",
                input.audit_id
            ));
        }

        if audit.target_ids.is_empty() {
            return Err(anyhow!("style revision patch audit has no target scenes"));
        }
        if audit.before_hashes.len() != audit.target_ids.len()
            || audit.after_hashes.len() != audit.target_ids.len()
        {
            return Err(anyhow!(
                "style revision patch audit has mismatched target/hash counts"
            ));
        }

        // 3. Validations: all target scenes must belong to the project and active branch
        let active_branch_id = self
            .repository()
            .active_branch_id_public(&input.project_id)
            .await?;

        let mut seen_target_ids = std::collections::HashSet::new();
        let mut scenes_and_versions = Vec::new();

        for (i, target_id) in audit.target_ids.iter().enumerate() {
            if !seen_target_ids.insert(target_id.clone()) {
                return Err(anyhow!(
                    "style revision patch audit contains duplicate target scene: {}",
                    target_id
                ));
            }
            let before_hash = &audit.before_hashes[i];
            let after_hash = &audit.after_hashes[i];

            // Load existing scene
            let existing_scene = self.repository().get_scene(target_id).await?;
            if existing_scene.project_id != input.project_id {
                return Err(anyhow!(
                    "Scene {} does not belong to project {}",
                    target_id,
                    input.project_id
                ));
            }
            if existing_scene.branch_id != active_branch_id {
                return Err(anyhow!(
                    "Scene {} does not belong to active branch {}",
                    target_id,
                    active_branch_id
                ));
            }

            // Reject if current scene hash does not match after_hash
            let current_hash = hash_text(&existing_scene.full_text);
            if current_hash != *after_hash {
                return Err(anyhow!(
                    "Scene {} text has changed; rollback is stale (current hash: {}, expected after_hash: {})",
                    target_id,
                    current_hash,
                    after_hash
                ));
            }

            // Find matching before version
            let versions = self.repository().list_scene_versions(target_id).await?;
            let mut matching_version = None;
            for version in versions {
                if hash_text(&version.full_text) == *before_hash {
                    matching_version = Some(version);
                    break;
                }
            }

            let Some(matched_version) = matching_version else {
                return Err(anyhow!(
                    "No matching prior scene version with before_hash {} found for scene {}",
                    before_hash,
                    target_id
                ));
            };

            scenes_and_versions.push((existing_scene.id.clone(), matched_version));
        }

        // 4. Perform the rollback/restores
        let mut restored_scene_ids = Vec::new();
        for (scene_id, version) in scenes_and_versions {
            self.repository()
                .restore_scene_version_and_mark_reviews_stale(&scene_id, &version)
                .await?;
            restored_scene_ids.push(scene_id);
        }

        // 5. Invalidate caches
        self.resolve_phase_four_caches_for_project(
            &input.project_id,
            &[
                PhaseFourCacheId::StyleCompliance,
                PhaseFourCacheId::WorldRuleSemanticDrift,
            ],
        )
        .await?;

        // 6. Update audit row
        let rolled_back_at = chrono::Utc::now().to_rfc3339();
        self.repository()
            .update_style_revision_patch_audit_rollback(
                &input.audit_id,
                Some(rolled_back_at.clone()),
                "rolled_back",
            )
            .await?;

        Ok(spindle_core::style::RollbackStyleRevisionPatchOutput {
            project_id: input.project_id,
            audit_id: input.audit_id,
            rolled_back_at,
            restored_scene_ids,
        })
    }
}

// ── Shared Apply Helpers ───────────────────────────────────────────

fn is_generated_style_note(note: &str) -> bool {
    note.contains(" (Style Profile: style_profile:")
}

fn compute_metrics_deltas(
    ma: &spindle_core::style::StyleCorpusMetrics,
    mb: &spindle_core::style::StyleCorpusMetrics,
) -> spindle_core::style::StyleCorpusMetricsDeltas {
    spindle_core::style::StyleCorpusMetricsDeltas {
        average_sentence_words_delta: mb.average_sentence_words - ma.average_sentence_words,
        median_sentence_words_delta: mb.median_sentence_words - ma.median_sentence_words,
        p90_sentence_words_delta: mb.p90_sentence_words - ma.p90_sentence_words,
        average_paragraph_words_delta: mb.average_paragraph_words - ma.average_paragraph_words,
        median_paragraph_words_delta: mb.median_paragraph_words - ma.median_paragraph_words,
        dialogue_line_ratio_delta: mb.dialogue_line_ratio - ma.dialogue_line_ratio,
        dialogue_word_ratio_delta: mb.dialogue_word_ratio - ma.dialogue_word_ratio,
        question_mark_rate_delta: mb.question_mark_rate_per_1k_words
            - ma.question_mark_rate_per_1k_words,
        exclamation_rate_delta: mb.exclamation_rate_per_1k_words - ma.exclamation_rate_per_1k_words,
        semicolon_rate_delta: mb.semicolon_rate_per_1k_words - ma.semicolon_rate_per_1k_words,
        em_dash_rate_delta: mb.em_dash_rate_per_1k_words - ma.em_dash_rate_per_1k_words,
        ellipsis_rate_delta: mb.ellipsis_rate_per_1k_words - ma.ellipsis_rate_per_1k_words,
        first_person_pronoun_rate_delta: mb.first_person_pronoun_rate_per_1k_words
            - ma.first_person_pronoun_rate_per_1k_words,
        third_person_pronoun_rate_delta: mb.third_person_pronoun_rate_per_1k_words
            - ma.third_person_pronoun_rate_per_1k_words,
    }
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

fn hash_text(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn generate_diff_and_hunks(
    old: &str,
    new: &str,
) -> (String, Vec<spindle_core::style::StyleRevisionPatchHunk>) {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // LCS DP
    let m = old_lines.len();
    let n = new_lines.len();
    let mut dp = vec![vec![0; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if old_lines[i - 1] == new_lines[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum EditKind {
        Unchanged,
        Delete,
        Insert,
    }

    struct Edit<'a> {
        kind: EditKind,
        line: &'a str,
        old_idx: Option<usize>,
        new_idx: Option<usize>,
    }

    let mut i = m;
    let mut j = n;
    let mut path = Vec::new();

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old_lines[i - 1] == new_lines[j - 1] {
            path.push(Edit {
                kind: EditKind::Unchanged,
                line: old_lines[i - 1],
                old_idx: Some(i),
                new_idx: Some(j),
            });
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            path.push(Edit {
                kind: EditKind::Insert,
                line: new_lines[j - 1],
                old_idx: None,
                new_idx: Some(j),
            });
            j -= 1;
        } else {
            path.push(Edit {
                kind: EditKind::Delete,
                line: old_lines[i - 1],
                old_idx: Some(i),
                new_idx: None,
            });
            i -= 1;
        }
    }
    path.reverse();

    let mut hunks = Vec::new();
    let mut current_hunk_edits = Vec::new();
    let mut last_change_idx: Option<usize> = None;
    let context_size = 3;

    for (idx, edit) in path.iter().enumerate() {
        if edit.kind != EditKind::Unchanged {
            last_change_idx = Some(idx);
        }
    }

    if last_change_idx.is_none() {
        return (String::new(), Vec::new());
    }

    let mut idx = 0;
    while idx < path.len() {
        // Check if a change is nearby
        let window_end = (idx + context_size * 2 + 1).min(path.len());
        let change_nearby = path[idx..window_end]
            .iter()
            .any(|edit| edit.kind != EditKind::Unchanged);

        if change_nearby {
            current_hunk_edits.push(&path[idx]);
        } else if !current_hunk_edits.is_empty() {
            let mut trailing = 0;
            while idx < path.len()
                && path[idx].kind == EditKind::Unchanged
                && trailing < context_size
            {
                current_hunk_edits.push(&path[idx]);
                trailing += 1;
                idx += 1;
            }
            hunks.push(current_hunk_edits);
            current_hunk_edits = Vec::new();
            continue;
        }
        idx += 1;
    }

    if !current_hunk_edits.is_empty() {
        hunks.push(current_hunk_edits);
    }

    let mut diff_output = String::new();
    diff_output.push_str("--- original\n+++ revised\n");

    let mut structured_hunks = Vec::new();

    for hunk in hunks {
        let old_start = hunk.iter().find_map(|e| e.old_idx).unwrap_or(0);
        let new_start = hunk.iter().find_map(|e| e.new_idx).unwrap_or(0);
        let old_len = hunk.iter().filter(|e| e.kind != EditKind::Insert).count();
        let new_len = hunk.iter().filter(|e| e.kind != EditKind::Delete).count();

        diff_output.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start, old_len, new_start, new_len
        ));

        let mut hunk_lines = Vec::new();
        for edit in hunk {
            let prefix = match edit.kind {
                EditKind::Unchanged => ' ',
                EditKind::Delete => '-',
                EditKind::Insert => '+',
            };
            let line_str = format!("{}{}", prefix, edit.line);
            diff_output.push_str(&line_str);
            diff_output.push('\n');
            hunk_lines.push(line_str);
        }

        structured_hunks.push(spindle_core::style::StyleRevisionPatchHunk {
            old_range: format!("{},{}", old_start, old_len),
            new_range: format!("{},{}", new_start, new_len),
            lines: hunk_lines,
        });
    }

    (diff_output, structured_hunks)
}
