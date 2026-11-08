use super::*;

pub fn static_tool_names() -> &'static BTreeSet<&'static str> {
    static NAMES: std::sync::OnceLock<BTreeSet<&'static str>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        all_tool_schemas()
            .iter()
            .map(|tool| match &tool.name {
                std::borrow::Cow::Borrowed(name) => *name,
                std::borrow::Cow::Owned(name) => Box::leak(name.clone().into_boxed_str()),
            })
            .collect()
    })
}

pub(super) fn all_tool_schemas() -> &'static [Tool] {
    static ALL_TOOLS: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    ALL_TOOLS.get_or_init(build_all_tools)
}

pub(super) fn build_all_tools() -> Vec<Tool> {
    vec![
        tool::<GetEditorialQueueInput, EditorialQueueOutput>(
            "get_editorial_queue",
            "Review reader concerns with chapter evidence, source freshness and author decisions. Default shows unresolved work.",
        ),
        tool::<DecideEditorialItemInput, EditorialItem>(
            "decide_editorial_item",
            "Accept, defer, resolve, dismiss or reopen an editorial concern against the reviewed source and decision revision. Accepted work enters drafting guidance; never changes prose or canon.",
        ),
        tool::<ReadEpisodeInput, ReadEpisodeOutput>(
            "read_episode",
            "Read an episode as the promised reader, continuing source-validated memory across runs and books. Reuses unchanged reads and reports gaps and stale memory.",
        ),
        tool::<GetSeriesStatusInput, SeriesStatusOutput>(
            "get_series_status",
            "Inspect the published cursor, draft and ready backlog, episode revisions, and changes since release.",
        ),
        tool::<PrepareEpisodeReleaseInput, EpisodeReleasePreview>(
            "prepare_episode_release",
            "Preview one chapter as an episode. Reports missing/stub scenes and returns a source hash for a local immutable release.",
        ),
        tool::<ReleaseEpisodeInput, EpisodeRelease>(
            "release_episode",
            "Record an immutable local episode snapshot from an unchanged preview. Corrections append revisions. Does not post to any platform.",
        ),
        tool::<GetEpisodeReleaseInput, EpisodeRelease>(
            "get_episode_release",
            "Read the exact prose and source metadata of an immutable episode release, including the previous revision id.",
        ),
        tool::<GetModelUsageInput, GetModelUsageOutput>(
            "get_model_usage",
            "Inspect persisted model calls, reported tokens, elapsed time and unknown usage. No prose or credentials are logged.",
        ),
        tool::<CreateProjectInput, CreateProjectOutput>(
            "create_project",
            "Create a project, book 1, and chapter 1",
        ),
        tool::<EmptyInput, ListProjectsOutput>(
            "list_projects",
            "List all projects with their record ids",
        ),
        tool::<SetActiveProjectInput, SetActiveProjectOutput>(
            "set_active_project",
            "Persist the default project and branch for this MCP session so follow-up tool calls can omit project_id and use the current branch by default",
        ),
        tool::<CreateBookInput, CreateBookOutput>(
            "create_book",
            "Create the next book within a project",
        ),
        tool::<CreateChapterInput, CreateChapterOutput>(
            "create_chapter",
            "Create the next chapter within a book using book_number or book_id; optional chapter_number may be any unused non-negative number (gaps are allowed, e.g. for restructures) and defaults to one past the current maximum",
        ),
        tool::<CreateBranchInput, CreateBranchOutput>(
            "create_branch",
            "Create a new project branch from the active or specified parent branch",
        ),
        tool::<SwitchBranchInput, SwitchBranchOutput>(
            "switch_branch",
            "Switch a project's active branch",
        ),
        tool::<SetNarratorVoiceInput, SetNarratorVoiceOutput>(
            "set_narrator_voice",
            "Set or clear the project's narrator-voice directive — the prose-level narration style (comedy density, pacing feel, interiority ratio, emotional register, chapter-ending style) that governs the whole reading experience and is distinct from per-character dialogue voice profiles. Enforced across scene context, the save-draft gate, the style_compliance validator, and the review's Target Reader persona",
        ),
        tool::<SetProjectCalendarInput, SetProjectCalendarOutput>(
            "set_project_calendar",
            "Define the project's in-world calendar (days per week, hours per day, months, days per year, epoch). Enables the chronology check and the in-world-time hard constraint; validated for internal consistency.",
        ),
        tool::<SetSceneClockInput, SetSceneClockOutput>(
            "set_scene_clock",
            "Place a scene on the in-world story clock (day_index, time_of_day, duration_days, precision), optionally marking temporal_mode (linear|flashback|flashforward|concurrent) and a parallel thread_key. Drives the between-scene chronology check, the intra-scene temporal_coherence check (duration_days sets the expected span; temporal_mode/precision suppress it), and the in-world-time context constraint that feeds the prior scene's end clock forward.",
        ),
        tool::<SetTimelineEventClockInput, SetTimelineEventClockOutput>(
            "set_timeline_event_clock",
            "Set the in-world time a timeline event occurs (distinct from its manuscript placement) so flashbacks and time-skips are oriented by story time.",
        ),
        tool::<SetCharacterBirthInput, SetCharacterBirthOutput>(
            "set_character_birth",
            "Anchor a character's birth on the story clock so their age can be derived at any story moment. day_index is a signed offset from the story's opening day, so births before the epoch are negative (e.g. -8913 for a character born ~24 years before day 1).",
        ),
        tool::<SetProjectQuantitySchemeInput, SetProjectQuantitySchemeOutput>(
            "set_project_quantity_scheme",
            "Declare a per-project quantity scheme for a measure (e.g. wealth, mana, a cultivation ladder): ordered currency denominations and ordered bands/tiers, plus an optional max_band_jump. Validated for internal consistency; fully opt-in. Money and progression systems reuse the same primitive.",
        ),
        tool::<CommitQuantityStateInput, CommitQuantityStateOutput>(
            "commit_quantity_state",
            "Record a stamped quantity reading for a subject's measure at a story position (band is the primary signal; amount/unit are optional). A change_reason marks a deliberate large jump as legitimate. Validated against the declared scheme when one exists. Manual path; authoring runs with a mining_policy stage quantity_change deltas that apply through this tool after ratification via decide_canon_deltas.",
        ),
        tool::<DeriveQuantitySchemeFromOverlayInput, DeriveQuantitySchemeFromOverlayOutput>(
            "derive_quantity_scheme_from_system_overlay",
            "Derive a quantity scheme from a system overlay so LitRPG / cultivation progression reuses band-monotonicity: the overlay's advancement_tiers become the scheme's ordered bands (measure = its progression_currency, else its name).",
        ),
        tool::<ScanScenePricesInput, ScanScenePricesOutput>(
            "scan_scene_prices",
            "Scan a scene's prose for '<number> <unit>' price mentions using the project's declared denominations + economy currencies (review-gated; nothing is auto-registered). Use the results to seed price-canon facts.",
        ),
        tool::<CreateSavePointInput, CreateSavePointOutput>(
            "create_save_point",
            "Create a save point on the active branch",
        ),
        tool::<RestoreSavePointInput, RestoreSavePointOutput>(
            "restore_save_point",
            "Restore the active branch from a save point snapshot",
        ),
        tool::<DiffBranchesInput, DiffBranchesOutput>(
            "diff_branches",
            "Compare two project branches across scenes, states, relationships, and pacing",
        ),
        tool::<CreateStyleProfileFromMarkdownInput, CreateStyleProfileFromMarkdownOutput>(
            "create_style_profile_from_markdown",
            "Derive a reusable project style profile from user-provided local Markdown files or folders. Persists metadata, hashes, metrics, and generated guidance, but no source text by default. The guidance is synthesized by the routed style_analyze model against an explicit schema; if the model returns no usable guidance the call fails loudly and creates no profile (use metrics_only=true to create a metrics-only profile deliberately).",
        ),
        tool::<CheckStyleProfileSourcesInput, CheckStyleProfileSourcesOutput>(
            "check_style_profile_sources",
            "Check the staleness of source files for a derived style profile compared to disk. Exposes metadata only.",
        ),
        tool::<PreviewRefreshStyleProfileInput, PreviewRefreshStyleProfileOutput>(
            "preview_refresh_style_profile",
            "Preview rebuilding/refreshing a style profile from source files without persisting. Exposes metadata only.",
        ),
        tool::<RefreshStyleProfileInput, RefreshStyleProfileOutput>(
            "refresh_style_profile",
            "Refresh/rebuild a style profile from current local sources, saving a new version linked to the parent.",
        ),
        tool::<ListStyleProfilesInput, ListStyleProfilesOutput>(
            "list_style_profiles",
            "List derived style profiles saved for a project.",
        ),
        tool::<GetStyleProfileInput, GetStyleProfileOutput>(
            "get_style_profile",
            "Retrieve a specific derived style profile by its profile_id.",
        ),
        tool::<ApplyStyleProfileInput, ApplyStyleProfileOutput>(
            "apply_style_profile",
            "Apply a derived style profile's guidance to a project's existing style contract surfaces (NarratorVoice, ReaderContract.style_notes, and style world rules), invalidating style-sensitive validator cache rows. Requires the profile to be ready and to carry application guidance; pass force=true to deliberately activate a NeedsReview or metrics-only profile — with no prose guidance, only the profile is activated and the project's narrator voice/style notes are left untouched.",
        ),
        tool::<PreviewApplyStyleProfileInput, PreviewApplyStyleProfileOutput>(
            "preview_apply_style_profile",
            "Preview applying a derived style profile's changes without mutating state.",
        ),
        tool::<ListStyleProfileApplicationsInput, ListStyleProfileApplicationsOutput>(
            "list_style_profile_applications",
            "List the history of applied style profiles and their rollback status.",
        ),
        tool::<RollbackStyleProfileApplicationInput, RollbackStyleProfileApplicationOutput>(
            "rollback_style_profile_application",
            "Rollback a previously applied style profile, reverting narrator voice, style notes, and conservatively reverting style world rules.",
        ),
        tool::<CheckStyleAgainstProfileInput, CheckStyleAgainstProfileOutput>(
            "check_style_against_profile",
            "Check a scene or raw text for style drift against a derived style profile.",
        ),
        tool::<PlanStyleRevisionInput, PlanStyleRevisionOutput>(
            "plan_style_revision",
            "Generate a non-mutating style revision plan for a target (scene, chapter, or raw text) against a derived style profile.",
        ),
        tool::<CompareStyleProfilesInput, CompareStyleProfilesOutput>(
            "compare_style_profiles",
            "Compare two derived style profiles to report metric deltas, guidance differences, and whether applying the second profile would likely change the project style materially.",
        ),
        tool::<ArchiveStyleProfileInput, ArchiveStyleProfileOutput>(
            "archive_style_profile",
            "Safer profile deletion/archiving. archived profiles cannot be active or default, but preserve audit history.",
        ),
        tool::<PreviewStyleRevisionPatchInput, PreviewStyleRevisionPatchOutput>(
            "preview_style_revision_patch",
            "Preview a style revision patch for a scene or chapter against a style profile, returning proposed revised text and unified diffs without persisting changes.",
        ),
        tool::<EvaluateStyleRevisionPatchInput, EvaluateStyleRevisionPatchOutput>(
            "evaluate_style_revision_patch",
            "Evaluate a proposed style revision patch for alignment improvement and risks without persisting changes.",
        ),
        tool::<ApplyStyleRevisionPatchInput, ApplyStyleRevisionPatchOutput>(
            "apply_style_revision_patch",
            "Apply proposed style revision patches to target scenes, saving drafts through the standard writing pipeline, invalidating caches, and writing an audit trail.",
        ),
        tool::<ListStyleRevisionPatchAuditsInput, ListStyleRevisionPatchAuditsOutput>(
            "list_style_revision_patch_audits",
            "List style revision patch audits for a project.",
        ),
        tool::<RollbackStyleRevisionPatchInput, RollbackStyleRevisionPatchOutput>(
            "rollback_style_revision_patch",
            "Roll back an applied style revision patch, restoring prior prose versions using scene version history.",
        ),
        tool::<MergeBranchInput, MergeBranchOutput>(
            "merge_branch",
            "Merge a source branch into a target branch for branch-aware story records",
        ),
        tool::<ReviseSceneInput, ReviseSceneOutput>(
            "revise_scene",
            "Revise a scene on a non-main branch and flag downstream invalidation impact",
        ),
        tool::<GenerateAlternativesInput, GenerateAlternativesOutput>(
            "generate_alternatives",
            "Generate branch-backed alternative scene drafts from shared context",
        ),
        tool::<CompareAlternativesInput, CompareAlternativesOutput>(
            "compare_alternatives",
            "Compare generated alternative branches and rank them heuristically",
        ),
        tool::<SelectAlternativeInput, SelectAlternativeOutput>(
            "select_alternative",
            "Select an alternative branch and merge it into main",
        ),
        tool::<ListRevisionMarkersInput, ListRevisionMarkersOutput>(
            "list_revision_markers",
            "List persisted revision markers for a scene on the active branch",
        ),
        tool::<ResolveRevisionMarkerInput, ResolveRevisionMarkerOutput>(
            "resolve_revision_marker",
            "Mark a persisted revision marker as resolved",
        ),
        tool::<CreateCharacterInput, CreateCharacterOutput>(
            "create_character",
            "Create a character with profiles and baseline state",
        ),
        tool::<CreateLocationInput, CreateLocationOutput>(
            "create_location",
            "Create a location with optional initial world state; accepts type as an alias for kind and infers a kind when omitted",
        ),
        tool::<CreateFactionInput, CreateFactionOutput>(
            "create_faction",
            "Create a faction entity",
        ),
        tool::<CreateReligionInput, CreateReligionOutput>(
            "create_religion",
            "Create a religion entity",
        ),
        tool::<CreateEconomyInput, CreateEconomyOutput>(
            "create_economy",
            "Create an economy entity",
        ),
        tool::<CreateTermInput, CreateTermOutput>("create_term", "Create a glossary term"),
        tool::<BatchCreateTermsInput, BatchCreateTermsOutput>(
            "batch_create_terms",
            "Create multiple glossary terms in one call",
        ),
        tool::<CreateRelationshipInput, CreateRelationshipOutput>(
            "create_relationship",
            "Create a directed relationship between two characters",
        ),
        tool::<CreateWorldRuleInput, CreateWorldRuleOutput>(
            "create_world_rule",
            "Create a project world rule",
        ),
        tool::<UpdateWorldRuleInput, UpdateWorldRuleOutput>(
            "update_world_rule",
            "Update a world rule by record id",
        ),
        tool::<BatchSetCharacterVoiceProfilesInput, BatchSetCharacterVoiceProfilesOutput>(
            "batch_set_character_voice_profiles",
            "Replace multiple character voice profiles in one call",
        ),
        tool::<UpdateEntityInput, UpdateEntityOutput>(
            "update_entity",
            "Update a supported entity by record id. Character aliases are settable via changes {\"aliases\": [...]}. Renaming a character (changes {\"name\": ...}) is a controlled operation: pass allow_rename: true to move the name's uniqueness key, keep the old name as an alias, refresh the search index, and get a rename_report listing scenes/facts/knowledge/arcs still referencing the old name. Without allow_rename the rename is rejected. For entity_type=project, the reader contract is updatable via changes {\"reader_contract\": {...}} or the sub-fields {\"promise\", \"style_notes\", \"boundaries\"} (partial merge; unset fields are preserved); if the edit leaves the contract's word-count target contradicting the narrator voice or a style world rule, the response warnings flag it",
        ),
        tool::<ArchiveEntityInput, ArchiveEntityOutput>(
            "archive_entity",
            "Archive a supported entity by record id",
        ),
        tool::<CreatePlotLineInput, CreatePlotLineOutput>("create_plot_line", "Create a plot line"),
        tool::<CreateConflictInput, CreateConflictOutput>(
            "create_conflict",
            "Create a conflict record",
        ),
        tool::<CreateThemeInput, CreateThemeOutput>("create_theme", "Create a theme record"),
        tool::<CreateMotifInput, CreateMotifOutput>("create_motif", "Create a motif record"),
        tool::<BatchCreateMotifsInput, BatchCreateMotifsOutput>(
            "batch_create_motifs",
            "Create multiple motif records in one call",
        ),
        tool::<CreateNarrativePromiseInput, CreateNarrativePromiseOutput>(
            "create_narrative_promise",
            "Create a narrative promise",
        ),
        tool::<BatchCreateNarrativePromisesInput, BatchCreateNarrativePromisesOutput>(
            "batch_create_narrative_promises",
            "Create multiple narrative promises in one call",
        ),
        tool::<UpdatePromiseStatusInput, UpdatePromiseStatusOutput>(
            "update_promise_status",
            "Advance a narrative promise lifecycle. Manual path; authoring runs with a mining_policy stage promise_payoff_candidate/promise_reinforced deltas that apply through this tool after ratification via decide_canon_deltas.",
        ),
        tool::<CreateCharacterArcInput, CreateCharacterArcOutput>(
            "create_character_arc",
            "Create a character arc and pacing tracker",
        ),
        tool::<UpdateArcMilestoneInput, UpdateArcMilestoneOutput>(
            "update_arc_milestone",
            "Update a single character-arc milestone by label: move its placement and/or stamp reached_at (e.g. after a chapter renumber or when the milestone lands). Only the supplied fields change; description and unlocks are preserved. Read milestones via get_entity(table=\"character_arc\") or get_character_snapshot",
        ),
        tool::<CreateFutureKnowledgeInput, CreateFutureKnowledgeOutput>(
            "create_future_knowledge",
            "Record future knowledge held by a character",
        ),
        tool::<CreateTimelineEventInput, CreateTimelineEventOutput>(
            "create_timeline_event",
            "Record a timeline event for time-aware stories",
        ),
        tool::<CreateTemporalInterventionInput, CreateTemporalInterventionOutput>(
            "create_temporal_intervention",
            "Track a time-travel intervention between timeline events",
        ),
        tool::<CreateSystemOverlayInput, CreateSystemOverlayOutput>(
            "create_system_overlay",
            "Create a LitRPG or cultivation system overlay",
        ),
        tool::<RunDualPersonaReviewInput, RunDualPersonaReviewOutput>(
            "run_dual_persona_review",
            "Run a literary and craft review loop for a branch scene",
        ),
        tool::<CreatePacingConfigInput, CreatePacingConfigOutput>(
            "create_pacing_config",
            "Create pacing configuration for a project",
        ),
        tool::<CreatePacingCurveInput, CreatePacingCurveOutput>(
            "create_pacing_curve",
            "Create pacing curve for a book",
        ),
        tool::<SetArcPacingConstraintsInput, SetArcPacingConstraintsOutput>(
            "set_arc_pacing_constraints",
            "Set pacing constraints for a character arc",
        ),
        tool::<PlanChapterInput, PlanChapterOutput>("plan_chapter", "Create a chapter plan"),
        tool::<AnnotateSceneBeatsInput, AnnotateSceneBeatsOutput>(
            "annotate_scene_beats",
            "Annotate structural beats for a scene. Manual path; authoring runs with a mining_policy stage beat_annotation deltas that apply through this tool after ratification via decide_canon_deltas.",
        ),
        tool::<SaveSummaryInput, SaveSummaryOutput>(
            "save_summary",
            "Save a chapter summary using chapter entity_id/chapter_id or explicit book_number and chapter_number",
        ),
        tool::<SetBookOutlineInput, SetBookOutlineOutput>(
            "set_book_outline",
            "Set or replace a book outline on the active branch using book_id or book_number",
        ),
        tool::<SetChapterOutlineInput, SetChapterOutlineOutput>(
            "set_chapter_outline",
            "Set or replace a chapter outline on the active branch using chapter_id/entity_id or explicit book_number and chapter_number",
        ),
        tool::<CheckConsistencyInput, CheckConsistencyOutput>(
            "check_consistency",
            "Run a structured consistency audit (includes scene_divergence and canonical_fact_consistency checks)",
        ),
        tool::<RegisterCanonicalFactInput, RegisterCanonicalFactOutput>(
            "register_canonical_fact",
            "Register a canonical story fact (pull result, stat change, item, ability) for contradiction detection. scene_id is optional: omit it to register a fact decided during planning but not yet dramatised (placed by book_number/chapter_number only), then attach it later with bind_canonical_fact_to_scene",
        ),
        tool::<ExtractCanonicalFactsFromSceneInput, ExtractCanonicalFactsFromSceneOutput>(
            "extract_canonical_facts_from_scene",
            "Extract proposed typed canonical facts from committed scene prose without registering them",
        ),
        tool::<MigrateCanonicalFactInput, MigrateCanonicalFactOutput>(
            "migrate_canonical_fact",
            "Promote a legacy_untyped canonical fact into a typed canonical fact and supersede the legacy row",
        ),
        tool::<BindCanonicalFactToSceneInput, BindCanonicalFactToSceneOutput>(
            "bind_canonical_fact_to_scene",
            "Attach a planned-and-pending canonical fact (registered without a scene_id) to the scene that dramatises it. The fact and scene must share a project and branch; placement is left untouched",
        ),
        tool::<SearchBibleInput, SearchBibleOutput>(
            "search_bible",
            "Search project records by meaning, exact text, or fuzzy match",
        ),
        tool::<FindScenesReferencingInput, FindScenesReferencingOutput>(
            "find_scenes_referencing",
            "Find up to 100 active-branch scenes that reference a subject record id or literal phrase",
        ),
        tool::<RebuildSearchIndexInput, RebuildSearchIndexOutput>(
            "rebuild_search_index",
            "Rebuild semantic search embeddings for a project",
        ),
        tool::<BackfillSceneSourceOffsetsInput, BackfillSceneSourceOffsetsOutput>(
            "backfill_scene_source_offsets",
            "Recompute scene_source_link offsets with the import slicer for one project branch",
        ),
        tool::<PullChapterFromFileInput, PullReport>(
            "pull_chapter_from_file",
            "Import chapter scene text from a source file into active-branch scenes",
        ),
        tool::<PushChapterToFileInput, PushReport>(
            "push_chapter_to_file",
            "Export active-branch chapter scene text to a source file and store source offsets",
        ),
        tool::<ConfigureAgentsInput, ConfigureAgentsOutput>(
            "configure_agents",
            "Reload model agent and route configuration from spindle.toml",
        ),
        tool::<EmptyInput, ListAgentsOutput>(
            "list_agents",
            "List configured model agents and their route assignments",
        ),
        tool::<InitGrokSkillsInput, InitGrokSkillsOutput>(
            "init_grok_skills",
            "Initialize Grok-compatible Spindle skill adapters. By default installs into ~/.grok/skills/ (global). Pass global=false + target_dir if you want repo-scoped adapters instead. This makes all bible://skills/* (scene-writer, character-creator, etc.) work as first-class skills in Grok.",
        ),
        tool::<EmptyInput, ListSkillsOutput>(
            "list_skills",
            "List the embedded Spindle workflow skills and craft references. Tool equivalent of browsing bible://skills/* and bible://references/*; use get_skill / get_reference to read one.",
        ),
        tool::<GetSkillInput, GetSkillOutput>(
            "get_skill",
            "Return the full content of an embedded Spindle skill. Tool equivalent of reading bible://skills/{name}; call this to load a workflow such as scene-writer or character-creator.",
        ),
        tool::<GetReferenceInput, GetReferenceOutput>(
            "get_reference",
            "Return the full content of an embedded craft reference. Tool equivalent of reading bible://references/{name}.",
        ),
        tool::<TestAgentInput, TestAgentOutput>(
            "test_agent",
            "Send a test prompt through a configured model agent",
        ),
        tool::<ContinueGenerationInput, ContinueGenerationOutput>(
            "continue_generation",
            "Continue a model generation and return a server-side generation receipt",
        ),
        tool::<ReviseGenerationInput, ReviseGenerationOutput>(
            "revise_generation",
            "Revise a server-side generation through the same explicit-capable route and return a new receipt",
        ),
        tool::<ResearchQueryInput, ResearchQueryOutput>(
            "research_query",
            "Research a factual question using Gemini, grounded in project context from the Bible",
        ),
        tool::<ResearchAddSourceInput, ResearchAddSourceOutput>(
            "research_add_source",
            "Create new research source metadata and optional summary in the project-local library",
        ),
        tool::<ResearchAddNoteInput, ResearchAddNoteOutput>(
            "research_add_note",
            "Attach a freeform note and optional quote/locator to a research source",
        ),
        tool::<ResearchAddClaimInput, ResearchAddClaimOutput>(
            "research_add_claim",
            "Store a distilled factual claim with confidence, topic, location, time period, and tags",
        ),
        tool::<ResearchSearchInput, ResearchSearchOutput>(
            "research_search",
            "Search the project-local research library for matching sources, notes, and claims",
        ),
        tool::<ResearchPackForSceneInput, ResearchPackForSceneOutput>(
            "research_pack_for_scene",
            "Retrieve a compact, budget-aware packet of relevant research for a scene or chapter",
        ),
        tool::<ResearchUsageForSceneInput, ResearchUsageForSceneOutput>(
            "research_usage_for_scene",
            "Retrieve the recorded research usage history for a drafted scene",
        ),
        tool::<ResearchIngestReportInput, ResearchIngestReportOutput>(
            "research_ingest_report",
            "Ingest and parse a text-based research report, creating sources, notes, and claims in the project-local database",
        ),
        tool::<ResearchPlanForSceneInput, ResearchPlanForSceneOutput>(
            "research_plan_for_scene",
            "Determine missing research, suggest queries, and check if a scene is blocked on research",
        ),
        tool::<ExportEpubInput, ExportEpubOutput>(
            "export_epub",
            "Export a project, single book, or inclusive chapter range within a book as an EPUB file",
        ),
        tool::<PreflightBookExportInput, PreflightBookExportOutput>(
            "preflight_book_export",
            "Validate a project, single book, or inclusive chapter range within a book for EPUB export and return blocking issues or warnings before writing a file",
        ),
        tool::<CompileManuscriptInput, CompileManuscriptOutput>(
            "compile_manuscript",
            "Assemble the committed prose of a book (or an inclusive chapter range within it) on the active branch into one Markdown read-so-far document, with per-chapter and per-scene headings; planned-but-undrafted scenes render an explicit placeholder and are reported in missing_scenes. Optionally writes the Markdown to the project's workspace artifacts directory.",
        ),
        tool::<ExportRecapInput, ExportRecapOutput>(
            "export_recap",
            "Assemble a spoiler-bounded, reader-facing \"previously on\" recap of a book up to (and including) through_chapter on the active branch: a story-so-far section from chapter summaries at/under the cursor, a \"paid off\" section of resolved promises, and a \"questions still hanging\" section of open promises. Pure read model — no model calls. A secret canonical fact (and any line naming it) is withheld unless a reader-visible reveal has been placed at or before the cursor. Optionally writes the Markdown to the project's workspace artifacts directory.",
        ),
        tool::<ExportSeriesBibleInput, ExportSeriesBibleOutput>(
            "export_series_bible",
            "Assemble a spoiler-bounded, reader-facing series bible on the active branch as of an optional cursor (through, absent = whole project): character pages with state as-of the cursor and relationship bands, locations, a sorted glossary of terms, and factions/religions when present. Pure read model — no model calls. A secret canonical fact (and any line naming it) is withheld unless a reader-visible reveal has been placed at or before the cursor. Optionally writes the Markdown to the project's workspace artifacts directory.",
        ),
        tool::<MineSceneCanonInput, MineSceneCanonOutput>(
            "mine_scene_canon",
            "Mine one committed scene's prose into proposed canon deltas (staged for operator ratification, never auto-applied). One rating-gated model call; malformed model output is rejected and evidence must appear verbatim in the prose. Re-mining a scene supersedes its prior staged deltas. Returns the staged deltas, discard/supersede counts, and a status of staged, skipped (no cleared route or empty scene), or model_output_rejected.",
        ),
        tool::<ListCanonDeltasInput, ListCanonDeltasOutput>(
            "list_canon_deltas",
            "List the canon deltas mined for a project's active branch (the ratify queue). Filter by status (staged | applied | rejected | superseded) and by provenance — either one scene_id OR a chapter_range { book_number, start, end } (mutually exclusive). Results carry each delta's class, payload, verbatim evidence quote, confidence, and status, in deterministic order. Read this before deciding — never bulk-apply without reading the evidence.",
        ),
        tool::<DecideCanonDeltasInput, DecideCanonDeltasOutput>(
            "decide_canon_deltas",
            "Ratify a batch of staged canon deltas: each decision is apply or reject, with an optional edit (a corrected payload applied AND recorded) and an optional operator note (echoed in the output; not persisted). ALL decisions are pre-flighted before ANY write — a bad payload, dangling target, or a decision on an already-decided row (decisions are final) aborts the whole call with zero writes. Applies dispatch to the class's existing write tool (register_canonical_fact, update_promise_status, update_relationship, commit_character_state, record_knowledge, annotate_scene_beats, commit_quantity_state, update_entity for conflict/arc columns, or create_character/location/term). Returns a per-decision outcome (applied | rejected | failed | not_reached) with any created record id.",
        ),
        tool::<ReplanChapterInput, ReplanChapterOutput>(
            "replan_chapter",
            "Audit a book's not-yet-drafted chapter plans against the realized reality one just-summarized chapter established, and stage plan-amendment proposals for operator ratification (never auto-applied — ADR 0003 D5). Non-prose-bearing: the differ reads summaries + metadata only (no scene prose), so no rating clearance applies; on a missing route it falls to review then skips honestly, and it never blocks the run. Re-running for the same source chapter supersedes its prior staged amendments. Returns the staged amendments, discard/supersede counts, and a status of staged, no_summary, no_targets, skipped, or model_output_rejected.",
        ),
        tool::<ListPlanAmendmentsInput, ListPlanAmendmentsOutput>(
            "list_plan_amendments",
            "List the plan amendments staged for a project's active branch (the living-outline ratify queue). Filter by status (staged | applied | rejected | superseded) and by provenance (book_number + source_chapter together — the summarized chapter that triggered a replan pass). Each amendment carries its class, the future target_chapter (null for promise_followup), the typed payload, the replanner's rationale (ids/summaries, no prose), confidence, status, and any prior_state snapshot, in deterministic order. Read this — and the rationale — before deciding; never bulk-apply blind.",
        ),
        tool::<DecidePlanAmendmentsInput, DecidePlanAmendmentsOutput>(
            "decide_plan_amendments",
            "Ratify a batch of staged plan amendments (the living outline). Each decision is apply or reject, with an optional edit (a corrected payload applied AND recorded) and an optional operator note (echoed, not persisted). ALL decisions are pre-flighted before ANY write — a bad payload, a target chapter that already has drafted scenes (the immutability guard: drafted reality is never rewritten), a dangling scene_order/incoherent reorder, or a decision on an already-decided row (decisions are final) aborts the whole call with zero writes. Applies replay through the existing plan write path (plan_chapter for the chapter classes, create_narrative_promise for promise_followup), snapshot the prior plan into prior_state, and bump plan_revision. Returns a per-decision outcome (applied | rejected | failed | not_reached) with any created record id.",
        ),
        tool::<ExportBibleInput, ExportBibleOutput>(
            "export_bible",
            "Export a full project backup as JSON, including branch data",
        ),
        tool::<GetWriterStateInput, WriterStateEnvelope>(
            "get_writer_state",
            "Return a branch-aware re-anchor packet with current cursor state, constraints, subjects, recent scenes, overlays, divergence warnings, and recent activity",
        ),
        tool::<GetSceneContextInput, SceneContextEnvelope>(
            "get_scene_context",
            "Assemble standards, novel, and scene context using chapter_id or explicit book_number and chapter_number",
        ),
        tool::<GetEntityInput, EntitySubjectSnapshot>(
            "get_entity",
            "Resolve one entity by table and record id as a polymorphic subject snapshot",
        ),
        tool::<FindEntityInput, FindEntityOutput>(
            "find_entity",
            "Resolve entities by name or alias with ExactName/SemanticMatch confidence",
        ),
        tool::<GetCharacterSnapshotInput, CharacterSnapshotOutput>(
            "get_character_snapshot",
            "Resolve one character snapshot and promote voice profile, current state, and recent appearances",
        ),
        tool::<SetCharacterVoiceProfileInput, SetCharacterVoiceProfileOutput>(
            "set_character_voice_profile",
            "Set a character voice profile and append a session activity entry summarizing the change",
        ),
        tool::<GetChapterBriefingInput, GetChapterBriefingOutput>(
            "get_chapter_briefing",
            "Assemble a compact pre-write briefing with continuity sheets, recent summaries, chapter plans, and lean scene context",
        ),
        tool::<ListChapterScenesInput, ListChapterScenesOutput>(
            "list_chapter_scenes",
            "List the active-branch scenes in one chapter with canonical order, summary-first-line, word count, and canonical-fact flags",
        ),
        tool::<ListBookChaptersInput, ListBookChaptersOutput>(
            "list_book_chapters",
            "List the active-branch chapters in one book with nested ordered scene spines",
        ),
        tool::<SaveSceneDraftInput, SaveSceneDraftOutput>(
            "save_scene_draft",
            "Create or update a scene draft using chapter_id or explicit book_number and chapter_number; accepts content as an alias for full_text",
        ),
        tool::<MoveSceneInput, MoveSceneOutput>(
            "move_scene",
            "Move an active-branch scene only when its move audit is clear; leaves a gap at the source position",
        ),
        tool::<DeleteSceneInput, DeleteSceneOutput>(
            "delete_scene",
            "Delete an active-branch scene only when its dependency audit is clear; leaves a gap in scene_order. Address by scene_id (preferred — unambiguous even when duplicates or orphans share a position) or by book_number/chapter_number/scene_order",
        ),
        tool::<OperatorDeleteSceneInput, OperatorDeleteSceneOutput>(
            "operator_delete_scene",
            "Delete an active-branch scene after removing scene_source_link records and invalidating stale chapter_plan/chapter_summary artifacts, but only when no other blockers or semantic risks remain. Accepts scene_id for unambiguous addressing, like delete_scene",
        ),
        tool::<ListSceneVersionsInput, ListSceneVersionsOutput>(
            "list_scene_versions",
            "List saved historical versions for a scene",
        ),
        tool::<RestoreSceneVersionInput, RestoreSceneVersionOutput>(
            "restore_scene_version",
            "Restore a scene from one of its saved historical versions",
        ),
        tool::<CommitSceneChangesInput, CommitSceneChangesOutput>(
            "commit_scene_changes",
            "Best-effort batch commit of scene character states, canonical facts, and relationship updates; accepts shorthand summary entries for state notes, canonical fact summaries, and relationship summaries",
        ),
        tool::<CommitCharacterStateInput, CommitCharacterStateOutput>(
            "commit_character_state",
            "Append a character state snapshot from a saved scene. Manual path; authoring runs with a mining_policy stage character_state deltas that apply through this tool after ratification via decide_canon_deltas.",
        ),
        tool::<UpdateRelationshipInput, UpdateRelationshipOutput>(
            "update_relationship",
            "Update trust and tension for one directed relationship. Manual path; authoring runs with a mining_policy stage relationship_shift deltas that apply through this tool after ratification via decide_canon_deltas.",
        ),
        tool::<ImportManuscriptInput, ImportManuscriptOutput>(
            "import_manuscript",
            "Create an import session and persist normalized manuscript structure",
        ),
        tool::<ImportStatusInput, ImportStatusOutput>(
            "import_status",
            "Read the current state of an import session",
        ),
        tool::<ImportExtractEntitiesInput, ImportExtractEntitiesOutput>(
            "import_extract_entities",
            "Extract candidate entities from imported manuscript scenes",
        ),
        tool::<ImportConsolidateEntitiesInput, ImportConsolidateEntitiesOutput>(
            "import_consolidate_entities",
            "Consolidate imported entity mentions into canonical clusters",
        ),
        tool::<ImportAnalyzeCharacterInput, ImportAnalyzeCharacterOutput>(
            "import_analyze_character",
            "Build imported character dossiers from consolidated clusters",
        ),
        tool::<ImportExtractWorldInput, ImportExtractWorldOutput>(
            "import_extract_world",
            "Build imported world dossier candidates",
        ),
        tool::<ImportAnalyzeNarrativeInput, ImportAnalyzeNarrativeOutput>(
            "import_analyze_narrative",
            "Build imported narrative dossier candidates",
        ),
        tool::<ImportComputeFinalStateInput, ImportComputeFinalStateOutput>(
            "import_compute_final_state",
            "Compute the imported manuscript continuation point and final state",
        ),
        tool::<ImportHydrateBibleInput, ImportHydrateBibleOutput>(
            "import_hydrate_bible",
            "Hydrate an import session into canonical story records",
        ),
        tool::<ImportApplyReviewDecisionsInput, ImportApplyReviewDecisionsOutput>(
            "import_apply_review_decisions",
            "Resolve persisted import review items and update session readiness",
        ),
        tool::<RecordKnowledgeInput, RecordKnowledgeOutput>(
            "record_knowledge",
            "Record canonical knowledge for a character. Remains the direct reveal path for circle-of-trust expansion (with secret_of_fact_id + learned_at) alongside the save-draft continuity package and the mined knowledge_learned delta — it is not superseded; those flows apply through this same tool.",
        ),
        tool::<RecordNoteInput, RecordNoteOutput>(
            "record_note",
            "Append a freeform note to branch session activity",
        ),
        tool::<UpdateWriterPositionInput, WriterPosition>(
            "update_writer_position",
            "Persist a branch writer cursor position without saving a draft",
        ),
        tool::<AuthoringPrepareRunInput, AuthoringPrepareRunOutput>(
            "authoring_prepare_run",
            "Inspect the requested chapter range for the project and produce a readiness report before starting",
        ),
        tool::<AuthoringStartRunInput, AuthoringStartRunOutput>(
            "authoring_start_run",
            "Start a new authoring run for the project and book chapter range",
        ),
        tool::<AuthoringStatusInput, AuthoringStatusOutput>(
            "authoring_status",
            "Retrieve progress, next action, and status of the active authoring run",
        ),
        tool::<AuthoringExecuteNextInput, AuthoringExecuteNextOutput>(
            "authoring_execute_next",
            "Execute exactly one safe authoring action for the active run",
        ),
        tool::<AuthoringSaveSceneDraftInput, AuthoringSaveSceneDraftOutput>(
            "authoring_save_scene_draft",
            "Save a host-drafted authoring scene with its required structured continuity package",
        ),
        tool::<AuthoringRecordCheckpointAuditInput, AuthoringRecordCheckpointAuditOutput>(
            "authoring_record_checkpoint_audit",
            "Attach a completed deep consistency audit to a pending authoring checkpoint",
        ),
        tool::<AuthoringReviewCheckpointInput, AuthoringReviewCheckpointOutput>(
            "authoring_review_checkpoint",
            "Mark a checkpoint reviewed after deep consistency and sampled reviews are complete; rejects unresolved/fix-later directives",
        ),
        tool::<AuthoringResolveBlockInput, AuthoringResolveBlockOutput>(
            "authoring_resolve_block",
            "Clear a blocked scene/run and advance it manually",
        ),
        tool::<AuthoringCancelRunInput, AuthoringCancelRunOutput>(
            "authoring_cancel_run",
            "Pause/cancel the active authoring run without deleting progress",
        ),
    ]
}
