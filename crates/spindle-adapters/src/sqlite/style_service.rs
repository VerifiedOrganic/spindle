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
        let mut prompt_chunks = Vec::new();
        let mut prompt_source_words = 0usize;
        for (i, c) in chunks.iter().enumerate() {
            if prompt_source_words >= MAX_STYLE_SYNTHESIS_SOURCE_WORDS {
                break;
            }
            let remaining_words = MAX_STYLE_SYNTHESIS_SOURCE_WORDS - prompt_source_words;
            let text = trim_to_word_limit(&c.text, remaining_words);
            prompt_source_words += style_helper::count_words(&text);
            prompt_chunks.push(format!(
                "--- Chunk {} (Source: {}) ---\n{}",
                i + 1,
                c.source_display_name,
                text
            ));
        }
        let chunks_str = prompt_chunks.join("\n\n");

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
             [CORPUS CHUNKS]\n\
             Here are representative source samples for style analysis, capped to avoid unbounded prompts. \
             Use them only to derive abstract guidance; do not quote or reuse source phrasing in output.\n\n\
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
            chunks_str
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

        // 2. Set NarratorVoice
        let narrator_voice = profile.guidance.narrator_voice.clone();
        self.repository()
            .set_narrator_voice(&input.project_id, narrator_voice.clone())
            .await?;

        // 3. Update ReaderContract style notes
        let project = self.repository().get_project(&input.project_id).await?;
        let mut reader_contract = project.reader_contract.into_core();

        let mut notes_to_add = Vec::new();
        if !profile.guidance.summary.is_empty() {
            notes_to_add.push(profile.guidance.summary.clone());
        }
        for rule in &profile.guidance.do_rules {
            notes_to_add.push(format!("Do: {}", rule));
        }
        for rule in &profile.guidance.avoid_rules {
            notes_to_add.push(format!("Avoid: {}", rule));
        }

        match input.mode {
            StyleProfileApplyMode::Merge => {
                let existing_set: std::collections::HashSet<String> = reader_contract
                    .style_notes
                    .iter()
                    .map(|n| n.trim().to_lowercase())
                    .collect();

                for note in notes_to_add {
                    if !existing_set.contains(&note.trim().to_lowercase()) {
                        reader_contract.style_notes.push(note);
                    }
                }
            }
            StyleProfileApplyMode::ReplaceGeneratedStyleNotes => {
                reader_contract.style_notes = notes_to_add;
            }
        }

        self.repository()
            .update_project_reader_contract(&input.project_id, &reader_contract)
            .await?;

        // 4. Optionally create/update world rule
        let mut style_rule_id = None;
        if let Some(branch_id) = &project.active_branch_id
            && !profile.guidance.prompt_snippet.trim().is_empty()
        {
            let rules = self
                .repository()
                .list_world_rules_by_project_and_branch(&input.project_id, branch_id)
                .await?;
            let existing = rules.iter().find(|r| {
                r.rule_name == profile.name
                    || r.rule_name == format!("Style Profile: {}", profile.name)
            });

            if let Some(r) = existing {
                let changes = serde_json::json!({
                    "description": profile.guidance.prompt_snippet.clone()
                });
                self.update_world_rule(UpdateWorldRuleInput {
                    world_rule_id: r.id.clone(),
                    changes,
                })
                .await?;
                style_rule_id = Some(r.id.clone());
            } else {
                let rule_out = self
                    .create_world_rule(CreateWorldRuleInput {
                        project_id: input.project_id.clone(),
                        rule_name: format!("Style Profile: {}", profile.name),
                        rule_type: "style".to_string(),
                        description: profile.guidance.prompt_snippet.clone(),
                        scan_pattern: None,
                        relevance_tags: Vec::new(),
                        established_in: None,
                    })
                    .await?;
                style_rule_id = Some(rule_out.world_rule_id);
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

        Ok(ApplyStyleProfileOutput {
            project_id: input.project_id,
            profile_id: input.profile_id,
            narrator_voice,
            reader_contract_style_notes: reader_contract.style_notes,
            style_rule_id,
            invalidated_validator_findings: invalidated,
        })
    }
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
