use super::*;
use spindle_core::canon::{
    KnowledgeValidationFactV1, SceneValidationInputV1, detect_knowledge_paraphrase_leak,
};

/// True when a deep-tier model call exceeded its wall-clock budget. A timeout
/// makes that unit INCONCLUSIVE; it must not be treated as a dead route.
fn is_deep_check_timeout(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<crate::ai::DeepCheckTimedOut>()
        .is_some()
}

impl SqliteSpindleService {
    /// Narrow a scene list to only those that reference any of the
    /// supplied subjects. A scene matches if its id is a direct hit for
    /// the subject (e.g., character_state.scene_id,
    /// scene_beat_annotation.{motif,theme,conflict}_ids) OR its full_text
    /// contains any of the subject's display terms (case-insensitive
    /// for ASCII). Mirrors `services/mod.rs:6193` in 705b835^.
    pub(super) async fn narrow_scenes_by_subjects(
        &self,
        project_id: &str,
        branch_id: &str,
        scenes: Vec<crate::sqlite::records::Scene>,
        subjects: &[String],
    ) -> Result<Vec<crate::sqlite::records::Scene>> {
        use std::collections::BTreeSet;
        if subjects.is_empty() {
            return Ok(scenes);
        }
        let mut direct_scene_ids: BTreeSet<String> = BTreeSet::new();
        let mut term_set: BTreeSet<String> = BTreeSet::new();
        for raw in subjects {
            let spec = self
                .scene_reference_subject_spec(project_id, branch_id, raw)
                .await?;
            direct_scene_ids.extend(spec.direct_scene_ids);
            for term in spec.terms {
                let trimmed = term.trim();
                if !trimmed.is_empty() {
                    term_set.insert(trimmed.to_string());
                }
            }
        }
        let terms: Vec<String> = term_set.into_iter().collect();
        Ok(scenes
            .into_iter()
            .filter(|scene| {
                if direct_scene_ids.contains(&scene.id) {
                    return true;
                }
                find_scene_reference_term_match(&scene.full_text, &terms).is_some()
            })
            .collect())
    }

    /// Derive `(display terms, direct-hit scene ids)` for one subject id.
    /// Used by `narrow_scenes_by_subjects` and `find_scenes_referencing`.
    /// Mirrors `services/mod.rs:6826` in 705b835^.
    pub(super) async fn scene_reference_subject_spec(
        &self,
        project_id: &str,
        branch_id: &str,
        subject_id: &str,
    ) -> Result<SubjectSceneReferenceSpec> {
        use std::collections::BTreeSet;
        let (table, _key) = subject_id
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("expected `table:id` format, got `{subject_id}`"))?;

        let mut spec = match table {
            "character" => {
                let character = self.repository.get_character(subject_id).await?;
                ensure_reference_subject_project(&character.project_id, project_id, "character")?;
                let mut direct_scene_ids = BTreeSet::new();
                for state in self
                    .repository
                    .list_character_states_by_project_and_branch(project_id, branch_id)
                    .await?
                {
                    if state.character_id == subject_id
                        && let Some(scene_id) = state.scene_id
                    {
                        direct_scene_ids.insert(scene_id);
                    }
                }
                for relationship in self
                    .repository
                    .list_relationships_by_branch(branch_id)
                    .await?
                {
                    if (relationship.in_id == subject_id || relationship.out_id == subject_id)
                        && let Some(scene_id) = relationship.last_scene_id
                    {
                        direct_scene_ids.insert(scene_id);
                    }
                }
                SubjectSceneReferenceSpec {
                    terms: vec![character.name],
                    direct_scene_ids,
                }
            }
            "location" => {
                let location = self.repository.get_location(subject_id).await?;
                ensure_reference_subject_project(&location.project_id, project_id, "location")?;
                SubjectSceneReferenceSpec {
                    terms: vec![location.name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "faction" => {
                let faction = self.repository.get_faction(subject_id).await?;
                ensure_reference_subject_project(&faction.project_id, project_id, "faction")?;
                SubjectSceneReferenceSpec {
                    terms: vec![faction.name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "religion" => {
                let religion = self.repository.get_religion(subject_id).await?;
                ensure_reference_subject_project(&religion.project_id, project_id, "religion")?;
                SubjectSceneReferenceSpec {
                    terms: vec![religion.name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "economy" => {
                let economy = self.repository.get_economy(subject_id).await?;
                ensure_reference_subject_project(&economy.project_id, project_id, "economy")?;
                SubjectSceneReferenceSpec {
                    terms: vec![economy.name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "term" => {
                let term = self.repository.get_term(subject_id).await?;
                ensure_reference_subject_project(&term.project_id, project_id, "term")?;
                SubjectSceneReferenceSpec {
                    terms: vec![term.term_text],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "plot_line" => {
                let plot_line = self
                    .repository
                    .list_plot_lines_by_project(project_id)
                    .await?
                    .into_iter()
                    .find(|pl| pl.id == subject_id)
                    .ok_or_else(|| anyhow::anyhow!("plot line not found"))?;
                SubjectSceneReferenceSpec {
                    terms: vec![plot_line.name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "conflict" => {
                let conflict = self.repository.get_conflict(subject_id).await?;
                ensure_reference_subject_project(&conflict.project_id, project_id, "conflict")?;
                let mut direct_scene_ids = BTreeSet::new();
                for annotation in self
                    .repository
                    .list_scene_beat_annotations_by_project(project_id)
                    .await?
                {
                    if annotation.conflict_ids.iter().any(|id| id == subject_id) {
                        direct_scene_ids.insert(annotation.scene_id);
                    }
                }
                SubjectSceneReferenceSpec {
                    terms: vec![conflict.name],
                    direct_scene_ids,
                }
            }
            "theme" => {
                let theme = self.repository.get_theme(subject_id).await?;
                ensure_reference_subject_project(&theme.project_id, project_id, "theme")?;
                let mut direct_scene_ids = BTreeSet::new();
                for annotation in self
                    .repository
                    .list_scene_beat_annotations_by_project(project_id)
                    .await?
                {
                    if annotation.theme_ids.iter().any(|id| id == subject_id) {
                        direct_scene_ids.insert(annotation.scene_id);
                    }
                }
                SubjectSceneReferenceSpec {
                    terms: vec![theme.theme_statement],
                    direct_scene_ids,
                }
            }
            "motif" => {
                let motif = self.repository.get_motif(subject_id).await?;
                ensure_reference_subject_project(&motif.project_id, project_id, "motif")?;
                let mut direct_scene_ids = BTreeSet::new();
                for annotation in self
                    .repository
                    .list_scene_beat_annotations_by_project(project_id)
                    .await?
                {
                    if annotation.motif_ids.iter().any(|id| id == subject_id) {
                        direct_scene_ids.insert(annotation.scene_id);
                    }
                }
                SubjectSceneReferenceSpec {
                    terms: vec![motif.name],
                    direct_scene_ids,
                }
            }
            "world_rule" => {
                let world_rule = self
                    .repository
                    .list_world_rules_by_project(project_id)
                    .await?
                    .into_iter()
                    .find(|wr| wr.id == subject_id)
                    .ok_or_else(|| anyhow::anyhow!("world rule not found"))?;
                SubjectSceneReferenceSpec {
                    terms: vec![world_rule.rule_name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "narrative_promise" => {
                let promise = self.repository.get_narrative_promise(subject_id).await?;
                ensure_reference_subject_project(
                    &promise.project_id,
                    project_id,
                    "narrative_promise",
                )?;
                SubjectSceneReferenceSpec {
                    terms: vec![promise.description],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "system_overlay" => {
                let overlay = self.repository.get_system_overlay(subject_id).await?;
                ensure_reference_subject_project(
                    &overlay.project_id,
                    project_id,
                    "system_overlay",
                )?;
                SubjectSceneReferenceSpec {
                    terms: vec![overlay.system_name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            other => {
                anyhow::bail!("find_scenes_referencing does not support subject table `{other}`");
            }
        };
        spec.terms.retain(|term| !term.trim().is_empty());
        if spec.terms.is_empty() && spec.direct_scene_ids.is_empty() {
            anyhow::bail!(
                "find_scenes_referencing could not derive any search terms for `{subject_id}`"
            );
        }
        Ok(spec)
    }

    /// Scan a scene's prose for hits against branch-scoped world rules.
    /// Mirrors `services/mod.rs:5737` in 705b835^.
    pub(super) async fn scan_world_rules(
        &self,
        project_id: &str,
        branch_id: &str,
        prose: &str,
    ) -> Result<Vec<spindle_core::models::WorldRuleHit>> {
        let rules = self
            .repository
            .list_world_rules_by_project_and_branch(project_id, branch_id)
            .await?;
        let scan_rules: Vec<spindle_core::world_rules::ScanRule> = rules
            .iter()
            .map(|r| spindle_core::world_rules::ScanRule {
                rule_id: r.id.clone(),
                scan_pattern: r.scan_pattern.clone(),
                rule_name: r.rule_name.clone(),
                description: r.description.clone(),
                rule_type: r.rule_type.clone(),
            })
            .collect();
        Ok(spindle_core::world_rules::scan_prose_for_world_rules(
            prose,
            &scan_rules,
        ))
    }

    /// Scan a scene's prose for forbidden-phrase voice drift against every
    /// character on the active branch (with per-project main fallback). Only
    /// characters whose name/aliases appear in the prose are checked. Mirrors
    /// `services/mod.rs:5758` in 705b835^.
    pub(super) async fn scan_voice_drift(
        &self,
        project_id: &str,
        prose: &str,
    ) -> Result<Vec<spindle_core::models::VoiceDriftFinding>> {
        use spindle_core::models::CharacterVoiceProfileData;
        use spindle_core::voice::{
            VoiceDriftCharacter, character_present_in_scene, check_voice_drift,
        };

        let active_branch = self.repository.get_active_branch(project_id).await?;
        let mut characters = self
            .repository
            .list_characters_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        characters.retain(|character| {
            character_present_in_scene(
                prose,
                &VoiceDriftCharacter {
                    id: character.id.clone(),
                    name: character.name.clone(),
                },
            )
        });
        if characters.is_empty() {
            return Ok(Vec::new());
        }

        let mut findings = Vec::new();
        for character in characters {
            let voice_profile = self
                .repository
                .get_character_voice_profile(&character.id)
                .await?;
            let profile_data = CharacterVoiceProfileData {
                tone: voice_profile.tone.clone(),
                vocabulary: voice_profile.vocabulary.clone(),
                sentence_structure: voice_profile.sentence_structure.clone(),
                tics: voice_profile.tics.clone(),
                forbidden_words: voice_profile.forbidden_words.clone(),
                example_lines: voice_profile.example_lines.clone(),
                established_in_scene_id: voice_profile.established_in_scene_id.clone(),
                updated_at: voice_profile.updated_at.as_ref().map(ToString::to_string),
            };
            let drift_character = VoiceDriftCharacter {
                id: character.id.clone(),
                name: character.name.clone(),
            };
            findings.extend(check_voice_drift(prose, &drift_character, &profile_data));
        }
        Ok(findings)
    }

    /// Scan a scene's prose for retcon-reachability problems on the active
    /// branch (with per-project main fallback). Mirrors `check_reachability`
    /// (`services/mod.rs:5800` area) in 705b835^.
    /// Single POV character for a scene: an explicit chapter-plan POV wins;
    /// otherwise only a project with exactly one protagonist role qualifies.
    pub(super) async fn pov_character_id_for_scene(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<Option<String>> {
        let plans = self
            .repository
            .list_chapter_plans_by_project_and_branch(project_id, branch_id)
            .await?;
        if let Some(pov) = plans
            .iter()
            .find(|plan| plan.book_number == book_number && plan.chapter_number == chapter_number)
            .and_then(|plan| plan.pov_character_id.clone())
        {
            return Ok(Some(pov));
        }
        let characters = self
            .repository
            .list_characters_by_project_and_branch(project_id, branch_id)
            .await?;
        let protagonists: Vec<String> = characters
            .into_iter()
            .filter(|character| character.role.eq_ignore_ascii_case("protagonist"))
            .map(|character| character.id)
            .collect();
        Ok((protagonists.len() == 1).then(|| protagonists[0].clone()))
    }

    pub(super) async fn scan_retcon_findings(
        &self,
        project_id: &str,
        scene: &crate::sqlite::records::Scene,
    ) -> Result<Vec<spindle_core::models::RetconFinding>> {
        self.scan_retcon_findings_with_pending(project_id, scene, &[])
            .await
    }

    /// Scan a scene for retcon findings, including character states declared
    /// by the commit that is about to write this scene.
    ///
    /// `pending_states` carries only statuses that replace a prior status with
    /// a living one. They are treated as the newest state at this scene's
    /// position, so a respawn committed in the same write supersedes the death
    /// that the scan would otherwise find in the persisted state history.
    pub(super) async fn scan_retcon_findings_with_pending(
        &self,
        project_id: &str,
        scene: &crate::sqlite::records::Scene,
        pending_states: &[PendingCharacterStatus],
    ) -> Result<Vec<spindle_core::models::RetconFinding>> {
        use spindle_core::models::RetconFinding;
        use std::collections::BTreeMap;

        let branch_id = &scene.branch_id;
        let pov_character_id = self
            .pov_character_id_for_scene(
                project_id,
                branch_id,
                scene.book_number,
                scene.chapter_number,
            )
            .await?;
        let future_knowledge = self
            .repository
            .list_future_knowledge_by_project_and_branch(project_id, branch_id)
            .await?;
        let timeline_events = self
            .repository
            .list_timeline_events_by_project_and_branch(project_id, branch_id)
            .await?;
        let temporal_interventions = self
            .repository
            .list_temporal_interventions_by_project_and_branch(project_id, branch_id)
            .await?;

        let scene_position = (scene.book_number, scene.chapter_number, scene.scene_order);
        let mut findings = Vec::new();
        for knowledge in &future_knowledge {
            let learned_at = (
                knowledge.learned_at.book_number,
                knowledge.learned_at.chapter_number,
                knowledge.learned_at.scene_order.unwrap_or(0),
            );
            if !position_gt(learned_at, scene_position) {
                continue;
            }
            if !contains_case_insensitive_phrase(&scene.full_text, &knowledge.knowledge_summary)
                && !contains_case_insensitive_phrase(&scene.summary, &knowledge.knowledge_summary)
            {
                continue;
            }
            findings.push(RetconFinding::OutOfBandsKnowledge {
                character_id: knowledge.character_id.clone(),
                knowledge_summary: knowledge.knowledge_summary.clone(),
                learned_at: knowledge.learned_at.clone().into_core(),
                message: format!(
                    "scene references future knowledge '{}' before learned_at {}.{}.{}",
                    knowledge.knowledge_summary, learned_at.0, learned_at.1, learned_at.2
                ),
            });
        }

        let events_by_id = timeline_events
            .iter()
            .map(|event| (event.id.clone(), event))
            .collect::<BTreeMap<_, _>>();
        for intervention in &temporal_interventions {
            let anchor_position = intervention
                .target_event_id
                .as_ref()
                .and_then(|id| events_by_id.get(id))
                .or_else(|| {
                    intervention
                        .source_event_id
                        .as_ref()
                        .and_then(|id| events_by_id.get(id))
                })
                .map(|event| {
                    (
                        event.placement.book_number,
                        event.placement.chapter_number,
                        event.placement.scene_order.unwrap_or(0),
                    )
                });
            let Some(anchor_position) = anchor_position else {
                continue;
            };
            if !position_gt(anchor_position, scene_position) {
                continue;
            }

            let mut anchor_terms: Vec<String> = Vec::new();
            if !intervention.title.trim().is_empty() {
                anchor_terms.push(intervention.title.clone());
            }
            if let Some(source) = intervention
                .source_event_id
                .as_ref()
                .and_then(|id| events_by_id.get(id))
                && !source.title.trim().is_empty()
            {
                anchor_terms.push(source.title.clone());
            }
            if let Some(target) = intervention
                .target_event_id
                .as_ref()
                .and_then(|id| events_by_id.get(id))
                && !target.title.trim().is_empty()
            {
                anchor_terms.push(target.title.clone());
            }
            if anchor_terms.is_empty() {
                continue;
            }

            let mentions_intervention = anchor_terms.iter().any(|term| {
                contains_case_insensitive_phrase(&scene.full_text, term)
                    || contains_case_insensitive_phrase(&scene.summary, term)
            });
            if !mentions_intervention {
                continue;
            }

            let has_future_knowledge_anchor = future_knowledge.iter().any(|knowledge| {
                let learned_at = (
                    knowledge.learned_at.book_number,
                    knowledge.learned_at.chapter_number,
                    knowledge.learned_at.scene_order.unwrap_or(0),
                );
                if position_gt(learned_at, scene_position) {
                    return false;
                }
                if let Some(expires_at) = knowledge.expires_at.as_ref() {
                    let expires_position = (
                        expires_at.book_number,
                        expires_at.chapter_number,
                        expires_at.scene_order.unwrap_or(0),
                    );
                    if position_lt(expires_position, scene_position) {
                        return false;
                    }
                }
                anchor_terms.iter().any(|term| {
                    contains_case_insensitive_phrase(&knowledge.knowledge_summary, term)
                        || contains_case_insensitive_phrase(&knowledge.source, term)
                })
            });
            if !has_future_knowledge_anchor {
                findings.push(RetconFinding::MissingFutureKnowledgeAnchor {
                    intervention_id: intervention.id.clone(),
                    intervention_title: intervention.title.clone(),
                    message: format!(
                        "scene references intervention '{}' before timeline anchor {}.{}.{} without a future_knowledge anchor",
                        intervention.title, anchor_position.0, anchor_position.1, anchor_position.2
                    ),
                });
            }
        }

        let characters = self
            .repository
            .list_characters_by_project_and_branch(project_id, branch_id)
            .await?;
        let states = self
            .repository
            .list_character_states_by_project_and_branch(project_id, branch_id)
            .await?;
        let life_facts = self
            .repository
            .list_active_canonical_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let world_rules = self
            .repository
            .list_world_rules_by_project_and_branch(project_id, branch_id)
            .await?;
        let world_declares_respawn = world_rules.iter().any(world_rule_declares_respawn);
        let mention_records: Vec<spindle_core::character_mentions::CharacterMentionRecord> =
            characters
                .iter()
                .map(
                    |character| spindle_core::character_mentions::CharacterMentionRecord {
                        id: character.id.clone(),
                        name: character.name.clone(),
                        aliases: character.aliases.clone(),
                    },
                )
                .collect();
        let mention_resolver =
            spindle_core::character_mentions::CharacterMentionResolver::new(&mention_records);
        let characters_by_id = characters
            .into_iter()
            .map(|character| (character.id.clone(), character))
            .collect::<BTreeMap<_, _>>();

        let clock = self.repository.get_scene_clock(&scene.id).await?;
        let is_flashback = clock
            .as_ref()
            .and_then(|clock| clock.temporal_mode.as_deref())
            .is_some_and(|mode| mode.eq_ignore_ascii_case("flashback"));
        for (character_id, character) in &characters_by_id {
            if is_flashback {
                continue;
            }
            let has_action = has_named_character_action(
                &scene.full_text,
                mention_resolver.aliases_for(character_id),
            ) || has_named_character_alive_assertion(
                &scene.full_text,
                mention_resolver.aliases_for(character_id),
            ) || (pov_character_id.as_deref() == Some(character_id.as_str())
                && has_named_character_action(
                    &spindle_core::character_mentions::unquoted_narration(&scene.full_text),
                    &["I".to_string()],
                ));
            let latest_state = crate::sqlite::repository::select_latest_character_state(
                states.iter(),
                character_id,
                scene_position,
            );
            // A pending living status is the newest state at this scene's
            // position: the commit is about to append it, so it supersedes any
            // persisted dead state even before the row exists.
            let pending_status = pending_states.iter().find(|pending| {
                pending.character_id == *character_id
                    && !pending.status.is_empty()
                    && !pending.status.iter().any(|status| is_dead_status(status))
            });
            let state_position = if pending_status.is_some() {
                Some(scene_position)
            } else {
                latest_state
                    .map(|state| (state.book_number, state.chapter_number, state.scene_order))
            };
            let current_scene_state = state_position == Some(scene_position);
            // A same-scene state proves cast presence, not that a canonical
            // death happened in this scene; only a same-scene dead status is
            // ambiguous and withheld.
            let state_death = if current_scene_state {
                None
            } else {
                latest_state.and_then(|state| {
                    state
                        .status
                        .iter()
                        .find(|status| is_dead_status(status))
                        .cloned()
                })
            };
            // Canonical life_status facts are a second, independent death
            // store. Only facts strictly before this scene count; a later
            // explicit non-dead character state clears an older death.
            let life_fact = life_facts
                .iter()
                .filter(|fact| fact.subject_id.as_deref() == Some(character_id.as_str()))
                .filter(|fact| {
                    matches!(
                        fact.predicate.trim().to_ascii_lowercase().as_str(),
                        "life_status" | "status" | "vital_status" | "vital" | "state" | "condition"
                    )
                })
                .filter(|fact| fact.scene_id.as_deref() != Some(scene.id.as_str()))
                .filter_map(|fact| {
                    let fact_position = (fact.book_number, fact.chapter_number);
                    let scene_chapter = (scene.book_number, scene.chapter_number);
                    // Only an explicitly marked pre-story/backstory fact
                    // applies to the opening scene. A scene-less chapter-1
                    // fact with an ordinary chapter/scene position (or no
                    // valid_from) still takes effect at that position, as it
                    // did before story-start anchors were introduced.
                    let story_start_fact = fact.scene_id.is_none()
                        && fact.valid_from.as_ref().is_some_and(|placement| {
                            placement.book_number == 1
                                && placement.chapter_number == 1
                                && placement.scene_order == Some(0)
                        });
                    if fact_position >= scene_chapter && !story_start_fact {
                        return None;
                    }
                    Some((
                        (fact.book_number, fact.chapter_number, 0),
                        fact.created_at,
                        fact.id.clone(),
                        crate::format::canonical_fact_value(fact),
                    ))
                })
                .max_by_key(|(position, created_at, id, _)| (*position, *created_at, id.clone()));
            let life_death = life_fact.and_then(|(position, _, _, value)| {
                is_dead_status(&value).then_some((position, value))
            });
            // Canonical death + current-scene cast state is presence after
            // death even when the prose uses a pronoun or an unlisted verb.
            // Otherwise require a named/alive action to avoid mere mentions.
            if !has_action && !(current_scene_state && life_death.is_some()) {
                continue;
            }
            let dead_status = match (state_death, life_death) {
                (Some(status), Some((fact_position, fact_status))) => {
                    if state_position.is_some_and(|position| position >= fact_position) {
                        status
                    } else {
                        fact_status
                    }
                }
                (Some(status), None) => status,
                (None, Some((fact_position, fact_status))) => {
                    let later_living_state = if pending_status.is_some() {
                        scene_position >= fact_position
                    } else {
                        latest_state.is_some_and(|state| {
                            state_position.is_some_and(|position| position >= fact_position)
                                && !state.status.is_empty()
                                && !state.status.iter().any(|status| is_dead_status(status))
                        })
                    };
                    if later_living_state {
                        continue;
                    }
                    fact_status
                }
                (None, None) => continue,
            };
            // A declared respawn cycle is the author's explicit statement that
            // this character returns after death. Without one, a recorded
            // death followed by action still reports.
            if world_declares_respawn || character_declares_respawn(character_id, &life_facts) {
                continue;
            }
            findings.push(RetconFinding::DeadCharacterAct {
                character_id: character_id.clone(),
                character_name: character.name.clone(),
                status: dead_status.clone(),
                message: format!(
                    "book {} chapter {} scene {} includes '{}' acting after status '{}'",
                    scene.book_number,
                    scene.chapter_number,
                    scene.scene_order,
                    character.name,
                    dead_status
                ),
            });
        }

        // Premature knowledge: a present character references, in prose, a
        // knowledge_fact they do not learn until a later story position. The
        // normalized_fact match is whole-phrase, so this is a high-precision,
        // low-recall advisory tripwire (mapped to a warning, not a hard block).
        let knowledge_facts = self
            .repository
            .list_knowledge_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        for fact in &knowledge_facts {
            let Some(learned_at) = fact.learned_at.as_ref() else {
                continue;
            };
            let learned_position = (
                learned_at.book_number,
                learned_at.chapter_number,
                learned_at.scene_order.unwrap_or(0),
            );
            if !position_gt(learned_position, scene_position) {
                continue;
            }
            let Some(character) = characters_by_id.get(&fact.character_id) else {
                continue;
            };
            if !mention_resolver.mentions_with_pov(
                &scene.full_text,
                &character.id,
                pov_character_id.as_deref(),
            ) {
                continue;
            }
            let exact_hit =
                contains_case_insensitive_phrase(&scene.full_text, &fact.normalized_fact);
            let paraphrase = if exact_hit {
                None
            } else {
                let scene_input = SceneValidationInputV1 {
                    scene_id: &scene.id,
                    book_number: scene.book_number,
                    chapter_number: scene.chapter_number,
                    scene_order: scene.scene_order,
                    text: &scene.full_text,
                };
                let core_fact = KnowledgeValidationFactV1 {
                    fact_id: fact.id.clone(),
                    character_id: fact.character_id.clone(),
                    character_names: vec![character.name.clone()],
                    character_aliases: mention_resolver.aliases_for(&character.id).to_vec(),
                    first_person_subject: pov_character_id.as_deref()
                        == Some(character.id.as_str()),
                    fact: fact.fact.clone(),
                    learned_book_number: learned_at.book_number,
                    learned_chapter_number: learned_at.chapter_number,
                    learned_scene_order: learned_at.scene_order,
                };
                detect_knowledge_paraphrase_leak(&scene_input, &core_fact)
            };
            if !exact_hit && paraphrase.is_none() {
                continue;
            }
            findings.push(RetconFinding::PrematureKnowledge {
                character_id: fact.character_id.clone(),
                fact: fact.fact.clone(),
                learned_at: learned_at.clone().into_core(),
                message: paraphrase.map(|finding| finding.message).unwrap_or_else(|| {
                    format!(
                        "scene has '{}' reference knowledge they do not learn until book {} chapter {}",
                        character.name, learned_at.book_number, learned_at.chapter_number
                    )
                }),
            });
        }

        Ok(findings)
    }

    /// Assemble a `ValidatorContext` that the Phase-4 validator registry can
    /// evaluate scenes against. Pulls active canonical facts, world rules,
    /// character voice profiles, timeline events, and temporal interventions
    /// for the project. Mirrors `services/mod.rs:6089` in 705b835^.
    pub(crate) async fn build_phase_four_validator_context(
        &self,
        project_id: &str,
        branch_id: &str,
        scenes: &[crate::sqlite::records::Scene],
    ) -> Result<spindle_core::validators::ValidatorContext> {
        use spindle_core::validators::{
            CanonicalFactSnapshot, CharacterVoiceProfileSnapshot, SceneSnapshot,
            TemporalInterventionSnapshot, TimelineEventSnapshot, ValidatorContext,
            WorldRuleSnapshot,
        };

        let canonical_facts_raw = self
            .repository
            .list_active_canonical_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let world_rules_raw = self
            .repository
            .list_world_rules_by_project_and_branch(project_id, branch_id)
            .await?;
        let characters = self
            .repository
            .list_characters_by_project_and_branch(project_id, branch_id)
            .await?;
        let chapter_plans = self
            .repository
            .list_chapter_plans_by_project_and_branch(project_id, branch_id)
            .await?;
        let single_protagonist = {
            let protagonists: Vec<&str> = characters
                .iter()
                .filter(|character| character.role.eq_ignore_ascii_case("protagonist"))
                .map(|character| character.id.as_str())
                .collect();
            (protagonists.len() == 1).then(|| protagonists[0].to_string())
        };
        let pov_by_chapter: std::collections::BTreeMap<(i32, i32), String> = chapter_plans
            .iter()
            .filter_map(|plan| {
                plan.pov_character_id
                    .clone()
                    .map(|pov| ((plan.book_number, plan.chapter_number), pov))
            })
            .collect();
        let timeline_events_raw = self
            .repository
            .list_timeline_events_by_project_and_branch(project_id, branch_id)
            .await?;
        let temporal_interventions_raw = self
            .repository
            .list_temporal_interventions_by_project_and_branch(project_id, branch_id)
            .await?;

        let mention_records: Vec<spindle_core::character_mentions::CharacterMentionRecord> =
            characters
                .iter()
                .map(
                    |character| spindle_core::character_mentions::CharacterMentionRecord {
                        id: character.id.clone(),
                        name: character.name.clone(),
                        aliases: character.aliases.clone(),
                    },
                )
                .collect();
        let mention_resolver =
            spindle_core::character_mentions::CharacterMentionResolver::new(&mention_records);
        let subject_names_by_id: std::collections::BTreeMap<String, Vec<String>> = characters
            .iter()
            .map(|character| {
                let mut names = mention_resolver.aliases_for(&character.id).to_vec();
                if names.is_empty() {
                    names.push(character.name.clone());
                }
                (character.id.clone(), names)
            })
            .collect();
        let canonical_facts = canonical_facts_raw
            .into_iter()
            .map(|fact| {
                let value = canonical_fact_value_for_check(&fact);
                let subject_names = fact
                    .subject_id
                    .as_ref()
                    .and_then(|subject_id| subject_names_by_id.get(subject_id))
                    .cloned()
                    .unwrap_or_default();
                CanonicalFactSnapshot {
                    fact_id: fact.id.clone(),
                    scene_id: fact.scene_id.clone(),
                    book_number: fact.book_number,
                    chapter_number: fact.chapter_number,
                    fact_type: fact.value_kind.clone(),
                    key: fact.predicate.clone(),
                    value,
                    subject_id: fact.subject_id.clone(),
                    subject_names,
                }
            })
            .collect::<Vec<_>>();

        let world_rules = world_rules_raw
            .into_iter()
            .map(|rule| WorldRuleSnapshot {
                rule_id: rule.id.clone(),
                rule_name: rule.rule_name.clone(),
                scan_pattern: rule.scan_pattern.clone(),
                established_in: rule
                    .established_in
                    .as_ref()
                    .map(|placement| (placement.book_number, placement.chapter_number)),
                rule_type: rule.rule_type.clone(),
                description: rule.description.clone(),
            })
            .collect::<Vec<_>>();

        let mut voice_profiles = Vec::new();
        for character in characters {
            let profile = self
                .repository
                .get_character_voice_profile(&character.id)
                .await?;
            voice_profiles.push(CharacterVoiceProfileSnapshot {
                character_id: character.id.clone(),
                character_name: character.name.clone(),
                forbidden_words: profile.forbidden_words.clone(),
            });
        }

        let timeline_events = timeline_events_raw
            .into_iter()
            .map(|event| TimelineEventSnapshot {
                event_id: event.id.clone(),
                title: event.title.clone(),
                book_number: event.placement.book_number,
                chapter_number: event.placement.chapter_number,
                scene_order: event.placement.scene_order.unwrap_or(0),
            })
            .collect::<Vec<_>>();

        let temporal_interventions = temporal_interventions_raw
            .into_iter()
            .map(|intervention| TemporalInterventionSnapshot {
                intervention_id: intervention.id.clone(),
                title: intervention.title.clone(),
                source_event_id: intervention.source_event_id.clone(),
                target_event_id: intervention.target_event_id.clone(),
            })
            .collect::<Vec<_>>();

        let scene_snapshots = scenes
            .iter()
            .map(|scene| SceneSnapshot {
                scene_id: scene.id.clone(),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: scene.scene_order,
                full_text: scene.full_text.clone(),
                summary: scene.summary.clone(),
                pov_character_id: pov_by_chapter
                    .get(&(scene.book_number, scene.chapter_number))
                    .cloned()
                    .or_else(|| single_protagonist.clone()),
            })
            .collect::<Vec<_>>();

        let style_directive = self
            .style_directive_for(project_id, branch_id)
            .await
            .ok()
            .filter(|directive| !directive.is_empty());

        Ok(ValidatorContext {
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            scenes: scene_snapshots,
            canonical_facts,
            world_rules,
            voice_profiles,
            timeline_events,
            temporal_interventions,
            style_directive,
        })
    }

    /// Run the Phase-4 validator registry against `scenes` and persist
    /// per-scene cache rows into `validator_finding`. Returns the
    /// `ConsistencyIssue` list filtered to `phase_four_checks`. Mirrors
    /// `services/mod.rs:6233` in 705b835^.
    pub(super) async fn run_phase_four_validator_checks_for_scenes(
        &self,
        project_id: &str,
        branch_id: &str,
        scenes: &[crate::sqlite::records::Scene],
        phase_four_checks: &std::collections::BTreeSet<PhaseFourCacheId>,
        use_cache: bool,
    ) -> Result<Vec<spindle_core::models::ConsistencyIssue>> {
        use spindle_core::models::ConsistencyIssue;

        if phase_four_checks.is_empty() || scenes.is_empty() {
            return Ok(Vec::new());
        }

        let context = self
            .build_phase_four_validator_context(project_id, branch_id, scenes)
            .await?;
        let registry = crate::sqlite::validators::phase_four_validator_registry();
        let validator_ids = phase_four_validator_ids(phase_four_checks);
        let context_hashes = phase_four_context_hashes(&context, phase_four_checks)?;
        let mut issues = Vec::new();

        for scene in &context.scenes {
            let scene_text_hash = generation_sha256_hex(scene.full_text.as_bytes());
            if use_cache {
                let cached_rows = self
                    .repository
                    .list_active_validator_findings_by_scene_hash(
                        branch_id,
                        &scene.scene_id,
                        &scene_text_hash,
                        &validator_ids,
                    )
                    .await?;
                let cached_rows = cached_rows
                    .into_iter()
                    .filter(|row| {
                        row.context_hash
                            .as_ref()
                            .and_then(|hash| {
                                context_hashes
                                    .get(row.validator_id.as_str())
                                    .map(|expected| expected == hash)
                            })
                            .unwrap_or(false)
                    })
                    .collect::<Vec<_>>();
                if has_cache_for_all_validators(&cached_rows, &validator_ids) {
                    issues.extend(cached_validator_issues(&cached_rows, &scene.scene_id));
                    continue;
                }
            }

            let findings = registry
                .validate_scene(scene, &context)
                .map_err(|error| anyhow::anyhow!(error))?;

            for validator_id in &validator_ids {
                let validator_findings = findings
                    .iter()
                    .filter(|finding| finding.check_type == validator_id)
                    .collect::<Vec<_>>();

                let serialized_issues = validator_findings
                    .iter()
                    .map(|finding| {
                        serde_json::json!({
                            "severity": finding.severity.as_str(),
                            "check_type": finding.check_type,
                            "message": finding.message,
                            "byte_range": finding.byte_range,
                            "entity_ids": finding.entity_ids,
                        })
                    })
                    .collect::<Vec<_>>();

                let summary_severity = validator_findings
                    .iter()
                    .map(|finding| finding.severity.as_str())
                    .min()
                    .unwrap_or("info")
                    .to_string();
                let summary_message = if validator_findings.is_empty() {
                    "no findings".to_string()
                } else {
                    format!("{} finding(s)", validator_findings.len())
                };

                self.repository
                    .upsert_validator_finding(
                        crate::sqlite::repository::UpsertValidatorFindingParams {
                            project_id: project_id.to_string(),
                            branch_id: branch_id.to_string(),
                            scene_id: scene.scene_id.clone(),
                            scene_text_hash: scene_text_hash.clone(),
                            context_hash: context_hashes.get(validator_id.as_str()).cloned(),
                            validator_id: validator_id.clone(),
                            finding_id: "__cache__".to_string(),
                            severity: summary_severity,
                            message: summary_message,
                            byte_range: None,
                            details_json: Some(serde_json::json!({ "issues": serialized_issues })),
                        },
                    )
                    .await?;
            }

            for finding in findings {
                if !phase_four_checks
                    .iter()
                    .any(|check| check.as_str() == finding.check_type)
                {
                    continue;
                }
                let mut message = finding.message;
                if let Some(byte_range) = finding.byte_range {
                    message = format!("{message} (bytes {}..{})", byte_range.start, byte_range.end);
                }
                let mut entity_ids = finding.entity_ids;
                if !entity_ids.iter().any(|id| id == &scene.scene_id) {
                    entity_ids.push(scene.scene_id.clone());
                }
                issues.push(ConsistencyIssue {
                    severity: finding.severity.as_str().to_string(),
                    check_type: finding.check_type.to_string(),
                    message,
                    entity_ids,
                    suggested_action: Some(
                        "revise scene prose or supporting canon records".to_string(),
                    ),
                    evidence: None,
                });
            }
        }

        Ok(issues)
    }

    /// Model-router driven semantic world-rule audit. Falls back to the
    /// in-process heuristic when the configured route does not produce a
    /// usable response. Mirrors `services/mod.rs:6344` in 705b835^.
    pub(super) async fn deep_world_rule_compliance_issues(
        &self,
        scenes: &[crate::sqlite::records::Scene],
        rules: &[crate::sqlite::records::WorldRule],
        coverage: &mut spindle_core::models::AuditCoverage,
    ) -> Result<Vec<spindle_core::models::ConsistencyIssue>> {
        use crate::format::world_rule_established_before_scene;
        use spindle_core::models::ConsistencyIssue;

        let mut issues = Vec::new();
        // Emission-cadence asymmetry, by design (mirrors the temporal tier):
        //   - a rating-clearance rejection (RatingNotCovered) is PER-SCENE and
        //     stops the run — ratings can differ per scene, so each uncleared
        //     scene emits its own honest skip; landed behavior, unchanged.
        //   - a generic TRANSPORT error is ONCE-PER-RUN: on the first transport
        //     failure we emit ONE degradation finding and set `model_route_dead`,
        //     then STOP attempting model calls for remaining scenes. The heuristic
        //     fallback STAYS — it is real coverage; the finding just makes the
        //     degradation visible instead of silent. (Previously this arm silently
        //     fell back to the heuristic, so a dead route degraded invisibly.)
        let mut model_route_dead = false;
        // Scenes that actually have rules to audit, fanned out with bounded
        // concurrency and served from the V0039 cache where this prose version
        // was already analyzed. Results arrive in scene order, so the skip
        // cadence below is unchanged; the stream is consumed lazily so the
        // dead-route exit still stops the calls queued behind it.
        let scanned: Vec<&crate::sqlite::records::Scene> = scenes
            .iter()
            .filter(|scene| {
                rules
                    .iter()
                    .any(|rule| world_rule_established_before_scene(rule, scene))
            })
            .collect();
        *coverage = spindle_core::models::AuditCoverage::new(
            "world_rule_compliance",
            scanned.iter().map(|s| s.id.clone()),
        );
        let rules_for_scene = |scene: &crate::sqlite::records::Scene| {
            rules
                .iter()
                .filter(|rule| world_rule_established_before_scene(rule, scene))
                .collect::<Vec<_>>()
        };
        let prompts = scanned
            .iter()
            .map(|scene| build_world_rule_deep_check_prompt(scene, &rules_for_scene(scene)))
            .collect();
        let mut outputs = std::pin::pin!(self.deep_tier_output_stream(
            &scanned,
            "world_rule_compliance",
            prompts
        ));

        let mut index = 0usize;
        while index < scanned.len() {
            // Once the route is known dead this run, STOP POLLING the stream:
            // a dead route won't heal mid-run, and leaving it unpolled is what
            // keeps the remaining scenes from dispatching a model call each.
            // The rest of the range still gets heuristic coverage below.
            let output = if model_route_dead {
                None
            } else {
                futures_util::StreamExt::next(&mut outputs).await
            };
            let scene = scanned[index];
            index += 1;
            let applicable_rules = rules_for_scene(scene);

            let model_violations = match output {
                None => heuristic_world_rule_violations(scene, &applicable_rules),
                Some(output) => match output {
                    Ok(response) if response.truncated => {
                        coverage.mark_inconclusive(
                            scene.id.clone(),
                            "deep check output was truncated at max_tokens",
                        );
                        heuristic_world_rule_violations(scene, &applicable_rules)
                    }
                    Ok(response) if !response.local_stub => {
                        match parse_deep_world_rule_check_output(&response.output) {
                            Ok(findings) => {
                                coverage.evaluated_ids.push(scene.id.clone());
                                findings
                            }
                            Err(_) => heuristic_world_rule_violations(scene, &applicable_rules),
                        }
                    }
                    Ok(_) => heuristic_world_rule_violations(scene, &applicable_rules),
                    // Honest skip on a rating-clearance rejection: the `review`
                    // agent is not cleared for this scene's rating, so the prose
                    // never left. Emit one info finding that reads as SKIPPED
                    // (route + rating, no prose) and stop — every remaining scene
                    // of this rating would hit the same wall. PER-SCENE cadence.
                    Err(err)
                        if matches!(
                            err.downcast_ref::<crate::ai::RouteClearanceError>(),
                            Some(crate::ai::RouteClearanceError::RatingNotCovered { .. })
                        ) =>
                    {
                        issues.push(world_rule_deep_skip_issue(&err));
                        return Ok(issues);
                    }
                    // A wall-clock timeout is INCONCLUSIVE for this scene, not a
                    // dead route: keep auditing the remaining scenes and keep the
                    // deterministic heuristic fallback for this one.
                    Err(err) if is_deep_check_timeout(&err) => {
                        coverage.mark_inconclusive(
                            scene.id.clone(),
                            "deep check timed out before producing a verdict",
                        );
                        heuristic_world_rule_violations(scene, &applicable_rules)
                    }
                    // Generic transport error (dead/unreachable route): emit ONE
                    // degradation finding, mark the route dead so no further model
                    // calls are attempted this run, and fall through to the
                    // heuristic for THIS scene (and every remaining scene). The
                    // heuristic coverage is preserved; the finding makes the
                    // silent degradation visible.
                    Err(err) => {
                        issues.push(world_rule_deep_skip_issue(&err));
                        model_route_dead = true;
                        heuristic_world_rule_violations(scene, &applicable_rules)
                    }
                },
            };

            if !coverage.evaluated_ids.contains(&scene.id) {
                coverage.heuristic_ids.push(scene.id.clone());
            }
            for violation in model_violations {
                let Some(rule) = applicable_rules
                    .iter()
                    .find(|rule| rule.id == violation.rule_id)
                else {
                    continue;
                };
                let mut message = format!(
                    "scene {} may violate world rule '{}': {}",
                    scene.id,
                    rule.rule_name,
                    violation.message.trim()
                );
                if let Some(evidence) = violation.evidence.as_ref()
                    && !evidence.trim().is_empty()
                {
                    message.push_str(&format!(" Evidence: {}", evidence.trim()));
                }
                issues.push(ConsistencyIssue {
                    severity: violation.severity.unwrap_or_else(|| "warning".to_string()),
                    check_type: "world_rule_compliance".to_string(),
                    message,
                    entity_ids: vec![scene.id.clone(), rule.id.clone()],
                    suggested_action: Some(
                        "revise the scene, explain the exception on page, or update the rule if canon changed"
                            .to_string(),
                    ),
                    evidence: None,
                });
            }
        }

        Ok(issues)
    }

    /// Tier 2 of the temporal-coherence subsystem: a model-router-driven
    /// semantic pass over each scene's prose that catches the in-world time
    /// jumps the deterministic lexicon scan (Tier 1) cannot — idiomatic elapsed
    /// time ("three cigarettes later"), implied light/meal/errand cues, and
    /// drift the fixed vocabulary misses. Advisory `warning`s under the same
    /// `temporal_coherence` check_type. When no review model is configured the
    /// route returns no parseable findings, so this adds nothing and the
    /// deterministic scan stands alone. Mirrors `deep_world_rule_compliance_issues`.
    /// One deep-tier model call for one scene, served from the V0039 cache when
    /// this exact prose version has already been analyzed by this tier.
    ///
    /// A deep check over a chapter range is `tiers x scenes` model calls; on a
    /// reasoning model that runs past any client timeout, and the audit used to
    /// hold everything in memory until the very end, so a timeout threw away
    /// every completed scene and the retry re-paid for all of it. Banking the
    /// raw output per (scene, prose version, tier) turns the retry into forward
    /// progress: analyzed scenes re-parse for free and only the remainder calls
    /// the model.
    ///
    /// Local-stub output is NOT cached — it is free to recompute, and caching
    /// it would freeze a stub answer in front of a real model configured later.
    pub(super) async fn deep_tier_model_output(
        &self,
        scene: &crate::sqlite::records::Scene,
        check_type: &str,
        prompt: String,
    ) -> Result<DeepTierOutput> {
        use crate::ai::ModelRequest;

        let fingerprint = scene_revision_fingerprint(scene);
        if let Some(cached) = self
            .repository
            .get_deep_check_cache(&scene.id, &fingerprint, check_type)
            .await?
        {
            // Only external output is ever cached, so a hit is never a stub.
            return Ok(DeepTierOutput {
                output: cached,
                local_stub: false,
                truncated: false,
            });
        }

        // This pass carries scene prose, so it MUST stamp the scene's content
        // rating (lowercased) — the rating-gated chokepoint then routes an
        // explicit-rated scene only to an explicit-cleared agent (evolution §4).
        let response = self
            .repository
            .model_router()
            .complete_deep(&ModelRequest {
                route: "review".to_string(),
                prompt,
                rating: Some(scene.content_rating.trim().to_ascii_lowercase()),
                context: Some(crate::ai::RequestContext {
                    project_id: Some(scene.project_id.clone()),
                    scene_id: Some(scene.id.clone()),
                    ..Default::default()
                }),
            })
            .await?;

        let local_stub = response.adapter_kind == "local";
        // A truncated response is not a verdict: never cache it, and let the
        // caller mark the unit INCONCLUSIVE instead of parsing partial JSON.
        if !local_stub && !response.truncated {
            // A cache write must never fail the audit: the analysis in hand is
            // still valid, we just lose the chance to reuse it.
            if let Err(error) = self
                .repository
                .put_deep_check_cache(&scene.id, &fingerprint, check_type, &response.output)
                .await
            {
                tracing::warn!(
                    scene_id = %scene.id,
                    check_type,
                    error = %format!("{error:#}"),
                    "failed to cache deep-check output; a retry will re-call the model"
                );
            }
        }
        Ok(DeepTierOutput {
            output: response.output,
            local_stub,
            truncated: response.truncated,
        })
    }

    /// Stream one deep tier's per-scene model calls with bounded concurrency,
    /// yielding results in scene order.
    ///
    /// The per-scene calls are independent, so running them in sequence only
    /// summed their latencies — the difference between a chapter range that
    /// finishes and one that never does. The cap keeps a wide range from opening
    /// one connection per scene against the provider.
    ///
    /// Returned as a lazy stream, not a collected Vec, so a caller that stops on
    /// the first failure (both per-scene tiers do — a dead route will not heal
    /// mid-run) drops the stream and cancels the calls still queued behind it.
    pub(super) fn deep_tier_output_stream<'a>(
        &'a self,
        scenes: &'a [&'a crate::sqlite::records::Scene],
        check_type: &'a str,
        prompts: Vec<String>,
    ) -> impl futures_util::Stream<Item = Result<DeepTierOutput>> + 'a {
        use futures_util::stream::StreamExt;

        // The FIRST scene is deliberately probed alone, and only then does the
        // rest fan out. Both tiers promise that a dead or uncleared route costs
        // exactly ONE attempt before they stop; fanning out from the start would
        // put `DEEP_CHECK_CONCURRENCY` calls on the wire before the first
        // failure was even observed, hammering a route already known broken.
        // Probing first keeps that promise exact, while a healthy route -- the
        // case that actually needs the speed -- still gets the full fan-out.
        let mut prompts = prompts.into_iter();
        let probe = scenes
            .first()
            .zip(prompts.next())
            .map(|(scene, prompt)| self.deep_tier_model_output(scene, check_type, prompt));
        let rest = futures_util::stream::iter(
            scenes
                .iter()
                .skip(1)
                .zip(prompts)
                .map(|(scene, prompt)| self.deep_tier_model_output(scene, check_type, prompt)),
        )
        .buffered(DEEP_CHECK_CONCURRENCY);

        // `chain` does not poll `rest` until the probe has yielded, so a caller
        // that bails on the probe's error never dispatches the remainder.
        futures_util::stream::iter(probe)
            .then(|future| future)
            .chain(rest)
    }

    pub(super) async fn deep_temporal_coherence_issues(
        &self,
        scenes: &[crate::sqlite::records::Scene],
        coverage: &mut spindle_core::models::AuditCoverage,
    ) -> Result<Vec<spindle_core::models::ConsistencyIssue>> {
        use spindle_core::models::ConsistencyIssue;

        let mut issues = Vec::new();
        // Emission-cadence asymmetry, by design:
        //   - a rating-clearance rejection (RatingNotCovered) is PER-SCENE:
        //     ratings can differ per scene, so each uncleared scene emits its own
        //     honest skip. It also stops the run (every remaining scene of that
        //     rating would hit the same wall), matching the landed behavior.
        //   - a generic TRANSPORT error is ONCE-PER-RUN: a dead route won't heal
        //     mid-run, so on the first transport failure we emit ONE deep-tier
        //     skip finding and STOP calling the model for remaining scenes rather
        //     than emitting a duplicate finding per scene. (Previously this arm
        //     silently returned Vec::new(), so a dead route read as a clean scan.)
        // Scenes with prose, fanned out with bounded concurrency and served from
        // the V0039 cache where this prose version was already analyzed. Results
        // come back in scene order, so the skip cadence below is unchanged.
        let scanned: Vec<&crate::sqlite::records::Scene> = scenes
            .iter()
            .filter(|scene| !scene.full_text.trim().is_empty())
            .collect();
        *coverage = spindle_core::models::AuditCoverage::new(
            "temporal_coherence",
            scanned.iter().map(|s| s.id.clone()),
        );
        let prompts = scanned
            .iter()
            .map(|scene| build_temporal_deep_check_prompt(scene))
            .collect();
        let mut outputs =
            std::pin::pin!(self.deep_tier_output_stream(&scanned, "temporal_coherence", prompts));

        let mut index = 0usize;
        while let Some(output) = futures_util::StreamExt::next(&mut outputs).await {
            let scene = scanned[index];
            index += 1;
            let findings = match output {
                // A truncated response is not a verdict: mark the scene
                // INCONCLUSIVE and keep auditing the remaining scenes.
                Ok(output) if output.truncated => {
                    coverage.mark_inconclusive(
                        scene.id.clone(),
                        "deep check output was truncated at max_tokens",
                    );
                    Vec::new()
                }
                // Parse regardless of adapter kind: a non-JSON local stub
                // (the default `review` route) parses to nothing, so Tier 2 is
                // a no-op unless a real review model returns structured findings.
                Ok(output) => match parse_deep_temporal_check_output(&output.output) {
                    Ok(findings) => {
                        if output.local_stub {
                            coverage.heuristic_ids.push(scene.id.clone());
                        } else {
                            coverage.evaluated_ids.push(scene.id.clone());
                        }
                        findings
                    }
                    _ => Vec::new(),
                },
                // Honest skip on a rating-clearance rejection: the `review` agent
                // is not cleared for this scene's rating, so the prose never left.
                // Emit one info finding that reads as SKIPPED (route + rating, no
                // prose) and stop — every remaining scene of this rating would hit
                // the same wall. PER-SCENE cadence (ratings can differ per scene).
                Err(err)
                    if matches!(
                        err.downcast_ref::<crate::ai::RouteClearanceError>(),
                        Some(crate::ai::RouteClearanceError::RatingNotCovered { .. })
                    ) =>
                {
                    issues.push(temporal_deep_skip_issue(&err));
                    return Ok(issues);
                }
                // A wall-clock timeout is INCONCLUSIVE for this scene, not a
                // dead route: keep auditing the remaining scenes.
                Err(err) if is_deep_check_timeout(&err) => {
                    coverage.mark_inconclusive(
                        scene.id.clone(),
                        "deep check timed out before producing a verdict",
                    );
                    Vec::new()
                }
                // Generic transport error (dead/unreachable route): a dead route
                // won't heal mid-run, so emit ONE deep-tier skip finding and STOP
                // calling the model for remaining scenes. This replaces the legacy
                // silent `Err(_) => Vec::new()` that let a dead route read as a
                // clean scan. Deterministic Tier-1 findings (produced by the
                // caller) are unaffected.
                Err(err) => {
                    issues.push(temporal_deep_skip_issue(&err));
                    return Ok(issues);
                }
            };

            for finding in findings {
                let mut message = format!("scene {}: {}", scene.id, finding.message.trim());
                if let Some(evidence) = finding.evidence.as_ref()
                    && !evidence.trim().is_empty()
                {
                    message.push_str(&format!(" Evidence: {}", evidence.trim()));
                }
                issues.push(ConsistencyIssue {
                    severity: finding.severity.unwrap_or_else(|| "warning".to_string()),
                    check_type: "temporal_coherence".to_string(),
                    message,
                    entity_ids: vec![scene.id.clone()],
                    suggested_action: Some(
                        "add a transition beat or scene break at the jump, split into separately \
                         clocked scenes, or mark an intended rewind with temporal_mode \"flashback\""
                            .to_string(),
                    ),
                    evidence: None,
                });
            }
        }

        Ok(issues)
    }

    /// Model-backed promise-payoff candidate detection (T-106, Tier 2).
    /// Proposes — never writes — that an unresolved promise the scoped prose
    /// already delivered should be confirmed via `update_promise_status`.
    ///
    /// `promises` arrives already filtered to non-resolved, non-archived rows in
    /// scope, urgency-ranked, and capped to the first 10; `unscanned` is the
    /// count that overflowed the cap (surfaced as one summary info finding so a
    /// silent cap never hides a dropped thread). `scenes` is the scoped prose.
    ///
    /// Unlike the temporal check's silent `Err(_) => Vec::new()`, an errored or
    /// unusable review route here emits ONE honest-skip info finding: a route
    /// failure must never read as a clean payoff scan.
    pub(super) async fn deep_promise_payoff_issues(
        &self,
        promises: &[crate::sqlite::records::NarrativePromise],
        scenes: &[crate::sqlite::records::Scene],
        unscanned: usize,
        coverage: &mut spindle_core::models::AuditCoverage,
    ) -> Result<Vec<spindle_core::models::ConsistencyIssue>> {
        use crate::ai::ModelRequest;
        use spindle_core::models::ConsistencyIssue;

        let mut issues = Vec::new();

        if unscanned > 0 {
            issues.push(ConsistencyIssue {
                severity: "info".to_string(),
                check_type: "promise_payoff_detection".to_string(),
                message: format!(
                    "{unscanned} additional unresolved promise(s) were not scanned for payoff \
                     (at most 10 candidates are audited per page; inspect audit_coverage.next_offset)"
                ),
                entity_ids: Vec::new(),
                suggested_action: Some(
                    "narrow the scope or resolve higher-urgency promises, then re-run".to_string(),
                ),
                evidence: None,
            });
        }

        // The prompt concatenates every scoped scene's prose, so the request
        // must carry the STRICTEST rating in the batch — the rating-gated
        // chokepoint then routes an explicit batch only to an explicit-cleared
        // agent (evolution §4). `None` (no scenes) leaves the request unrated.
        let batch_rating = max_scene_rating(scenes);

        for promise in promises {
            let matches = match self
                .repository
                .model_router()
                .complete_deep(&ModelRequest {
                    route: "review".to_string(),
                    prompt: build_promise_payoff_deep_check_prompt(promise, scenes),
                    rating: batch_rating.clone(),
                    context: Some(crate::ai::RequestContext {
                        project_id: Some(promise.project_id.clone()),
                        ..Default::default()
                    }),
                })
                .await
            {
                // A truncated response is not a verdict: mark the promise
                // INCONCLUSIVE and keep auditing the remaining promises.
                Ok(response) if response.truncated => {
                    coverage.mark_inconclusive(
                        promise.id.clone(),
                        "deep check output was truncated at max_tokens",
                    );
                    Vec::new()
                }
                // A non-JSON local stub (the default `review` route) parses to
                // nothing, so a local-only deployment degrades to no proposals.
                Ok(response) => match parse_deep_promise_payoff_output(&response.output) {
                    Ok(findings) => {
                        if response.adapter_kind == "local" {
                            coverage.heuristic_ids.push(promise.id.clone());
                        } else {
                            coverage.evaluated_ids.push(promise.id.clone());
                        }
                        findings
                    }
                    _ => Vec::new(),
                },
                // A wall-clock timeout is INCONCLUSIVE for this promise, not a
                // dead route: keep auditing the remaining promises.
                Err(err) if is_deep_check_timeout(&err) => {
                    coverage.mark_inconclusive(
                        promise.id.clone(),
                        "deep check timed out before producing a verdict",
                    );
                    Vec::new()
                }
                // Honest skip: the review route is unreachable/missing or not
                // cleared for the batch rating. Emit one info finding that reads
                // as SKIPPED rather than clean, then stop (every promise hits the
                // same route). A rating-clearance failure names route+rating —
                // never any prose.
                Err(err) => {
                    issues.push(payoff_skip_issue(&err));
                    return Ok(issues);
                }
            };

            for m in matches {
                if !m.paid_off {
                    continue;
                }
                let scene_ref = m
                    .scene_ref
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("the scoped prose");
                let desc = crate::format::truncate_at_chars(&promise.description, 80);
                let mut message = format!(
                    "promise {} (\"{}\") appears to be paid off in {} — confirm with \
                     update_promise_status",
                    promise.id, desc, scene_ref
                );
                if let Some(evidence) = m.evidence.as_ref() {
                    let cleaned = evidence.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !cleaned.is_empty() {
                        message.push_str(&format!(
                            " Evidence: {}",
                            crate::format::truncate_at_chars(&cleaned, 200)
                        ));
                    }
                }
                issues.push(ConsistencyIssue {
                    severity: "info".to_string(),
                    check_type: "promise_payoff_detection".to_string(),
                    message,
                    entity_ids: vec![promise.id.clone()],
                    suggested_action: Some(
                        "confirm the payoff and call update_promise_status(\"paid_off\"), or \
                         reinforce the thread if it is not actually landed"
                            .to_string(),
                    ),
                    evidence: None,
                });
            }
        }

        Ok(issues)
    }

    /// Model-backed scene-purpose fulfillment audit (evolution design §3.6):
    /// did the drafted scene accomplish its chapter-plan `purpose`?
    ///
    /// `units` arrives in scene story order, already capped to the first
    /// [`SCENE_PURPOSE_SCAN_CAP`]; `unscanned` is the overflow surfaced as one
    /// summary info finding so a silent cap never hides an unaudited scene. One
    /// model call per unit carries that scene's prose, stamped with the scene's
    /// rating so the router applies the `(review, rating)` override (evolution
    /// §4: an explicit-rated scene's audit never reaches an uncleared model) —
    /// the same per-scene pattern as the behavioral secret-leak audit.
    ///
    /// Findings are ADVISORY (`info`, design §3.6: purpose drift is editorial
    /// judgment, not an error). `fulfilled:true` yields no finding — silence is
    /// the pass. A verdict whose `fulfilled` field is missing is DISCARDED (the
    /// model does not get to leave the verdict ambiguous), and a malformed
    /// payload yields no finding for that scene (never a guess). A route failure
    /// emits one honest-skip info finding (never reads as clean) and stops.
    pub(super) async fn deep_scene_purpose_issues(
        &self,
        units: &[ScenePurposeScanUnit],
        unscanned: usize,
        coverage: &mut spindle_core::models::AuditCoverage,
    ) -> Result<Vec<spindle_core::models::ConsistencyIssue>> {
        use spindle_core::models::ConsistencyIssue;

        let mut issues = Vec::new();

        if unscanned > 0 {
            issues.push(ConsistencyIssue {
                severity: "info".to_string(),
                check_type: "scene_purpose_fulfillment".to_string(),
                message: format!(
                    "{unscanned} additional scene(s) with a planned purpose were not scanned for \
                     purpose fulfillment (at most {SCENE_PURPOSE_SCAN_CAP} scenes in \
                     scope are audited per page; inspect audit_coverage.next_offset)"
                ),
                entity_ids: Vec::new(),
                suggested_action: Some(
                    "narrow the scope to the scenes you want audited, then re-run".to_string(),
                ),
                evidence: None,
            });
        }

        for unit in units {
            let prompt = build_scene_purpose_prompt(&unit.purpose, &unit.scene_text);
            let request = build_scene_purpose_request(&unit.scene_rating, prompt);
            let verdict = match self.repository.model_router().complete_deep(&request).await {
                // A truncated response is not a verdict: mark the scene
                // INCONCLUSIVE and keep auditing the remaining scenes.
                Ok(response) if response.truncated => {
                    coverage.mark_inconclusive(
                        unit.scene_id.clone(),
                        "deep check output was truncated at max_tokens",
                    );
                    continue;
                }
                // A non-JSON local stub (the default `review` route) parses to
                // an error the caller treats as no finding rather than a guess.
                Ok(response) => match parse_scene_purpose_output(&response.output) {
                    Ok(verdict) if verdict.fulfilled.is_some() => {
                        if response.adapter_kind == "local" {
                            coverage.heuristic_ids.push(unit.scene_id.clone());
                        } else {
                            coverage.evaluated_ids.push(unit.scene_id.clone());
                        }
                        verdict
                    }
                    _ => continue,
                },
                // A wall-clock timeout is INCONCLUSIVE for this scene, not a
                // dead route: keep auditing the remaining scenes.
                Err(err) if is_deep_check_timeout(&err) => {
                    coverage.mark_inconclusive(
                        unit.scene_id.clone(),
                        "deep check timed out before producing a verdict",
                    );
                    continue;
                }
                // Honest skip: the review route is unreachable/missing or not
                // cleared for the scene rating. Emit one info finding that reads
                // as SKIPPED rather than clean, then stop (every unit hits the
                // same route). A rating-clearance failure names route+rating —
                // never any prose.
                Err(err) => {
                    issues.push(scene_purpose_skip_issue(&err));
                    return Ok(issues);
                }
            };

            // Discard a verdict with no `fulfilled` field (never guess); a
            // `fulfilled:true` verdict is the silent pass.
            match verdict.fulfilled {
                Some(false) => {}
                Some(true) | None => continue,
            }

            let purpose = crate::format::truncate_at_chars(&unit.purpose, 120);
            let mut message = format!(
                "scene {} did not appear to accomplish its planned purpose (\"{}\")",
                unit.scene_ref, purpose
            );
            if let Some(assessment) = unit.assessment(&verdict) {
                message.push_str(&format!(" — {assessment}"));
            }
            issues.push(ConsistencyIssue {
                severity: "info".to_string(),
                check_type: "scene_purpose_fulfillment".to_string(),
                message,
                entity_ids: vec![unit.scene_id.clone()],
                suggested_action: Some(
                    "if the scene drifted from its purpose, revise it or replan the beat; if the \
                     divergence improves the story, update the plan's purpose to match"
                        .to_string(),
                ),
                evidence: None,
            });
        }

        Ok(issues)
    }

    /// Model-backed behavioral secret-leak audit (design §2.4 deep tier, Item 5).
    ///
    /// `units` arrives ranked by scene story order and already capped to the
    /// first [`SECRET_LEAK_DEEP_SCAN_CAP`] (scene, secret-set) calls; `unscanned`
    /// is the overflow surfaced as one summary info finding so a silent cap never
    /// hides an unaudited scene. One model call per unit carries that scene's
    /// prose + its in-play secrets, stamped with the scene's rating so the router
    /// applies the `(review, rating)` override (design §5: an explicit-rated
    /// scene's audit never reaches an uncleared model).
    ///
    /// Findings add to the SAME `secret_leak` check_type as the deterministic
    /// tier, suffixed "(model-detected behavioral leak)". A finding whose
    /// `character_id` is not a present out-of-circle character for that scene is
    /// DISCARDED — the model does not get to invent violations. A route failure
    /// emits one honest-skip info finding (never reads as clean) and stops.
    pub(super) async fn deep_secret_behavioral_leak_issues(
        &self,
        units: &[SecretLeakScanUnit],
        unscanned: usize,
        coverage: &mut spindle_core::models::AuditCoverage,
    ) -> Result<Vec<spindle_core::models::ConsistencyIssue>> {
        use spindle_core::models::ConsistencyIssue;

        let mut issues = Vec::new();

        if unscanned > 0 {
            issues.push(ConsistencyIssue {
                severity: "info".to_string(),
                check_type: "secret_leak".to_string(),
                message: format!(
                    "{unscanned} additional (scene, secret-set) pair(s) were not scanned for \
                     behavioral secret leaks (at most {SECRET_LEAK_DEEP_SCAN_CAP} scenes \
                     in scope are audited per page; inspect audit_coverage.next_offset)"
                ),
                entity_ids: Vec::new(),
                suggested_action: Some(
                    "narrow the scope to the scenes you want audited, then re-run".to_string(),
                ),
                evidence: None,
            });
        }

        for unit in units {
            let prompt = build_secret_behavioral_leak_prompt(&unit.scene_text, &unit.secrets);
            let request = build_secret_behavioral_leak_request(&unit.scene_rating, prompt);
            let findings = match self.repository.model_router().complete_deep(&request).await {
                // A truncated response is not a verdict: mark the scene
                // INCONCLUSIVE and keep auditing the remaining scenes.
                Ok(response) if response.truncated => {
                    coverage.mark_inconclusive(
                        unit.scene_id.clone(),
                        "deep check output was truncated at max_tokens",
                    );
                    Vec::new()
                }
                // A non-JSON local stub (the default `review` route) parses to
                // nothing, so a local-only deployment adds no behavioral findings.
                Ok(response) => match parse_deep_secret_leak_output(&response.output) {
                    Ok(findings) => {
                        if response.adapter_kind == "local" {
                            coverage.heuristic_ids.push(unit.scene_id.clone());
                        } else {
                            coverage.evaluated_ids.push(unit.scene_id.clone());
                        }
                        findings
                    }
                    _ => Vec::new(),
                },
                // A wall-clock timeout is INCONCLUSIVE for this scene, not a
                // dead route: keep auditing the remaining scenes.
                Err(err) if is_deep_check_timeout(&err) => {
                    coverage.mark_inconclusive(
                        unit.scene_id.clone(),
                        "deep check timed out before producing a verdict",
                    );
                    Vec::new()
                }
                // Honest skip: the review route is unreachable/missing or not
                // cleared for the scene rating. Emit one info finding that reads
                // as SKIPPED rather than clean, then stop (every unit hits the
                // same route). A rating-clearance failure names route+rating —
                // never any prose.
                Err(err) => {
                    issues.push(secret_leak_skip_issue(&err));
                    return Ok(issues);
                }
            };

            for finding in findings {
                // Discard: the model may only name a present out-of-circle
                // character for this scene. Anything else (a knower, an invented
                // id, an absent character) is dropped.
                let Some(name) = unit.unaware_present.get(finding.character_id.as_str()) else {
                    continue;
                };
                let mut message = format!(
                    "scene {}: {} ({}) betrays knowledge of a secret they were never told",
                    unit.scene_ref, name, finding.character_id,
                );
                if let Some(evidence) = finding
                    .evidence
                    .as_deref()
                    .or(finding.description.as_deref())
                {
                    let cleaned = evidence.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !cleaned.is_empty() {
                        message.push_str(&format!(
                            " Evidence: {}",
                            crate::format::truncate_at_chars(&cleaned, 200)
                        ));
                    }
                }
                message.push_str(" (model-detected behavioral leak)");
                issues.push(ConsistencyIssue {
                    severity: "warning".to_string(),
                    check_type: "secret_leak".to_string(),
                    message,
                    entity_ids: vec![finding.character_id.clone(), unit.scene_id.clone()],
                    suggested_action: Some(
                        "record the reveal (record_knowledge with secret_of_fact_id) if intended, \
                         or revise the scene so the out-of-circle character does not act on the secret"
                            .to_string(),
                    ),
                    evidence: None,
                });
            }
        }

        Ok(issues)
    }

    /// Find scene/book/chapter spine inconsistencies. Mirrors
    /// `services/mod.rs:6672` in 705b835^.
    pub(super) async fn find_orphan_scenes(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<SceneSpineIntegrityIssue>> {
        use std::collections::BTreeMap;

        let books = self.repository.list_books_by_project(project_id).await?;
        let mut books_by_id = BTreeMap::new();
        let mut chapters_by_id = BTreeMap::new();

        for book in &books {
            books_by_id.insert(book.id.clone(), book.clone());
            for chapter in self.repository.list_chapters_by_book(&book.id).await? {
                chapters_by_id.insert(chapter.id.clone(), chapter);
            }
        }

        let scenes = self
            .repository
            .list_scenes_by_project_and_branch(project_id, branch_id)
            .await?;
        let mut issues = Vec::new();

        for scene in &scenes {
            let scene_id = scene.id.clone();
            let book_id = scene.book_id.clone();
            let chapter_id = scene.chapter_id.clone();

            match books_by_id.get(&book_id) {
                Some(book) => {
                    if book.book_number != scene.book_number {
                        issues.push(SceneSpineIntegrityIssue {
                            severity: "error",
                            book_number: scene.book_number,
                            chapter_number: scene.chapter_number,
                            scene_order: scene.scene_order,
                            message: format!(
                                "scene {} points to book {} but carries book_number {}",
                                scene_id, book_id, scene.book_number
                            ),
                            entity_ids: vec![scene_id.clone()],
                            suggested_action: Some(
                                "repair the scene book_id or renumber the scene placement"
                                    .to_string(),
                            ),
                        });
                    }
                }
                None => issues.push(SceneSpineIntegrityIssue {
                    severity: "error",
                    book_number: scene.book_number,
                    chapter_number: scene.chapter_number,
                    scene_order: scene.scene_order,
                    message: format!("scene {} points to missing book_id {}", scene_id, book_id),
                    entity_ids: vec![scene_id.clone()],
                    suggested_action: Some(
                        "repair the scene book_id or recreate the missing book record".to_string(),
                    ),
                }),
            }

            match chapters_by_id.get(&chapter_id) {
                Some(chapter) => {
                    let chapter_book_id = chapter.book_id.clone();
                    if chapter_book_id != book_id {
                        issues.push(SceneSpineIntegrityIssue {
                            severity: "error",
                            book_number: scene.book_number,
                            chapter_number: scene.chapter_number,
                            scene_order: scene.scene_order,
                            message: format!(
                                "scene {} points to chapter {} in book {} but carries book_id {}",
                                scene_id, chapter_id, chapter_book_id, book_id
                            ),
                            entity_ids: vec![scene_id.clone()],
                            suggested_action: Some(
                                "repair the scene chapter_id/book_id pair so they refer to the same chapter spine"
                                    .to_string(),
                            ),
                        });
                    }
                    if chapter.book_number != scene.book_number
                        || chapter.chapter_number != scene.chapter_number
                    {
                        issues.push(SceneSpineIntegrityIssue {
                            severity: "error",
                            book_number: scene.book_number,
                            chapter_number: scene.chapter_number,
                            scene_order: scene.scene_order,
                            message: format!(
                                "scene {} points to chapter {} but carries placement {}.{}",
                                scene_id, chapter_id, scene.book_number, scene.chapter_number
                            ),
                            entity_ids: vec![scene_id.clone()],
                            suggested_action: Some(
                                "repair the scene chapter_id or renumber the scene placement"
                                    .to_string(),
                            ),
                        });
                    }
                }
                None => issues.push(SceneSpineIntegrityIssue {
                    severity: "error",
                    book_number: scene.book_number,
                    chapter_number: scene.chapter_number,
                    scene_order: scene.scene_order,
                    message: format!(
                        "scene {} points to missing chapter_id {}",
                        scene_id, chapter_id
                    ),
                    entity_ids: vec![scene_id.clone()],
                    suggested_action: Some(
                        "repair the scene chapter_id or recreate the missing chapter record"
                            .to_string(),
                    ),
                }),
            }
        }

        let mut scenes_by_position: BTreeMap<(i32, i32, i32), Vec<String>> = BTreeMap::new();
        for scene in &scenes {
            scenes_by_position
                .entry((scene.book_number, scene.chapter_number, scene.scene_order))
                .or_default()
                .push(scene.id.clone());
        }

        for ((book_number, chapter_number, scene_order), entity_ids) in scenes_by_position {
            if entity_ids.len() < 2 {
                continue;
            }
            issues.push(SceneSpineIntegrityIssue {
                severity: "error",
                book_number,
                chapter_number,
                scene_order,
                message: format!(
                    "book {} chapter {} has duplicate scene_order {}",
                    book_number, chapter_number, scene_order
                ),
                entity_ids,
                suggested_action: Some(
                    "renumber or move the colliding scenes so each chapter position is unique"
                        .to_string(),
                ),
            });
        }

        Ok(issues)
    }
    pub(super) async fn resolve_phase_four_caches(
        &self,
        project_id: &str,
        branch_id: &str,
        ids: &[PhaseFourCacheId],
    ) -> Result<usize> {
        let mut resolved = 0;
        for id in ids {
            resolved += self
                .repository
                .resolve_validator_findings_for_validator(project_id, branch_id, id.as_str())
                .await?;
        }
        Ok(resolved)
    }

    pub(crate) async fn resolve_phase_four_caches_for_project(
        &self,
        project_id: &str,
        ids: &[PhaseFourCacheId],
    ) -> Result<usize> {
        let mut resolved = 0;
        for branch in self.repository.list_branches_by_project(project_id).await? {
            resolved += self
                .resolve_phase_four_caches(project_id, &branch.id, ids)
                .await?;
        }
        Ok(resolved)
    }

    pub(super) async fn phase_four_cache_target_for_entity_update(
        &self,
        entity_type: &str,
        entity_id: &str,
        changes: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Option<(String, Option<String>, Vec<PhaseFourCacheId>)>> {
        if changes.is_empty() {
            return Ok(None);
        }

        Ok(match entity_type {
            "project"
                if changes.contains_key("genre")
                    || changes.contains_key("project_type")
                    || changes.contains_key("reader_contract")
                    || changes.contains_key("promise")
                    || changes.contains_key("boundaries")
                    || changes.contains_key("style_notes") =>
            {
                let project = self.repository.get_project(entity_id).await?;
                Some((project.id, None, vec![PhaseFourCacheId::StyleCompliance]))
            }
            "world_rule" => {
                let rule = self.repository.get_world_rule(entity_id).await?;
                let mut cache_ids = vec![PhaseFourCacheId::WorldRuleSemanticDrift];
                if rule.rule_type.eq_ignore_ascii_case("style")
                    || changes
                        .get("rule_type")
                        .and_then(|value| value.as_str())
                        .is_some_and(|rule_type| rule_type.eq_ignore_ascii_case("style"))
                {
                    cache_ids.push(PhaseFourCacheId::StyleCompliance);
                }
                Some((rule.project_id, Some(rule.branch_id), cache_ids))
            }
            "character"
                if changes.contains_key("name")
                    || changes.contains_key("normalized_name")
                    || changes.contains_key("voice_profile")
                    || changes.contains_key("voice_profile_data") =>
            {
                let character = self.repository.get_character(entity_id).await?;
                Some((
                    character.project_id,
                    Some(character.branch_id),
                    vec![PhaseFourCacheId::VoiceDrift],
                ))
            }
            "timeline_event" => {
                let event = self.repository.get_timeline_event(entity_id).await?;
                Some((
                    event.project_id,
                    Some(event.branch_id),
                    vec![PhaseFourCacheId::RetconReachability],
                ))
            }
            "temporal_intervention" => {
                let intervention = self.repository.get_temporal_intervention(entity_id).await?;
                Some((
                    intervention.project_id,
                    Some(intervention.branch_id),
                    vec![PhaseFourCacheId::RetconReachability],
                ))
            }
            _ => None,
        })
    }
}

/// Conservative lexical prefilter, not a general prose claim extractor. Only a
/// named subject followed by an action is evidence; mentions alone are not.
/// Common base verbs used to recognize third-person finite forms (`counts`,
/// `sees`, `meets`) without requiring every inflected form in the main list.
const ACTION_STEMS: &[&str] = &[
    "count",
    "meet",
    "see",
    "run",
    "know",
    "think",
    "tell",
    "find",
    "come",
    "go",
    "take",
    "give",
    "make",
    "get",
    "grow",
    "hear",
    "keep",
    "leave",
    "lose",
    "mean",
    "pay",
    "ride",
    "ring",
    "rise",
    "sink",
    "sit",
    "sleep",
    "speak",
    "stand",
    "steal",
    "strike",
    "swim",
    "swing",
    "teach",
    "tear",
    "throw",
    "understand",
    "wake",
    "wear",
    "win",
    "write",
    "fight",
    "fly",
    "forget",
    "open",
    "read",
    "say",
    "ask",
    "walk",
    "step",
];

/// Irregular past/finite forms that no suffix rule can derive.
const IRREGULAR_FINITE_VERBS: &[&str] = &[
    "met",
    "ran",
    "saw",
    "came",
    "went",
    "took",
    "gave",
    "made",
    "knew",
    "thought",
    "found",
    "told",
    "said",
    "left",
    "felt",
    "became",
    "began",
    "brought",
    "held",
    "wrote",
    "spoke",
    "stood",
    "sat",
    "rose",
    "fell",
    "led",
    "bit",
    "broke",
    "chose",
    "drank",
    "drove",
    "ate",
    "fought",
    "flew",
    "forgot",
    "got",
    "grew",
    "heard",
    "hid",
    "hit",
    "kept",
    "laid",
    "lost",
    "meant",
    "paid",
    "put",
    "read",
    "rode",
    "rang",
    "sang",
    "sank",
    "set",
    "shook",
    "shone",
    "shot",
    "showed",
    "shut",
    "slept",
    "slid",
    "spread",
    "sprang",
    "stole",
    "stuck",
    "struck",
    "swore",
    "swept",
    "swam",
    "swung",
    "taught",
    "tore",
    "threw",
    "understood",
    "woke",
    "wore",
    "won",
    "wound",
];

fn is_finite_action_verb(verb: &str) -> bool {
    if NARRATIVE_ACTIONS.contains(&verb)
        || looks_like_narrative_verb(verb)
        || IRREGULAR_FINITE_VERBS.contains(&verb)
    {
        return true;
    }
    if let Some(stem) = verb.strip_suffix("es")
        && ACTION_STEMS.contains(&stem)
    {
        return true;
    }
    if let Some(stem) = verb.strip_suffix('s')
        && ACTION_STEMS.contains(&stem)
    {
        return true;
    }
    false
}

const NARRATIVE_ACTIONS: &[&str] = &[
    "said",
    "says",
    "asked",
    "asks",
    "whispered",
    "whispers",
    "shouted",
    "shouts",
    "stepped",
    "steps",
    "walked",
    "walks",
    "ran",
    "runs",
    "entered",
    "enters",
    "vaulted",
    "vaults",
    "loosed",
    "looses",
    "locked",
    "locks",
    "drew",
    "draws",
    "pointed",
    "points",
    "fired",
    "fires",
    "lifted",
    "lifts",
    "opened",
    "opens",
    "signed",
    "signs",
    "spoke",
    "speaks",
    "replied",
    "replies",
    "nodded",
    "nods",
    "reached",
    "reaches",
    "grabbed",
    "grabs",
    "laughed",
    "laughs",
    "smiled",
    "smiles",
    "frowned",
    "frowns",
    "crossed",
    "crosses",
    "turned",
    "turns",
    "moved",
    "moves",
    "stood",
    "stands",
    "sat",
    "sits",
    "rose",
    "rises",
    "climbed",
    "climbs",
    "knelt",
    "kneels",
    "fell",
    "falls",
    "struck",
    "strikes",
    "swung",
    "swings",
    "thrust",
    "thrusts",
    "kicked",
    "kicks",
    "pushed",
    "pushes",
    "pulled",
    "pulls",
    "carried",
    "carries",
    "threw",
    "throws",
    "caught",
    "catches",
    "held",
    "holds",
    "released",
    "releases",
    "attacked",
    "attacks",
    "defended",
    "defends",
    "screamed",
    "screams",
    "wept",
    "weeps",
    "cried",
    "cries",
    "breathed",
    "breathes",
    "stared",
    "stares",
    "looked",
    "looks",
    "watched",
    "watches",
    "listened",
    "listens",
    "read",
    "reads",
    "wrote",
    "writes",
    "sang",
    "sings",
    "danced",
    "dances",
    "drank",
    "drinks",
    "ate",
    "eats",
    "slept",
    "sleeps",
    "woke",
    "wakes",
    "waited",
    "waits",
    "returned",
    "returns",
    "left",
    "leaves",
    "arrived",
    "arrives",
    "followed",
    "follows",
    "led",
    "leads",
    "rode",
    "rides",
    "sailed",
    "sails",
    "flew",
    "flies",
    "jumped",
    "jumps",
    "crawled",
    "crawls",
    "touched",
    "touches",
];

/// Passing death references ("since Maren drowned", "Bran's funeral", "the
/// late Bran", "remembering Bran") are not present-tense action evidence.
fn is_death_reference_mention(lower: &str, start: usize, matched_len: usize) -> bool {
    let before = lower[..start].rsplit(';').next().unwrap_or_default();
    let before = before.rsplit(',').next().unwrap_or_default();
    let before = before.rsplit(" but ").next().unwrap_or_default();
    let before = before.rsplit(" yet ").next().unwrap_or_default();
    let before = before.trim_end_matches(|ch: char| ch.is_whitespace() || ch == ',');
    if before.ends_with("the late") || before.ends_with("late") {
        return true;
    }
    // Only a memory/mourning verb attached to this candidate suppresses the
    // action. A death in another clause ("After Tomas died, Veyra opened...")
    // or a place ("Beside the grave, Veyra opened...") belongs to someone else.
    if [
        "remembered",
        "remembering",
        "remember",
        "memory",
        "mourned",
        "mourning",
        "grieving",
        "grief",
    ]
    .iter()
    .any(|word| contains_case_insensitive_word_boundary(before, word))
    {
        return true;
    }
    let after = lower[start + matched_len..]
        .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | ':'));
    if after.starts_with("'s") || after.starts_with("’s") {
        let rest = after
            .trim_start_matches("'s")
            .trim_start_matches("’s")
            .trim_start_matches(|ch: char| ch.is_whitespace());
        if [
            "death", "funeral", "grave", "burial", "wake", "memory", "ghost", "corpse",
        ]
        .iter()
        .any(|word| rest.starts_with(word))
        {
            return true;
        }
    }
    [
        "died",
        "dies",
        "dying",
        "drowned",
        "drowns",
        "drowning",
        "was killed",
        "were killed",
        "had been killed",
        "has been killed",
        "was dead",
        "is dead",
        "death",
        "funeral",
        "grave",
        "buried",
        "burial",
        "lost at sea",
    ]
    .iter()
    .any(|phrase| after.starts_with(phrase))
}

/// A relative/object reference such as "the same kind Bran brought up" is not
/// a character-subject action after death.
fn is_relative_reference_before(before: &str) -> bool {
    let trimmed = before.trim_end_matches(|ch: char| ch.is_whitespace() || ch == ',');
    [
        "the same kind",
        "the kind",
        "the sort",
        "the one",
        "a kind of",
        "the same",
    ]
    .iter()
    .any(|marker| trimmed.ends_with(marker))
}

fn has_named_character_action(text: &str, aliases: &[String]) -> bool {
    if aliases.is_empty() {
        return false;
    }

    text.split(['.', '!', '?', '\n']).any(|sentence| {
        let lower = sentence.to_lowercase();
        aliases.iter().any(|alias| {
            let name = alias.trim().to_lowercase();
            lower.match_indices(&name).any(|(start, matched)| {
                // Exclusions qualify this named action, not arbitrary words in
                // its objects or another clause (e.g. a recording-room door).
                let before = lower[..start].rsplit(';').next().unwrap_or_default();
                let before = before.rsplit(" but ").next().unwrap_or_default();
                let before = before.rsplit(" yet ").next().unwrap_or_default();
                let hypothetical = ["if", "unless", "suppose", "supposing"]
                    .iter()
                    .any(|word| contains_case_insensitive_word_boundary(before, word));
                let framed = [
                    "in a flashback",
                    "years ago",
                    "before the death",
                    "before her death",
                    "before his death",
                    "in her memory",
                    "in his memory",
                    "in the memory",
                    "in a dream",
                    "in her dream",
                    "in his dream",
                    "remembered how",
                    "remembered when",
                    "remembered that",
                    "imagined that",
                    "dreamed that",
                    "the recording played",
                ]
                .iter()
                .any(|frame| {
                    before
                        .trim_end()
                        .trim_end_matches([',', ':'])
                        .trim_end()
                        .ends_with(frame)
                });
                let framed_prefix = [
                    "letter",
                    "memory",
                    "remembered",
                    "recording",
                    "portrait",
                    "photograph",
                    "legend",
                    "rumor",
                    "rumour",
                    "years ago",
                    "before the death",
                ]
                .iter()
                .any(|frame| contains_case_insensitive_word_boundary(before, frame));
                let framed_subject = [
                    "on the recording,",
                    "in the recording,",
                    "a ghost",
                    "the ghost",
                    "a hologram of",
                    "the hologram of",
                ]
                .iter()
                .any(|frame| before.trim_end().ends_with(frame));
                if hypothetical || framed || framed_prefix || framed_subject {
                    return false;
                }
                if lower[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric())
                {
                    return false;
                }
                if !sentence
                    .get(start..)
                    .and_then(|rest| rest.chars().next())
                    .is_some_and(char::is_uppercase)
                {
                    return false;
                }
                if is_death_reference_mention(&lower, start, matched.len())
                    || is_relative_reference_before(&lower[..start])
                {
                    return false;
                }
                let tail = &lower[start + matched.len()..];
                if !tail.starts_with(char::is_whitespace) {
                    return false;
                }
                let mut words = tail.split_whitespace();
                let first = words.next().unwrap_or_default();
                let verb = if ["was", "is", "were", "are"].contains(&first)
                    || ["quietly", "quickly", "slowly", "suddenly", "also", "then"].contains(&first)
                {
                    words.next().unwrap_or_default()
                } else {
                    first
                };
                let verb = verb.trim_matches(|c: char| !c.is_alphabetic());
                is_finite_action_verb(verb)
            })
        })
    })
}

/// Explicit post-death aliveness assertions ("was still alive", "kept
/// breathing") are acting evidence even though they use a copula rather than a
/// narrative verb. Keep the phrase list tight so memory/negation controls do
/// not become dead-character hits.
fn has_named_character_alive_assertion(text: &str, aliases: &[String]) -> bool {
    // Longest first so "was still alive" wins before "was alive" prefix checks.
    const ALIVE_PHRASES: &[&str] = &[
        "was still alive",
        "is still alive",
        "was still breathing",
        "is still breathing",
        "was alive",
        "is alive",
        "still alive",
        "was breathing",
        "is breathing",
        "still breathing",
        "kept breathing",
        "had survived",
        "has survived",
    ];
    const NEGATED: &[&str] = &["was not", "wasn't", "is not", "isn't", "never", "no longer"];
    const FRAMES: &[&str] = &[
        "letter",
        "dream",
        "memory",
        "remembered",
        "flashback",
        "recording",
        "portrait",
        "photograph",
        "legend",
        "rumor",
        "rumour",
        "years ago",
        "before the death",
        "before her death",
        "before his death",
    ];
    const HISTORICAL: &[&str] = &["years ago", "long ago", "in the past", "before the death"];
    text.split(['.', '!', '?', '\n']).any(|sentence| {
        let lower = sentence.to_ascii_lowercase();
        aliases.iter().any(|alias| {
            let alias = alias.trim().to_ascii_lowercase();
            if alias.is_empty() {
                return false;
            }
            lower.match_indices(&alias).any(|(start, matched)| {
                let before_ok = lower[..start]
                    .chars()
                    .next_back()
                    .is_none_or(|ch| !ch.is_alphanumeric());
                if !before_ok
                    || !sentence
                        .get(start..)
                        .and_then(|rest| rest.chars().next())
                        .is_some_and(char::is_uppercase)
                {
                    return false;
                }
                let before = &lower[..start];
                if FRAMES
                    .iter()
                    .any(|frame| contains_case_insensitive_word_boundary(before, frame))
                    || ["if", "unless", "suppose", "supposing"]
                        .iter()
                        .any(|word| contains_case_insensitive_word_boundary(before, word))
                {
                    return false;
                }
                let tail = lower[start + matched.len()..].trim_start_matches(|ch: char| {
                    ch.is_whitespace() || matches!(ch, ',' | ';' | ':')
                });
                if NEGATED.iter().any(|negation| tail.starts_with(negation)) {
                    return false;
                }
                ALIVE_PHRASES.iter().any(|phrase| {
                    tail.strip_prefix(phrase).is_some_and(|rest| {
                        (rest.is_empty()
                            || rest.starts_with(|ch: char| {
                                ch.is_whitespace() || matches!(ch, ',' | ';' | '.')
                            }))
                            && !HISTORICAL
                                .iter()
                                .any(|marker| rest.to_ascii_lowercase().contains(marker))
                    })
                })
            })
        })
    })
}

/// Broad, conservative verb shape fallback/// Broad, conservative verb shape fallback: past-tense/present-participle
/// narrative verbs not enumerated above. State adjectives that happen to end
/// in -ed are excluded so "was tired" is not an action.
fn looks_like_narrative_verb(verb: &str) -> bool {
    const NON_ACTION_ED: &[&str] = &[
        "bed", "red", "need", "feed", "seed", "shed", "wed", "dead", "tired", "sacred", "wicked",
        "rugged", "jagged", "naked", "aged", "indeed", "hundred", "afraid",
    ];
    (verb.len() > 3 && (verb.ends_with("ed") || verb.ends_with("ing")))
        && !NON_ACTION_ED.contains(&verb)
}

#[cfg(test)]
mod dead_action_tests {
    use super::{has_named_character_action, has_named_character_alive_assertion};

    #[test]
    fn named_actions_require_unqualified_narrative_evidence() {
        let aliases = vec!["Veyra".to_string()];
        for text in [
            "Veyra stepped into the hall.",
            "Veyra quietly opened the door.",
            "Veyra said, ‘Go.’",
            "They remembered Veyra. Veyra drew her sword.",
            "Veyra opened the recording room door.",
            "Veyra stepped past the ghost town.",
            "In a dream laboratory, Veyra opened the door.",
            "In a flashback exhibit room, Veyra opened the door.",
            "Garrick remembered how Eli died, but Veyra opened the door.",
        ] {
            assert!(has_named_character_action(text, &aliases), "{text}");
        }
        for text in [
            "Garrick remembered Veyra and whispered a prayer.",
            "In a flashback, Veyra stepped into the hall.",
            "Years ago, Veyra drew her sword.",
            "The recording played: Veyra said, ‘Go.’",
            "Veyra's ghost opened the door.",
            "Veyra did not open the door.",
            "If only Veyra walked beside them.",
            "If Veyra opened the door, the alarm would sound.",
            "Unless Veyra opened the door, they would be trapped.",
            "Garrick remembered how Veyra opened the door.",
            "Veyra had stepped into the hall.",
            "Veyra would have opened the door.",
            "OtherVeyra stepped into the hall.",
        ] {
            assert!(!has_named_character_action(text, &aliases), "{text}");
        }
    }

    #[test]
    fn irregular_and_third_person_finite_verbs_are_actions() {
        let aliases = vec!["Cress".to_string()];
        for text in [
            "Cress met her at the door.",
            "Cress counts the baskets.",
            "Cress ran down the hill.",
            "Cress sees the crack.",
            "Cress knows the ledger.",
            "Cress thought about the tide.",
        ] {
            assert!(has_named_character_action(text, &aliases), "{text}");
        }
        for text in [
            "Cress's mettle held.",
            "Cress counted on nothing.",
            "Cress seesawed on the beam.",
        ] {
            let _ = has_named_character_action(text, &aliases);
        }
    }

    #[test]
    fn death_references_and_relative_clauses_are_not_actions() {
        for (text, name) in [
            ("Three weeks had passed since Maren drowned.", "Maren"),
            ("Maren's funeral was held on the hill.", "Maren"),
            ("The late Maren left a key in the tin.", "Maren"),
            ("When Maren died, the bell rang.", "Maren"),
            ("Iris remembered Maren and looked away.", "Maren"),
            ("The same kind Bran brought up from the wreck.", "Bran"),
        ] {
            assert!(
                !has_named_character_action(text, &[name.to_string()]),
                "death reference must not be action: {text}"
            );
        }
        assert!(has_named_character_action(
            "Maren opened the door.",
            &["Maren".to_string()]
        ));
        for text in [
            "After Tomas died, Veyra opened the door.",
            "Beside the grave, Veyra opened the door.",
        ] {
            assert!(
                has_named_character_action(text, &["Veyra".to_string()]),
                "unrelated death reference must not hide action: {text}"
            );
        }
    }

    #[test]
    fn alive_assertions_bind_to_the_named_subject_and_ignore_history() {
        let aliases = vec!["Veyra Vale".to_string(), "Veyra".to_string()];
        assert!(has_named_character_alive_assertion(
            "Veyra was still alive when the door opened.",
            &aliases
        ));
        assert!(!has_named_character_alive_assertion(
            "At Veyra Vale's grave, Tomas was still alive.",
            &aliases
        ));
        assert!(!has_named_character_alive_assertion(
            "Veyra was alive years ago.",
            &aliases
        ));
        assert!(!has_named_character_alive_assertion(
            "Veyra was not alive.",
            &aliases
        ));
        assert!(!has_named_character_alive_assertion(
            "In the letter, Veyra said she was alive.",
            &aliases
        ));
    }

    #[test]
    fn action_framing_is_candidate_local() {
        let aliases = vec!["Veyra".to_string()];
        assert!(!has_named_character_action(
            "A letter from Veyra said she opened the door.",
            &aliases
        ));
        assert!(has_named_character_action(
            "A letter lay on the desk. Veyra opened the door.",
            &aliases
        ));
    }
}

#[cfg(test)]
mod state_selection_tests {
    use crate::sqlite::records::CharacterState;
    use chrono::Utc;
    use std::collections::BTreeMap;

    fn state(rowid: i64, status: &str) -> CharacterState {
        CharacterState {
            id: format!("character_state:{rowid:026}"),
            project_id: "project:test".into(),
            branch_id: "branch:test".into(),
            character_id: "character:test".into(),
            scene_id: None,
            book_number: 1,
            chapter_number: 1,
            scene_order: 0,
            emotional_state: BTreeMap::new(),
            goals: Vec::new(),
            status: vec![status.into()],
            notes: Vec::new(),
            source_summary: None,
            created_at: Utc::now(),
            rowid,
        }
    }

    #[test]
    fn newest_state_wins_at_the_same_position_in_either_load_order() {
        let empty = state(1, "alive");
        let dead = state(2, "life_status:dead");
        let position = (1, 1, 0);
        let forward = [empty.clone(), dead.clone()];
        let reverse = [dead.clone(), empty];
        for states in [forward.as_slice(), reverse.as_slice()] {
            let selected = crate::sqlite::repository::select_latest_character_state(
                states.iter(),
                "character:test",
                position,
            )
            .expect("a state");
            assert_eq!(selected.rowid, 2, "newest row must win: {selected:?}");
            assert_eq!(selected.status, vec!["life_status:dead".to_string()]);
        }
    }

    #[test]
    fn serialized_canon_state_preserves_rowid_recency() {
        let mut older = state(1, "alive");
        older.id = "character_state:zzzzzzzzzzzzzzzzzzzzzzzzzz".into();
        older.created_at = Utc::now();
        let mut newer = state(2, "life_status:dead");
        newer.id = "character_state:aaaaaaaaaaaaaaaaaaaaaaaaaa".into();
        newer.created_at = older.created_at;
        let json = serde_json::to_string(&[older, newer]).unwrap();
        let round_tripped: Vec<CharacterState> = serde_json::from_str(&json).unwrap();
        let selected = crate::sqlite::repository::select_latest_character_state(
            round_tripped.iter(),
            "character:test",
            (1, 1, 0),
        )
        .expect("a state");
        assert_eq!(
            selected.rowid, 2,
            "rowid must survive canon serialization even when created_at and id ordering disagree"
        );
        assert_eq!(selected.status, vec!["life_status:dead".to_string()]);
    }

    #[test]
    fn dead_state_severity_is_error_for_explicit_marker() {
        let finding = spindle_core::models::RetconFinding::DeadCharacterAct {
            character_id: "character:test".into(),
            character_name: "Test Character".into(),
            status: "life_status:dead".into(),
            message: "action after death".into(),
        };
        let issue = crate::sqlite::service::retcon_finding_to_consistency_issue(&finding);
        assert_eq!(issue.severity, "error");
    }
}
