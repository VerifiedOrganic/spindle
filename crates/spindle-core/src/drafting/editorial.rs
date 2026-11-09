//! Durable editorial-controller contracts: hybrid host tasks, agent parity,
//! admission reservations, and prose-free attribution events.
//!
//! These are pure data and policy contracts. Persistence, token counting, model
//! dispatch, and prompt rendering live in `spindle-adapters`; the harness only
//! reads the resolved settings and controller state. No contract here carries
//! model prose into telemetry: attribution records ids, hashes, enums and counts.

use crate::drafting::{
    CritiqueDirective, DraftingPacketV1, EditorialPolicy, EditorialStage, PacketSource, TargetWords,
};
use crate::models::ContentRating;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const EDITORIAL_CONTROLLER_SCHEMA_VERSION: &str = "editorial_controller_v1";
pub const HOST_EDITORIAL_TASK_SCHEMA_VERSION: &str = "host_editorial_task_v1";
pub const EDITORIAL_ATTRIBUTION_SCHEMA_VERSION: &str = "editorial_attribution_v1";
pub const RESOLVED_EDITORIAL_SETTINGS_SCHEMA_VERSION: &str = "resolved_editorial_settings_v1";

/// Stable prompt-template identifiers. Every editorial dispatch sends both the
/// id and the version on its model-call attribution row (V0047).
pub const TEMPLATE_PACKET_DRAFT: &str = "draft.packet";
pub const TEMPLATE_CRITIC: &str = "review.critic_v1";
pub const TEMPLATE_REVISION: &str = "draft.revise_v1";
pub const TEMPLATE_JUDGE: &str = "review.pairwise_judge_v1";
pub const TEMPLATE_LINE_EDIT: &str = "draft.line_edit_v1";
pub const TEMPLATE_ACCEPTED_EXTRACTION: &str = "extract.accepted_v1";
pub const TEMPLATE_VERSION_V1: &str = "v1";

/// Per-scene hard ceiling (`docs/drafting-and-revision-design.md`).
pub const DEFAULT_SCENE_CALL_CAP: u32 = 12;
pub const DEFAULT_SCENE_TOKEN_CAP: usize = 210_000;
pub const DEFAULT_SCENE_MILLIS_CAP: u64 = 720_000;
pub const DEFAULT_SCENE_USD_MICROS_CAP: u64 = 2_000_000;
/// Per-run hard ceiling shared by scenes, chapter, book and checkpoint work.
pub const DEFAULT_RUN_CALL_CAP: u32 = 200;
pub const DEFAULT_RUN_TOKEN_CAP: usize = 3_000_000;
pub const DEFAULT_RUN_MILLIS_CAP: u64 = 5_400_000;
pub const DEFAULT_RUN_USD_MICROS_CAP: u64 = 20_000_000;

pub const DEFAULT_EDITORIAL_MAX_ROUNDS: u32 = 1;
pub const HARD_EDITORIAL_MAX_ROUNDS: u32 = 2;

/// Model tokenizer/profile identity persisted with a resolved plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialPricingV1 {
    /// Micro-USD per one million input tokens.
    pub input_usd_micros_per_million: u64,
    /// Micro-USD per one million output tokens.
    pub output_usd_micros_per_million: u64,
}

impl EditorialPricingV1 {
    pub fn cost_micros(&self, input_tokens: usize, output_tokens: usize) -> u64 {
        let input =
            (input_tokens as u128) * (self.input_usd_micros_per_million as u128) / 1_000_000;
        let output =
            (output_tokens as u128) * (self.output_usd_micros_per_million as u128) / 1_000_000;
        u64::try_from(input.saturating_add(output)).unwrap_or(u64::MAX)
    }
}

/// A hard ceiling on one scope (scene or run). All four limits apply together;
/// the earliest exhaustion wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialBudgetCaps {
    pub max_calls: u32,
    pub max_tokens: usize,
    pub max_millis: u64,
    pub max_usd_micros: u64,
}

impl EditorialBudgetCaps {
    pub const fn scene_default() -> Self {
        Self {
            max_calls: DEFAULT_SCENE_CALL_CAP,
            max_tokens: DEFAULT_SCENE_TOKEN_CAP,
            max_millis: DEFAULT_SCENE_MILLIS_CAP,
            max_usd_micros: DEFAULT_SCENE_USD_MICROS_CAP,
        }
    }

    pub const fn run_default() -> Self {
        Self {
            max_calls: DEFAULT_RUN_CALL_CAP,
            max_tokens: DEFAULT_RUN_TOKEN_CAP,
            max_millis: DEFAULT_RUN_MILLIS_CAP,
            max_usd_micros: DEFAULT_RUN_USD_MICROS_CAP,
        }
    }

    pub const fn unlimited() -> Self {
        Self {
            max_calls: u32::MAX,
            max_tokens: usize::MAX,
            max_millis: u64::MAX,
            max_usd_micros: u64::MAX,
        }
    }
}

/// Charged usage. Unknown host/provider usage is *not* zero: a reservation
/// whose usage is unknown is charged at its configured maximum instead.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialBudgetUsage {
    pub calls: u32,
    pub tokens: usize,
    pub millis: u64,
    pub usd_micros: u64,
}

impl EditorialBudgetUsage {
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            calls: self.calls.saturating_add(other.calls),
            tokens: self.tokens.saturating_add(other.tokens),
            millis: self.millis.saturating_add(other.millis),
            usd_micros: self.usd_micros.saturating_add(other.usd_micros),
        }
    }

    pub fn exceeds(&self, caps: &EditorialBudgetCaps) -> Option<EditorialBudgetExhaustion> {
        if self.calls > caps.max_calls {
            return Some(EditorialBudgetExhaustion::Calls);
        }
        if self.tokens > caps.max_tokens {
            return Some(EditorialBudgetExhaustion::Tokens);
        }
        if self.millis > caps.max_millis {
            return Some(EditorialBudgetExhaustion::Duration);
        }
        if self.usd_micros > caps.max_usd_micros {
            return Some(EditorialBudgetExhaustion::Cost);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EditorialBudgetExhaustion {
    Calls,
    Tokens,
    Duration,
    Cost,
}

/// One editorial pass. Reservations are exact proposed calls, never permission
/// to exceed a cap.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EditorialActionKind {
    Draft,
    DeterministicCheck,
    Critique,
    Revise,
    Judge,
    LineEdit,
    Extract,
}

impl EditorialActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::DeterministicCheck => "deterministic_check",
            Self::Critique => "critique",
            Self::Revise => "revise",
            Self::Judge => "judge",
            Self::LineEdit => "line_edit",
            Self::Extract => "extract",
        }
    }

    pub fn prompt_template_id(self) -> Option<&'static str> {
        match self {
            Self::Draft => Some(TEMPLATE_PACKET_DRAFT),
            Self::Critique => Some(TEMPLATE_CRITIC),
            Self::Revise => Some(TEMPLATE_REVISION),
            Self::Judge => Some(TEMPLATE_JUDGE),
            Self::LineEdit => Some(TEMPLATE_LINE_EDIT),
            Self::Extract => Some(TEMPLATE_ACCEPTED_EXTRACTION),
            Self::DeterministicCheck => None,
        }
    }

    pub fn is_model_pass(self) -> bool {
        !matches!(self, Self::DeterministicCheck)
    }

    /// Calls proposed by this action, including required judge passes and the
    /// critic's bounded one-shot schema/anchor repair.
    pub fn proposed_calls(self) -> u32 {
        match self {
            Self::Draft | Self::Revise | Self::LineEdit | Self::Extract => 1,
            Self::Critique => 2,
            Self::Judge => 2,
            Self::DeterministicCheck => 0,
        }
    }

    /// Token reservation: input limit + output reserve, matching the per-pass
    /// ceilings in the design doc (input includes system framing).
    pub fn proposed_tokens(self) -> usize {
        match self {
            Self::Draft => 15_512,
            Self::Critique => 35_000,
            Self::Revise => 21_000,
            Self::Judge => 34_000,
            Self::LineEdit => 21_000,
            Self::Extract => 10_000,
            Self::DeterministicCheck => 0,
        }
    }

    /// Wall-clock reservation: hard timeouts, not p50 promises.
    pub fn proposed_millis(self) -> u64 {
        match self {
            Self::Draft | Self::Revise | Self::LineEdit => 90_000,
            Self::Critique => 90_000,
            Self::Extract => 45_000,
            Self::Judge => 90_000,
            Self::DeterministicCheck => 0,
        }
    }

    pub fn proposed_input_tokens(self) -> usize {
        match self {
            Self::Draft => 12_512,
            Self::Critique => 32_000,
            Self::Revise | Self::LineEdit => 18_000,
            Self::Judge => 32_000,
            Self::Extract => 8_000,
            Self::DeterministicCheck => 0,
        }
    }

    pub fn proposed_output_tokens(self) -> usize {
        match self {
            Self::Draft => 3_000,
            Self::Critique => 3_000,
            Self::Revise | Self::LineEdit => 3_000,
            Self::Judge => 2_000,
            Self::Extract => 2_000,
            Self::DeterministicCheck => 0,
        }
    }
}

/// Stable reservation identity: `(run, branch, scene, packet, stage, attempt, action)`.
/// Re-admitting the same key is idempotent; restart cannot double-charge it.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct EditorialReservationKey {
    pub run_id: String,
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    pub packet_hash: String,
    pub stage: EditorialStage,
    pub attempt: u32,
    pub action: EditorialActionKind,
}

impl EditorialReservationKey {
    pub fn idempotency_key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            self.run_id,
            self.branch_id,
            self.scene_id.as_deref().unwrap_or("unassigned"),
            self.packet_hash,
            self.stage.as_str(),
            self.attempt,
            self.action.as_str()
        )
    }
}

/// A persisted admission reservation. `usage_known == false` keeps the full
/// reservation charged; `paid_dispatch` with no known price blocks admission
/// because the monetary cap would be unverifiable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialBudgetReservation {
    pub reservation_id: String,
    pub key: EditorialReservationKey,
    pub idempotency_key: String,
    pub reserved: EditorialBudgetUsage,
    pub paid_dispatch: bool,
    pub pricing_known: bool,
    /// `false` means usage was not reported (host completion, transport loss).
    pub usage_known: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<EditorialBudgetUsage>,
    #[serde(default)]
    pub model_call_ids: Vec<String>,
    /// Set atomically when the paid dispatch is claimed. A reservation that was
    /// dispatched but not reconciled cannot be reused for a second call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_at_millis: Option<u64>,
    pub created_at_millis: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconciled_at_millis: Option<u64>,
}

impl EditorialBudgetReservation {
    /// Charge the actual usage when known; otherwise keep the maximum reserved.
    pub fn charged(&self) -> EditorialBudgetUsage {
        if self.usage_known {
            self.actual.unwrap_or(self.reserved)
        } else {
            self.reserved
        }
    }

    pub fn is_reconciled(&self) -> bool {
        self.reconciled_at_millis.is_some()
    }

    pub fn is_dispatched(&self) -> bool {
        self.dispatched_at_millis.is_some()
    }

    /// Claim the one paid dispatch for this reservation. Returns `false` when
    /// another dispatch already owns it.
    pub fn claim_dispatch(&mut self, now_millis: u64) -> bool {
        if self.dispatched_at_millis.is_some() {
            return false;
        }
        self.dispatched_at_millis = Some(now_millis);
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetDenial {
    SceneCalls {
        requested: u32,
        used: u32,
        cap: u32,
    },
    SceneTokens {
        requested: usize,
        used: usize,
        cap: usize,
    },
    SceneDuration {
        requested: u64,
        used: u64,
        cap: u64,
    },
    SceneCost {
        requested: u64,
        used: u64,
        cap: u64,
    },
    RunCalls {
        requested: u32,
        used: u32,
        cap: u32,
    },
    RunTokens {
        requested: usize,
        used: usize,
        cap: usize,
    },
    RunDuration {
        requested: u64,
        used: u64,
        cap: u64,
    },
    RunCost {
        requested: u64,
        used: u64,
        cap: u64,
    },
    MissingPrice,
    UnknownScene,
}

impl std::fmt::Display for BudgetDenial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SceneCalls {
                requested,
                used,
                cap,
            } => write!(
                f,
                "scene_calls_exhausted: reserved {requested} with {used} used of {cap}"
            ),
            Self::SceneTokens {
                requested,
                used,
                cap,
            } => write!(
                f,
                "scene_tokens_exhausted: reserved {requested} with {used} used of {cap}"
            ),
            Self::SceneDuration {
                requested,
                used,
                cap,
            } => write!(
                f,
                "scene_duration_exhausted: reserved {requested}ms with {used}ms used of {cap}ms"
            ),
            Self::SceneCost {
                requested,
                used,
                cap,
            } => write!(
                f,
                "scene_cost_exhausted: reserved {requested} with {used} used of {cap}"
            ),
            Self::RunCalls {
                requested,
                used,
                cap,
            } => write!(
                f,
                "run_calls_exhausted: reserved {requested} with {used} used of {cap}"
            ),
            Self::RunTokens {
                requested,
                used,
                cap,
            } => write!(
                f,
                "run_tokens_exhausted: reserved {requested} with {used} used of {cap}"
            ),
            Self::RunDuration {
                requested,
                used,
                cap,
            } => write!(
                f,
                "run_duration_exhausted: reserved {requested}ms with {used}ms used of {cap}ms"
            ),
            Self::RunCost {
                requested,
                used,
                cap,
            } => write!(
                f,
                "run_cost_exhausted: reserved {requested} with {used} used of {cap}"
            ),
            Self::MissingPrice => write!(
                f,
                "missing_price: a paid dispatch requires a configured model price"
            ),
            Self::UnknownScene => write!(f, "unknown_scene: no editorial scene at that index"),
        }
    }
}

impl std::error::Error for BudgetDenial {}

/// Durable admission ledger for one scene. Reservations are pushed only after
/// all scene and run ceilings admit them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialBudgetLedgerV1 {
    pub scene_caps: EditorialBudgetCaps,
    pub run_caps: EditorialBudgetCaps,
    /// Scene-scope reservations; every admission is also projected into `run_reservations`.
    #[serde(default)]
    pub scene_reservations: Vec<EditorialBudgetReservation>,
    #[serde(default)]
    pub run_reservations: Vec<EditorialBudgetReservation>,
}

impl EditorialBudgetLedgerV1 {
    pub fn new(scene_caps: EditorialBudgetCaps, run_caps: EditorialBudgetCaps) -> Self {
        Self {
            scene_caps,
            run_caps,
            scene_reservations: Vec::new(),
            run_reservations: Vec::new(),
        }
    }

    pub fn scene_used(&self) -> EditorialBudgetUsage {
        self.scene_reservations
            .iter()
            .fold(EditorialBudgetUsage::default(), |acc, reservation| {
                acc.saturating_add(reservation.charged())
            })
    }

    pub fn run_used(&self) -> EditorialBudgetUsage {
        self.run_reservations
            .iter()
            .fold(EditorialBudgetUsage::default(), |acc, reservation| {
                acc.saturating_add(reservation.charged())
            })
    }

    pub fn scene_remaining(&self) -> EditorialBudgetCaps {
        let used = self.scene_used();
        EditorialBudgetCaps {
            max_calls: self.scene_caps.max_calls.saturating_sub(used.calls),
            max_tokens: self.scene_caps.max_tokens.saturating_sub(used.tokens),
            max_millis: self.scene_caps.max_millis.saturating_sub(used.millis),
            max_usd_micros: self
                .scene_caps
                .max_usd_micros
                .saturating_sub(used.usd_micros),
        }
    }

    pub fn run_remaining(&self) -> EditorialBudgetCaps {
        let used = self.run_used();
        EditorialBudgetCaps {
            max_calls: self.run_caps.max_calls.saturating_sub(used.calls),
            max_tokens: self.run_caps.max_tokens.saturating_sub(used.tokens),
            max_millis: self.run_caps.max_millis.saturating_sub(used.millis),
            max_usd_micros: self.run_caps.max_usd_micros.saturating_sub(used.usd_micros),
        }
    }

    pub fn find(&self, idempotency_key: &str) -> Option<&EditorialBudgetReservation> {
        self.scene_reservations
            .iter()
            .find(|reservation| reservation.idempotency_key == idempotency_key)
    }

    pub fn find_mut(&mut self, idempotency_key: &str) -> Option<&mut EditorialBudgetReservation> {
        self.scene_reservations
            .iter_mut()
            .find(|reservation| reservation.idempotency_key == idempotency_key)
    }

    /// Claim dispatch on the scene and run projections together. Returns
    /// `None` when the key is absent, `Some(false)` when already dispatched.
    pub fn claim_dispatch(&mut self, idempotency_key: &str, now_millis: u64) -> Option<bool> {
        let mut claimed = None;
        for reservation in self
            .scene_reservations
            .iter_mut()
            .chain(self.run_reservations.iter_mut())
        {
            if reservation.idempotency_key != idempotency_key {
                continue;
            }
            let this_claim = reservation.claim_dispatch(now_millis);
            if claimed.is_none() {
                claimed = Some(this_claim);
            }
        }
        claimed
    }

    /// Admit one exact proposed reservation. Re-admission of an existing
    /// idempotency key returns the persisted reservation unchanged.
    pub fn admit(
        &mut self,
        reservation: EditorialBudgetReservation,
    ) -> Result<EditorialBudgetReservation, BudgetDenial> {
        self.admit_with_prior_run_usage(reservation, EditorialBudgetUsage::default())
    }

    /// Admit with run usage already charged by other scenes. The per-scene
    /// ledger owns its scene reservations; a controller sums the other scenes
    /// so the shared per-run ceiling cannot be reset by switching scenes.
    pub fn admit_with_prior_run_usage(
        &mut self,
        reservation: EditorialBudgetReservation,
        prior_run_usage: EditorialBudgetUsage,
    ) -> Result<EditorialBudgetReservation, BudgetDenial> {
        if let Some(existing) = self.find(&reservation.idempotency_key) {
            return Ok(existing.clone());
        }
        if reservation.paid_dispatch && !reservation.pricing_known {
            return Err(BudgetDenial::MissingPrice);
        }
        let reserved = reservation.reserved;
        let scene_used = self.scene_used();
        let run_used = prior_run_usage.saturating_add(self.run_used());

        let scene_candidate = scene_used.saturating_add(reserved);
        match scene_candidate.exceeds(&self.scene_caps) {
            Some(EditorialBudgetExhaustion::Calls) => {
                return Err(BudgetDenial::SceneCalls {
                    requested: reserved.calls,
                    used: scene_used.calls,
                    cap: self.scene_caps.max_calls,
                });
            }
            Some(EditorialBudgetExhaustion::Tokens) => {
                return Err(BudgetDenial::SceneTokens {
                    requested: reserved.tokens,
                    used: scene_used.tokens,
                    cap: self.scene_caps.max_tokens,
                });
            }
            Some(EditorialBudgetExhaustion::Duration) => {
                return Err(BudgetDenial::SceneDuration {
                    requested: reserved.millis,
                    used: scene_used.millis,
                    cap: self.scene_caps.max_millis,
                });
            }
            Some(EditorialBudgetExhaustion::Cost) => {
                return Err(BudgetDenial::SceneCost {
                    requested: reserved.usd_micros,
                    used: scene_used.usd_micros,
                    cap: self.scene_caps.max_usd_micros,
                });
            }
            None => {}
        }

        let run_candidate = run_used.saturating_add(reserved);
        match run_candidate.exceeds(&self.run_caps) {
            Some(EditorialBudgetExhaustion::Calls) => {
                return Err(BudgetDenial::RunCalls {
                    requested: reserved.calls,
                    used: run_used.calls,
                    cap: self.run_caps.max_calls,
                });
            }
            Some(EditorialBudgetExhaustion::Tokens) => {
                return Err(BudgetDenial::RunTokens {
                    requested: reserved.tokens,
                    used: run_used.tokens,
                    cap: self.run_caps.max_tokens,
                });
            }
            Some(EditorialBudgetExhaustion::Duration) => {
                return Err(BudgetDenial::RunDuration {
                    requested: reserved.millis,
                    used: run_used.millis,
                    cap: self.run_caps.max_millis,
                });
            }
            Some(EditorialBudgetExhaustion::Cost) => {
                return Err(BudgetDenial::RunCost {
                    requested: reserved.usd_micros,
                    used: run_used.usd_micros,
                    cap: self.run_caps.max_usd_micros,
                });
            }
            None => {}
        }

        self.scene_reservations.push(reservation.clone());
        self.run_reservations.push(reservation.clone());
        Ok(reservation)
    }

    /// Reconcile a reservation with reported usage. Unknown usage keeps the
    /// maximum charged; a later known report may replace it.
    pub fn reconcile(
        &mut self,
        idempotency_key: &str,
        actual: Option<EditorialBudgetUsage>,
        model_call_ids: Vec<String>,
        reconciled_at_millis: u64,
    ) -> Option<EditorialBudgetReservation> {
        let mut updated: Option<EditorialBudgetReservation> = None;
        for reservation in self
            .scene_reservations
            .iter_mut()
            .chain(self.run_reservations.iter_mut())
        {
            if reservation.idempotency_key != idempotency_key {
                continue;
            }
            reservation.actual = actual;
            reservation.usage_known = actual.is_some();
            reservation.model_call_ids = model_call_ids.clone();
            reservation.reconciled_at_millis = Some(reconciled_at_millis);
            if updated.is_none() {
                updated = Some(reservation.clone());
            }
        }
        updated
    }
}

/// How the controller executes an editorial pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EditorialExecutionMode {
    /// The active host (assistant) drafts/revises; the controller validates and
    /// judges via cleared routes when available.
    Hybrid,
    /// The controller dispatches the same rendered packet to configured routes.
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HostEditorialTaskKind {
    Draft,
    Revision,
}

/// Bounded host task. Carries the exact packet (or its immutable hash for an
/// MCP-pull agent) and, for revision, the incumbent plus selected directives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HostEditorialTaskV1 {
    #[serde(default = "default_host_task_schema_version")]
    pub schema_version: String,
    pub task_id: String,
    pub kind: HostEditorialTaskKind,
    pub run_id: String,
    pub project_id: String,
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    pub packet_hash: String,
    pub source_packet_sha256: String,
    pub reservation_id: String,
    pub rating: ContentRating,
    pub target_words: TargetWords,
    /// Full packet for inline hybrid hosts.
    pub packet: DraftingPacketV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incumbent_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incumbent_text_sha256: Option<String>,
    #[serde(default)]
    pub directives: Vec<CritiqueDirective>,
    #[serde(default)]
    pub strengths_to_preserve: Vec<String>,
    pub max_output_tokens: usize,
    pub created_at_millis: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at_millis: Option<u64>,
}

fn default_host_task_schema_version() -> String {
    HOST_EDITORIAL_TASK_SCHEMA_VERSION.into()
}

impl HostEditorialTaskV1 {
    pub fn is_pending(&self) -> bool {
        self.submitted_at_millis.is_none()
    }
}

/// Host completion usage may be unknown. `None` means unknown; the controller
/// keeps the configured maximum reserved and records the fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HostEditorialSubmissionV1 {
    pub task_id: String,
    pub packet_hash: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<EditorialBudgetUsage>,
    #[serde(default)]
    pub truncated: bool,
}

/// Explicit host-completion receipt. Host prose has no provider generation id,
/// so the controller binds it to the immutable task instead of fabricating one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HostCompletionReceiptV1 {
    pub task_id: String,
    pub agent_id: String,
    pub text_sha256: String,
    pub normalized_output_sha256: String,
    pub usage_known: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManualJudgmentReason {
    NoAutomatedJudgeRoute,
    JudgeRouteNotCleared,
    MissingVerifiedTokenizer,
    InvalidJudgeOutput,
}

impl ManualJudgmentReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoAutomatedJudgeRoute => "no_automated_judge_route",
            Self::JudgeRouteNotCleared => "judge_route_not_cleared",
            Self::MissingVerifiedTokenizer => "missing_verified_tokenizer",
            Self::InvalidJudgeOutput => "invalid_judge_output",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingManualJudgmentV1 {
    pub task_id: String,
    pub reason: ManualJudgmentReason,
    pub incumbent_text_sha256: String,
    pub candidate_text_sha256: String,
    pub created_at_millis: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HostSubmissionRejection {
    TaskNotFound,
    TaskAlreadySubmitted,
    PacketHashMismatch,
    EmptyText,
    Truncated,
    ReservationMissing,
    BudgetExhausted,
}

impl HostSubmissionRejection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskNotFound => "task_not_found",
            Self::TaskAlreadySubmitted => "task_already_submitted",
            Self::PacketHashMismatch => "packet_hash_mismatch",
            Self::EmptyText => "empty_text",
            Self::Truncated => "truncated",
            Self::ReservationMissing => "reservation_missing",
            Self::BudgetExhausted => "budget_exhausted",
        }
    }
}

/// Outcome of validating a host submission. Missing automated judgment is an
/// explicit pending state, never a preference for the candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum HostSubmissionOutcomeV1 {
    Rejected {
        rejection: HostSubmissionRejection,
        message: String,
    },
    /// Initial host draft accepted for deterministic checking.
    DraftAccepted {
        receipt: HostCompletionReceiptV1,
        text_sha256: String,
    },
    /// Revision candidate passed deterministic checks and awaits judge.
    CandidateReady {
        receipt: HostCompletionReceiptV1,
        text_sha256: String,
    },
    PendingManualJudgment {
        pending: PendingManualJudgmentV1,
    },
    /// Both judge orders preferred the candidate.
    RevisionAccepted {
        receipt: HostCompletionReceiptV1,
        text_sha256: String,
    },
    /// Judge tie/incumbent win/order disagreement or deterministic rejection.
    RevisionRetained {
        reason: String,
        incumbent_text_sha256: String,
        candidate_text_sha256: String,
    },
}

/// Persisted, resolved editorial configuration. Older run state has no such
/// record and resumes exact legacy behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResolvedEditorialSettingsV1 {
    #[serde(default = "default_resolved_settings_schema_version")]
    pub schema_version: String,
    pub editorial_policy: EditorialPolicy,
    pub execution_mode: EditorialExecutionMode,
    pub packet_schema_version: String,
    pub packet_version: String,
    pub rubric_version: String,
    pub critic_rubric_version: String,
    pub max_rounds: u32,
    pub line_edit_enabled: bool,
    pub chapter_pass_enabled: bool,
    pub scene_budget: EditorialBudgetCaps,
    pub run_budget: EditorialBudgetCaps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<EditorialPricingV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_seed: Option<i64>,
}

fn default_resolved_settings_schema_version() -> String {
    RESOLVED_EDITORIAL_SETTINGS_SCHEMA_VERSION.into()
}

impl ResolvedEditorialSettingsV1 {
    pub fn legacy() -> Self {
        Self {
            schema_version: RESOLVED_EDITORIAL_SETTINGS_SCHEMA_VERSION.into(),
            editorial_policy: EditorialPolicy::Legacy,
            execution_mode: EditorialExecutionMode::Hybrid,
            packet_schema_version: "drafting_packet_v1".into(),
            packet_version: TEMPLATE_VERSION_V1.into(),
            rubric_version: "scene-craft-v1".into(),
            critic_rubric_version: "scene-critic-v1".into(),
            max_rounds: 0,
            line_edit_enabled: false,
            chapter_pass_enabled: false,
            scene_budget: EditorialBudgetCaps::scene_default(),
            run_budget: EditorialBudgetCaps::run_default(),
            pricing: None,
            generation_seed: None,
        }
    }

    pub fn critique_v1_default() -> Self {
        Self {
            editorial_policy: EditorialPolicy::CritiqueV1,
            max_rounds: DEFAULT_EDITORIAL_MAX_ROUNDS,
            ..Self::legacy()
        }
    }

    pub fn is_legacy(&self) -> bool {
        self.editorial_policy == EditorialPolicy::Legacy
    }

    pub fn pricing_known(&self) -> bool {
        self.pricing.is_some()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.max_rounds > HARD_EDITORIAL_MAX_ROUNDS {
            return Err(format!(
                "max_rounds must be <= {HARD_EDITORIAL_MAX_ROUNDS}, got {}",
                self.max_rounds
            ));
        }
        if self.scene_budget.max_calls > DEFAULT_SCENE_CALL_CAP {
            return Err(format!(
                "scene max_calls must be <= {DEFAULT_SCENE_CALL_CAP}, got {}",
                self.scene_budget.max_calls
            ));
        }
        if self.scene_budget.max_tokens > DEFAULT_SCENE_TOKEN_CAP {
            return Err(format!(
                "scene max_tokens must be <= {DEFAULT_SCENE_TOKEN_CAP}, got {}",
                self.scene_budget.max_tokens
            ));
        }
        if self.scene_budget.max_millis > DEFAULT_SCENE_MILLIS_CAP {
            return Err(format!(
                "scene max_millis must be <= {DEFAULT_SCENE_MILLIS_CAP}, got {}",
                self.scene_budget.max_millis
            ));
        }
        if self.run_budget.max_calls > DEFAULT_RUN_CALL_CAP {
            return Err(format!(
                "run max_calls must be <= {DEFAULT_RUN_CALL_CAP}, got {}",
                self.run_budget.max_calls
            ));
        }
        if self.run_budget.max_tokens > DEFAULT_RUN_TOKEN_CAP {
            return Err(format!(
                "run max_tokens must be <= {DEFAULT_RUN_TOKEN_CAP}, got {}",
                self.run_budget.max_tokens
            ));
        }
        if self.run_budget.max_millis > DEFAULT_RUN_MILLIS_CAP {
            return Err(format!(
                "run max_millis must be <= {DEFAULT_RUN_MILLIS_CAP}, got {}",
                self.run_budget.max_millis
            ));
        }
        Ok(())
    }
}

/// Additive seed/run configuration. Every field is optional so older seeds
/// deserialize unchanged and resolve to exact legacy behavior.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialRunConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editorial_policy: Option<EditorialPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<EditorialExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critic_rubric_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_edit_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_pass_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_budget: Option<EditorialBudgetCaps>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_budget: Option<EditorialBudgetCaps>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<EditorialPricingV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_seed: Option<i64>,
}

impl ResolvedEditorialSettingsV1 {
    /// Resolve a seed/run config into the persisted settings record. `Legacy`
    /// always resolves to the legacy record so pre-PR-7 runs behave identically.
    pub fn resolve(config: &EditorialRunConfig) -> Result<Self, String> {
        let policy = config.editorial_policy.unwrap_or_default();
        if policy == EditorialPolicy::Legacy {
            return Ok(Self::legacy());
        }
        let mut resolved = Self::critique_v1_default();
        resolved.execution_mode = config
            .execution_mode
            .unwrap_or(EditorialExecutionMode::Hybrid);
        if let Some(version) = &config.packet_version {
            resolved.packet_version = version.clone();
        }
        if let Some(version) = &config.rubric_version {
            resolved.rubric_version = version.clone();
        }
        if let Some(version) = &config.critic_rubric_version {
            resolved.critic_rubric_version = version.clone();
        }
        if let Some(rounds) = config.max_rounds {
            resolved.max_rounds = rounds;
        }
        if let Some(line_edit) = config.line_edit_enabled {
            resolved.line_edit_enabled = line_edit;
        }
        if let Some(chapter_pass) = config.chapter_pass_enabled {
            resolved.chapter_pass_enabled = chapter_pass;
        }
        if let Some(scene_budget) = config.scene_budget {
            resolved.scene_budget = scene_budget;
        }
        if let Some(run_budget) = config.run_budget {
            resolved.run_budget = run_budget;
        }
        resolved.pricing = config.pricing.clone();
        resolved.generation_seed = config.generation_seed;
        resolved.validate()?;
        Ok(resolved)
    }
}

/// Route/tokenizer availability observed without dispatching a model call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialRouteAvailability {
    pub draft: bool,
    pub review: bool,
    pub line_edit: bool,
    pub extract: bool,
    pub verified_tokenizer: bool,
}

impl EditorialRouteAvailability {
    pub fn all_available() -> Self {
        Self {
            draft: true,
            review: true,
            line_edit: true,
            extract: true,
            verified_tokenizer: true,
        }
    }

    /// `(missing route, missing tokenizer)` for an action.
    pub fn missing_for(&self, action: EditorialActionKind) -> (Option<String>, bool) {
        let missing_route = match action {
            EditorialActionKind::Draft | EditorialActionKind::Revise => {
                (!self.draft).then(|| "draft".to_string())
            }
            EditorialActionKind::Critique | EditorialActionKind::Judge => {
                (!self.review).then(|| "review".to_string())
            }
            EditorialActionKind::LineEdit => (!self.line_edit).then(|| "line_edit".to_string()),
            EditorialActionKind::Extract => (!self.extract).then(|| "extract".to_string()),
            EditorialActionKind::DeterministicCheck => None,
        };
        let needs_tokenizer = action.is_model_pass();
        (missing_route, needs_tokenizer && !self.verified_tokenizer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialCriticSummaryV1 {
    pub status: String,
    pub rubric_version: String,
    pub directive_count: usize,
    pub requires_author_decision_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialJudgeSummaryV1 {
    pub verdict: String,
    pub accepted_revision: bool,
    pub order_agreement: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_reason: Option<String>,
}

/// Text identity only; the controller artifact holds prose, telemetry does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialTextRefV1 {
    pub text_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// One persisted scene in the editorial controller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialSceneStateV1 {
    pub chapter_number: u32,
    pub scene_order: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    pub packet_hash: String,
    pub source_packet_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_artifact_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_artifact_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_artifact_path: Option<String>,
    pub stage: EditorialStage,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incumbent: Option<EditorialTextRefV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<EditorialTextRefV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critic: Option<EditorialCriticSummaryV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge: Option<EditorialJudgeSummaryV1>,
    #[serde(default)]
    pub selected_directives: Vec<CritiqueDirective>,
    #[serde(default)]
    pub strengths_to_preserve: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_edit_span_summary: Option<crate::drafting::EditSpanSummary>,
    #[serde(default)]
    pub parked_issues: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_host_task: Option<HostEditorialTaskV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_manual_judgment: Option<PendingManualJudgmentV1>,
    pub availability: EditorialRouteAvailability,
    pub ledger: EditorialBudgetLedgerV1,
    #[serde(default)]
    pub attribution_event_ids: Vec<String>,
    #[serde(default)]
    pub line_edit_attempted: bool,
    #[serde(default)]
    pub extracted: bool,
}

impl EditorialSceneStateV1 {
    pub fn find_pending_task(&self) -> Option<&HostEditorialTaskV1> {
        self.pending_host_task
            .as_ref()
            .filter(|task| task.is_pending())
    }
}

/// Durable controller artifact. Reservations are written with the same atomic
/// save as stage state, so a process restart cannot reset counters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialControllerStateV1 {
    #[serde(default = "default_controller_schema_version")]
    pub schema_version: String,
    pub run_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub settings: ResolvedEditorialSettingsV1,
    #[serde(default)]
    pub scenes: Vec<EditorialSceneStateV1>,
    /// Prose-free attribution events linked to `model_call` ids.
    #[serde(default)]
    pub attribution: Vec<EditorialAttributionV1>,
}

fn default_controller_schema_version() -> String {
    EDITORIAL_CONTROLLER_SCHEMA_VERSION.into()
}

impl EditorialControllerStateV1 {
    pub fn new(
        run_id: String,
        project_id: String,
        branch_id: String,
        settings: ResolvedEditorialSettingsV1,
    ) -> Self {
        Self {
            schema_version: EDITORIAL_CONTROLLER_SCHEMA_VERSION.into(),
            run_id,
            project_id,
            branch_id,
            settings,
            scenes: Vec::new(),
            attribution: Vec::new(),
        }
    }

    pub fn scene(
        &self,
        scene_id: Option<&str>,
        chapter: u32,
        order: u32,
    ) -> Option<&EditorialSceneStateV1> {
        self.scenes.iter().find(|scene| {
            scene_id.is_some_and(|id| scene.scene_id.as_deref() == Some(id))
                || (scene.chapter_number == chapter && scene.scene_order == order)
        })
    }

    pub fn scene_mut(
        &mut self,
        scene_id: Option<&str>,
        chapter: u32,
        order: u32,
    ) -> Option<&mut EditorialSceneStateV1> {
        self.scenes.iter_mut().find(|scene| {
            scene_id.is_some_and(|id| scene.scene_id.as_deref() == Some(id))
                || (scene.chapter_number == chapter && scene.scene_order == order)
        })
    }

    /// Run usage charged by every scene except `scene_index`.
    pub fn run_usage_excluding(&self, scene_index: usize) -> EditorialBudgetUsage {
        self.scenes
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != scene_index)
            .fold(EditorialBudgetUsage::default(), |usage, (_, scene)| {
                usage.saturating_add(scene.ledger.run_used())
            })
    }

    pub fn attribution(&self, event_id: &str) -> Option<&EditorialAttributionV1> {
        self.attribution
            .iter()
            .find(|event| event.event_id == event_id)
    }

    pub fn record_attribution(&mut self, event: EditorialAttributionV1) {
        if self
            .attribution
            .iter()
            .any(|existing| existing.event_id == event.event_id)
        {
            return;
        }
        self.attribution.push(event);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != EDITORIAL_CONTROLLER_SCHEMA_VERSION {
            return Err(format!(
                "unknown editorial controller schema_version {:?}",
                self.schema_version
            ));
        }
        self.settings.validate()?;
        for scene in &self.scenes {
            if scene.packet_hash.is_empty() {
                return Err("editorial scene is missing packet_hash".into());
            }
            if let Some(task) = &scene.pending_host_task
                && (task.task_id.is_empty()
                    || task.packet_hash != scene.packet_hash
                    || task.project_id != self.project_id
                    || task.branch_id != self.branch_id)
            {
                return Err("host editorial task does not match its controller scene".into());
            }
        }
        Ok(())
    }
}

/// The next persisted editorial action for one scene. `DeterministicCheck` is
/// deterministic and reserves no model calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialNextActionV1 {
    pub action: EditorialActionKind,
    pub stage: EditorialStage,
    pub attempt: u32,
    pub reservation: EditorialBudgetUsage,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_route: Option<String>,
    #[serde(default)]
    pub missing_tokenizer: bool,
    #[serde(default)]
    pub pending_host_task_id: Option<String>,
    #[serde(default)]
    pub pending_manual_judgment: bool,
}

pub fn editorial_next_action(
    scene: &EditorialSceneStateV1,
    settings: &ResolvedEditorialSettingsV1,
) -> Option<EditorialNextActionV1> {
    if settings.is_legacy() {
        return None;
    }
    if scene.pending_manual_judgment.is_some() && scene.stage.is_terminal() {
        return Some(EditorialNextActionV1 {
            action: EditorialActionKind::Judge,
            stage: scene.stage,
            attempt: scene.attempt,
            reservation: EditorialBudgetUsage::default(),
            message: "await author review: automated judge was unavailable".into(),
            missing_route: Some("review".into()),
            missing_tokenizer: false,
            pending_host_task_id: None,
            pending_manual_judgment: true,
        });
    }
    if scene.stage.is_terminal() {
        if settings.line_edit_enabled && !scene.line_edit_attempted {
            return Some(EditorialNextActionV1 {
                action: EditorialActionKind::LineEdit,
                stage: scene.stage,
                attempt: scene.attempt,
                reservation: reservation_usage(EditorialActionKind::LineEdit, settings),
                message: "optional line edit".into(),
                missing_route: scene
                    .availability
                    .missing_for(EditorialActionKind::LineEdit)
                    .0,
                missing_tokenizer: scene
                    .availability
                    .missing_for(EditorialActionKind::LineEdit)
                    .1,
                pending_host_task_id: None,
                pending_manual_judgment: false,
            });
        }
        return None;
    }

    let action = match scene.stage {
        EditorialStage::PacketReady => EditorialActionKind::Draft,
        EditorialStage::Drafted => EditorialActionKind::DeterministicCheck,
        EditorialStage::DeterministicChecked => {
            if scene.candidate.is_some() {
                EditorialActionKind::DeterministicCheck
            } else {
                EditorialActionKind::Critique
            }
        }
        EditorialStage::Critiqued => {
            if scene.selected_directives.is_empty() {
                return None;
            }
            EditorialActionKind::Revise
        }
        EditorialStage::CandidateReady => EditorialActionKind::DeterministicCheck,
        EditorialStage::CandidateChecked => EditorialActionKind::Judge,
        EditorialStage::PairwiseJudged
        | EditorialStage::AcceptedOriginal
        | EditorialStage::AcceptedRevision => return None,
    };
    let (missing_route, missing_tokenizer) = scene.availability.missing_for(action);
    Some(EditorialNextActionV1 {
        action,
        stage: scene.stage,
        attempt: scene.attempt,
        reservation: reservation_usage(action, settings),
        message: action_message(action),
        missing_route,
        missing_tokenizer,
        pending_host_task_id: scene.find_pending_task().map(|task| task.task_id.clone()),
        pending_manual_judgment: scene.pending_manual_judgment.is_some(),
    })
}

fn action_message(action: EditorialActionKind) -> String {
    match action {
        EditorialActionKind::Draft => "draft the scene".into(),
        EditorialActionKind::DeterministicCheck => "run deterministic checks".into(),
        EditorialActionKind::Critique => "critique the draft".into(),
        EditorialActionKind::Revise => "revise the draft".into(),
        EditorialActionKind::Judge => "run the swapped-order judge".into(),
        EditorialActionKind::LineEdit => "polish accepted prose".into(),
        EditorialActionKind::Extract => "extract structured metadata".into(),
    }
}

fn reservation_usage(
    action: EditorialActionKind,
    settings: &ResolvedEditorialSettingsV1,
) -> EditorialBudgetUsage {
    let usd_micros = settings.pricing.as_ref().map_or(0, |pricing| {
        pricing.cost_micros(
            action.proposed_input_tokens(),
            action.proposed_output_tokens(),
        )
    });
    EditorialBudgetUsage {
        calls: action.proposed_calls(),
        tokens: action.proposed_tokens(),
        millis: action.proposed_millis(),
        usd_micros,
    }
}

pub fn reservation_usage_for(
    action: EditorialActionKind,
    settings: &ResolvedEditorialSettingsV1,
) -> EditorialBudgetUsage {
    reservation_usage(action, settings)
}

/// Prose-free attribution record for one editorial pass. It links to
/// `model_call` ids and carries only hashes, enums and counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialAttributionV1 {
    #[serde(default = "default_attribution_schema_version")]
    pub schema_version: String,
    pub event_id: String,
    pub run_id: String,
    pub project_id: String,
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    pub stage: EditorialStage,
    pub attempt: u32,
    pub action: EditorialActionKind,
    pub prompt_template_id: String,
    pub prompt_version: String,
    pub prompt_hash: String,
    pub context_packet_hash: String,
    pub packet_content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_version: Option<String>,
    pub source_snapshot_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_text_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_text_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_order: Option<String>,
    pub outcome: String,
    pub duration_millis: u64,
    pub usage_known: bool,
    pub actual_tokens: Option<usize>,
    pub reserved_tokens: usize,
    pub actual_usd_micros: Option<u64>,
    pub reserved_usd_micros: u64,
    #[serde(default)]
    pub model_call_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_seed: Option<i64>,
    pub recorded_at_millis: u64,
}

fn default_attribution_schema_version() -> String {
    EDITORIAL_ATTRIBUTION_SCHEMA_VERSION.into()
}

/// Immutable packet retrieval for a hybrid host task. Retrieval is free of
/// model calls but cannot outlive the task or bypass its reservation/branch
/// snapshot: a submitted, stale, or unadmitted task is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacketRetrievalError {
    TaskNotFound(String),
    TaskAlreadySubmitted(String),
    PacketHashMismatch {
        expected: String,
        found: String,
    },
    ReservationMissing(String),
    BranchSnapshotMismatch {
        task_branch: String,
        active_branch: String,
    },
}

impl std::fmt::Display for PacketRetrievalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TaskNotFound(task) => write!(f, "task_not_found: no pending host task {task}"),
            Self::TaskAlreadySubmitted(task) => {
                write!(
                    f,
                    "task_already_submitted: host task {task} is no longer pending"
                )
            }
            Self::PacketHashMismatch { expected, found } => write!(
                f,
                "packet_hash_mismatch: expected {expected}, found {found}"
            ),
            Self::ReservationMissing(task) => write!(
                f,
                "reservation_missing: host task {task} has no admitted reservation"
            ),
            Self::BranchSnapshotMismatch {
                task_branch,
                active_branch,
            } => write!(
                f,
                "branch_snapshot_mismatch: task branch {task_branch} is not active {active_branch}"
            ),
        }
    }
}

impl std::error::Error for PacketRetrievalError {}

/// Retrieve the immutable packet for a pending host task. The packet is
/// snapshotted on the task; this validates task id, hash, branch and budget
/// linkage before returning it.
pub fn retrieve_host_task_packet(
    scene: &EditorialSceneStateV1,
    task_id: &str,
    expected_packet_hash: &str,
    active_branch_id: &str,
) -> Result<DraftingPacketV1, PacketRetrievalError> {
    let task = scene
        .pending_host_task
        .as_ref()
        .filter(|task| task.task_id == task_id)
        .ok_or_else(|| PacketRetrievalError::TaskNotFound(task_id.to_string()))?;
    if !task.is_pending() {
        return Err(PacketRetrievalError::TaskAlreadySubmitted(
            task_id.to_string(),
        ));
    }
    if task.packet_hash != expected_packet_hash {
        return Err(PacketRetrievalError::PacketHashMismatch {
            expected: expected_packet_hash.to_string(),
            found: task.packet_hash.clone(),
        });
    }
    if task.branch_id != active_branch_id {
        return Err(PacketRetrievalError::BranchSnapshotMismatch {
            task_branch: task.branch_id.clone(),
            active_branch: active_branch_id.to_string(),
        });
    }
    if scene
        .ledger
        .find(&reservation_key_for_task(scene, task).idempotency_key())
        .is_none()
    {
        return Err(PacketRetrievalError::ReservationMissing(
            task_id.to_string(),
        ));
    }
    Ok(task.packet.clone())
}

fn reservation_key_for_task(
    scene: &EditorialSceneStateV1,
    task: &HostEditorialTaskV1,
) -> EditorialReservationKey {
    EditorialReservationKey {
        run_id: task.run_id.clone(),
        branch_id: task.branch_id.clone(),
        scene_id: scene.scene_id.clone(),
        packet_hash: scene.packet_hash.clone(),
        stage: scene.stage,
        attempt: scene.attempt,
        action: match task.kind {
            HostEditorialTaskKind::Draft => EditorialActionKind::Draft,
            HostEditorialTaskKind::Revision => EditorialActionKind::Revise,
        },
    }
}

/// Packet sources must come from the controller's active branch; task
/// retrieval cannot smuggle an inherited/stale snapshot.
pub fn task_packet_sources_are_branch_consistent(
    task: &HostEditorialTaskV1,
    branch_id: &str,
) -> bool {
    task.branch_id == branch_id
        && task
            .packet
            .source_versions
            .iter()
            .all(|source: &PacketSource| source.branch_id == branch_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with_price() -> ResolvedEditorialSettingsV1 {
        let mut settings = ResolvedEditorialSettingsV1::critique_v1_default();
        settings.pricing = Some(EditorialPricingV1 {
            input_usd_micros_per_million: 1_000_000,
            output_usd_micros_per_million: 2_000_000,
        });
        settings
    }

    fn reservation(
        action: EditorialActionKind,
        scene: &str,
        settings: &ResolvedEditorialSettingsV1,
    ) -> EditorialBudgetReservation {
        let key = EditorialReservationKey {
            run_id: "run:1".into(),
            branch_id: "branch:1".into(),
            scene_id: Some(scene.into()),
            packet_hash: format!("packet:{scene}"),
            stage: EditorialStage::PacketReady,
            attempt: 0,
            action,
        };
        EditorialBudgetReservation {
            reservation_id: format!("reservation:{scene}:{}", action.as_str()),
            key: key.clone(),
            idempotency_key: key.idempotency_key(),
            reserved: reservation_usage(action, settings),
            paid_dispatch: false,
            pricing_known: settings.pricing_known(),
            usage_known: false,
            actual: None,
            model_call_ids: Vec::new(),
            dispatched_at_millis: None,
            created_at_millis: 1,
            reconciled_at_millis: None,
        }
    }

    #[test]
    fn reservation_usage_matches_design_defaults() {
        let settings = settings_with_price();
        assert_eq!(
            reservation_usage(EditorialActionKind::Draft, &settings),
            EditorialBudgetUsage {
                calls: 1,
                tokens: 15_512,
                millis: 90_000,
                // 12,512 @ $1/M + 3,000 @ $2/M = 12,512 + 6,000 micros.
                usd_micros: 18_512,
            }
        );
        let judge = reservation_usage(EditorialActionKind::Judge, &settings);
        assert_eq!(judge.calls, 2);
        assert_eq!(judge.tokens, 34_000);
        assert_eq!(judge.millis, 90_000);
    }

    #[test]
    fn admission_is_idempotent_and_persists_across_restart_shape() {
        let settings = settings_with_price();
        let mut ledger = EditorialBudgetLedgerV1::new(
            EditorialBudgetCaps::scene_default(),
            EditorialBudgetCaps::run_default(),
        );
        let draft = reservation(EditorialActionKind::Draft, "scene:1", &settings);
        let first = ledger.admit(draft.clone()).expect("draft admitted");
        let again = ledger.admit(draft).expect("re-admission is idempotent");
        assert_eq!(first, again);
        assert_eq!(ledger.scene_reservations.len(), 1);
        assert_eq!(ledger.run_reservations.len(), 1);
        let encoded = serde_json::to_string(&ledger).expect("serialize");
        let restored: EditorialBudgetLedgerV1 =
            serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(restored.scene_used().tokens, 15_512);
    }

    #[test]
    fn scene_call_cap_denies_admission() {
        let settings = settings_with_price();
        let caps = EditorialBudgetCaps {
            max_calls: 1,
            ..EditorialBudgetCaps::scene_default()
        };
        let mut ledger = EditorialBudgetLedgerV1::new(caps, EditorialBudgetCaps::run_default());
        ledger
            .admit(reservation(
                EditorialActionKind::Draft,
                "scene:1",
                &settings,
            ))
            .expect("first call admitted");
        let denial = ledger
            .admit(reservation(
                EditorialActionKind::Critique,
                "scene:1",
                &settings,
            ))
            .expect_err("second call denied");
        assert!(matches!(
            denial,
            BudgetDenial::SceneCalls {
                requested: 2,
                used: 1,
                cap: 1
            }
        ));
    }

    #[test]
    fn unknown_usage_keeps_full_reservation_charged() {
        let settings = settings_with_price();
        let mut ledger = EditorialBudgetLedgerV1::new(
            EditorialBudgetCaps::scene_default(),
            EditorialBudgetCaps::run_default(),
        );
        let draft = reservation(EditorialActionKind::Draft, "scene:1", &settings);
        ledger.admit(draft.clone()).expect("admitted");
        ledger.reconcile(&draft.idempotency_key, None, vec!["model_call:1".into()], 2);
        assert_eq!(ledger.scene_used().tokens, 15_512);
        assert!(!ledger.scene_reservations[0].usage_known);
        assert_eq!(
            ledger.scene_reservations[0].model_call_ids,
            vec!["model_call:1"]
        );
    }

    #[test]
    fn known_usage_reconciles_to_actual() {
        let settings = settings_with_price();
        let mut ledger = EditorialBudgetLedgerV1::new(
            EditorialBudgetCaps::scene_default(),
            EditorialBudgetCaps::run_default(),
        );
        let draft = reservation(EditorialActionKind::Draft, "scene:1", &settings);
        ledger.admit(draft.clone()).expect("admitted");
        ledger.reconcile(
            &draft.idempotency_key,
            Some(EditorialBudgetUsage {
                calls: 1,
                tokens: 1_000,
                millis: 900,
                usd_micros: 100,
            }),
            vec!["model_call:1".into()],
            2,
        );
        assert_eq!(ledger.scene_used().tokens, 1_000);
        assert_eq!(ledger.run_used().tokens, 1_000);
    }

    #[test]
    fn successful_nonzero_price_judging_keeps_cost_and_duration_charged() {
        let mut settings = settings_with_price();
        settings.pricing = Some(EditorialPricingV1 {
            input_usd_micros_per_million: 2_000_000,
            output_usd_micros_per_million: 4_000_000,
        });
        let mut ledger = EditorialBudgetLedgerV1::new(
            EditorialBudgetCaps::scene_default(),
            EditorialBudgetCaps::run_default(),
        );
        let judge = reservation(EditorialActionKind::Judge, "scene:1", &settings);
        ledger.admit(judge.clone()).expect("judge admitted");
        let actual = EditorialBudgetUsage {
            calls: 2,
            tokens: 3_000,
            millis: 12_345,
            usd_micros: 7_000,
        };
        ledger.reconcile(
            &judge.idempotency_key,
            Some(actual),
            vec!["model_call:judge1".into(), "model_call:judge2".into()],
            2,
        );
        let used = ledger.scene_used();
        assert_eq!(used.usd_micros, 7_000, "known cost must not be refunded");
        assert_eq!(used.millis, 12_345, "measured time must be charged");
        assert_eq!(used.calls, 2);
        assert_eq!(used.tokens, 3_000);
        assert_eq!(ledger.run_used(), used);
    }

    #[test]
    fn paid_dispatch_without_price_is_denied() {
        let settings = ResolvedEditorialSettingsV1::critique_v1_default();
        let mut ledger = EditorialBudgetLedgerV1::new(
            EditorialBudgetCaps::scene_default(),
            EditorialBudgetCaps::run_default(),
        );
        let mut draft = reservation(EditorialActionKind::Draft, "scene:1", &settings);
        draft.paid_dispatch = true;
        draft.pricing_known = false;
        assert_eq!(ledger.admit(draft), Err(BudgetDenial::MissingPrice));
    }

    #[test]
    fn next_action_progresses_through_stages() {
        let settings = settings_with_price();
        let mut scene = EditorialSceneStateV1 {
            chapter_number: 1,
            scene_order: 1,
            scene_id: Some("scene:1".into()),
            packet_hash: "packet:1".into(),
            source_packet_sha256: "source:1".into(),
            draft_artifact_path: None,
            candidate_artifact_path: None,
            loop_artifact_path: None,
            stage: EditorialStage::PacketReady,
            attempt: 0,
            incumbent: None,
            candidate: None,
            critic: None,
            judge: None,
            selected_directives: Vec::new(),
            strengths_to_preserve: Vec::new(),
            last_edit_span_summary: None,
            parked_issues: Vec::new(),
            pending_host_task: None,
            pending_manual_judgment: None,
            availability: EditorialRouteAvailability::all_available(),
            ledger: EditorialBudgetLedgerV1::new(
                EditorialBudgetCaps::scene_default(),
                EditorialBudgetCaps::run_default(),
            ),
            attribution_event_ids: Vec::new(),
            line_edit_attempted: false,
            extracted: false,
        };
        assert_eq!(
            editorial_next_action(&scene, &settings)
                .expect("action")
                .action,
            EditorialActionKind::Draft
        );
        scene.stage = EditorialStage::Drafted;
        assert_eq!(
            editorial_next_action(&scene, &settings)
                .expect("action")
                .action,
            EditorialActionKind::DeterministicCheck
        );
        scene.stage = EditorialStage::DeterministicChecked;
        assert_eq!(
            editorial_next_action(&scene, &settings)
                .expect("action")
                .action,
            EditorialActionKind::Critique
        );
        scene.stage = EditorialStage::Critiqued;
        scene.selected_directives = vec![];
        assert!(
            editorial_next_action(&scene, &settings).is_none(),
            "critique with no actionable directives is terminal"
        );
        scene.availability.review = false;
        scene.stage = EditorialStage::CandidateChecked;
        let action = editorial_next_action(&scene, &settings).expect("action");
        assert_eq!(action.action, EditorialActionKind::Judge);
        assert_eq!(action.missing_route.as_deref(), Some("review"));
    }

    #[test]
    fn run_call_cap_is_shared_across_scenes() {
        let settings = settings_with_price();
        let mut controller = EditorialControllerStateV1::new(
            "run:1".into(),
            "project:1".into(),
            "branch:1".into(),
            settings.clone(),
        );
        let run_caps = EditorialBudgetCaps {
            max_calls: 1,
            ..EditorialBudgetCaps::run_default()
        };
        for scene_id in ["scene:1", "scene:2"] {
            let scene = EditorialSceneStateV1 {
                chapter_number: 1,
                scene_order: controller.scenes.len() as u32 + 1,
                scene_id: Some(scene_id.into()),
                packet_hash: format!("packet:{scene_id}"),
                source_packet_sha256: "source".into(),
                draft_artifact_path: None,
                candidate_artifact_path: None,
                loop_artifact_path: None,
                stage: EditorialStage::PacketReady,
                attempt: 0,
                incumbent: None,
                candidate: None,
                critic: None,
                judge: None,
                selected_directives: Vec::new(),
                strengths_to_preserve: Vec::new(),
                last_edit_span_summary: None,
                parked_issues: Vec::new(),
                pending_host_task: None,
                pending_manual_judgment: None,
                availability: EditorialRouteAvailability::all_available(),
                ledger: EditorialBudgetLedgerV1::new(
                    EditorialBudgetCaps::scene_default(),
                    run_caps,
                ),
                attribution_event_ids: Vec::new(),
                line_edit_attempted: false,
                extracted: false,
            };
            controller.scenes.push(scene);
        }
        crate::drafting::editorial::tests_support::admit_for(&mut controller, 0, &settings)
            .expect("first scene admits");
        let denial =
            crate::drafting::editorial::tests_support::admit_for(&mut controller, 1, &settings)
                .expect_err("shared run cap denies the second scene");
        assert!(
            matches!(
                denial,
                BudgetDenial::RunCalls {
                    requested: 1,
                    used: 1,
                    cap: 1
                }
            ),
            "unexpected denial: {denial:?}"
        );
    }

    #[test]
    fn legacy_settings_have_no_editorial_action() {
        let settings = ResolvedEditorialSettingsV1::legacy();
        let scene = EditorialSceneStateV1 {
            chapter_number: 1,
            scene_order: 1,
            scene_id: None,
            packet_hash: "packet:1".into(),
            source_packet_sha256: "source:1".into(),
            draft_artifact_path: None,
            candidate_artifact_path: None,
            loop_artifact_path: None,
            stage: EditorialStage::PacketReady,
            attempt: 0,
            incumbent: None,
            candidate: None,
            critic: None,
            judge: None,
            selected_directives: Vec::new(),
            strengths_to_preserve: Vec::new(),
            last_edit_span_summary: None,
            parked_issues: Vec::new(),
            pending_host_task: None,
            pending_manual_judgment: None,
            availability: EditorialRouteAvailability::default(),
            ledger: EditorialBudgetLedgerV1::new(
                EditorialBudgetCaps::scene_default(),
                EditorialBudgetCaps::run_default(),
            ),
            attribution_event_ids: Vec::new(),
            line_edit_attempted: false,
            extracted: false,
        };
        assert!(editorial_next_action(&scene, &settings).is_none());
    }

    #[test]
    fn settings_validation_rejects_over_hard_caps() {
        let mut settings = ResolvedEditorialSettingsV1::critique_v1_default();
        settings.max_rounds = HARD_EDITORIAL_MAX_ROUNDS + 1;
        assert!(settings.validate().is_err());
        let mut settings = ResolvedEditorialSettingsV1::critique_v1_default();
        settings.scene_budget.max_calls = DEFAULT_SCENE_CALL_CAP + 1;
        assert!(settings.validate().is_err());
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub(crate) fn admit_for(
        controller: &mut EditorialControllerStateV1,
        scene_index: usize,
        settings: &ResolvedEditorialSettingsV1,
    ) -> Result<EditorialBudgetReservation, BudgetDenial> {
        let prior = controller.run_usage_excluding(scene_index);
        let scene = &mut controller.scenes[scene_index];
        let key = EditorialReservationKey {
            run_id: controller.run_id.clone(),
            branch_id: controller.branch_id.clone(),
            scene_id: scene.scene_id.clone(),
            packet_hash: scene.packet_hash.clone(),
            stage: scene.stage,
            attempt: scene.attempt,
            action: EditorialActionKind::Draft,
        };
        let reservation = EditorialBudgetReservation {
            reservation_id: format!("reservation:{}", key.idempotency_key()),
            idempotency_key: key.idempotency_key(),
            key,
            reserved: reservation_usage(EditorialActionKind::Draft, settings),
            paid_dispatch: false,
            pricing_known: settings.pricing_known(),
            usage_known: false,
            actual: None,
            model_call_ids: Vec::new(),
            dispatched_at_millis: None,
            created_at_millis: 1,
            reconciled_at_millis: None,
        };
        scene.ledger.admit_with_prior_run_usage(reservation, prior)
    }
}
