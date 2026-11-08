//! Pinned, calibrated text-only model tokenization. No estimated fallback.
use crate::drafting::{DraftMessages, DraftTokenizer};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const ENGINE_VERSION: &str = "hf-tokenizers-0.22.2/v1";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeMode {
    LocalZeroOutput,
    CloudOneToken,
}
impl ProbeMode {
    pub fn output_limit(self) -> u32 {
        match self {
            Self::LocalZeroOutput => 0,
            Self::CloudOneToken => 1,
        }
    }
}

/// Fixed two-message framing, copied from a pinned model template. Concatenation
/// deliberately does not interpret placeholders occurring inside story text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatFraming {
    pub prefix: String,
    pub between: String,
    pub suffix: String,
    #[serde(default)]
    pub trim_system: bool,
    #[serde(default)]
    pub trim_user: bool,
}
impl ChatFraming {
    pub fn render(&self, messages: &DraftMessages) -> String {
        let system = if self.trim_system {
            messages.system.trim()
        } else {
            &messages.system
        };
        let user = if self.trim_user {
            messages.user.trim()
        } else {
            &messages.user
        };
        format!(
            "{}{system}{}{user}{}",
            self.prefix, self.between, self.suffix
        )
    }
}
fn default_true() -> bool {
    true
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenizerConfig {
    pub tokenizer_json: PathBuf,
    pub tokenizer_sha256: String,
    pub model_revision: String,
    pub framing: ChatFraming,
    pub calibration_file: PathBuf,
    pub cache_dir: PathBuf,
    pub probe_mode: ProbeMode,
    #[serde(default)]
    pub tolerance_tokens: u32,
    #[serde(default = "default_true")]
    pub verify_each_prompt: bool,
}
impl TokenizerConfig {
    pub fn resolved(&self, base: &Path) -> Self {
        let resolve = |path: &PathBuf| {
            if path.is_absolute() {
                path.clone()
            } else {
                base.join(path)
            }
        };
        Self {
            tokenizer_json: resolve(&self.tokenizer_json),
            calibration_file: resolve(&self.calibration_file),
            cache_dir: resolve(&self.cache_dir),
            ..self.clone()
        }
    }
}
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn messages_hash(messages: &DraftMessages) -> Result<String> {
    Ok(sha256(&serde_json::to_vec(messages)?))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeReceipt {
    pub model: String,
    pub messages_hash: String,
    pub input_tokens: usize,
    pub output_tokens: u32,
    pub model_call_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationCertificate {
    pub provider_identity: String,
    pub engine: String,
    pub model: String,
    pub model_revision: String,
    pub tokenizer_sha256: String,
    pub framing_sha256: String,
    pub tolerance_tokens: u32,
    pub samples: Vec<ProbeReceipt>,
}
/// Synthetic, non-manuscript probes cover role boundaries, Unicode, whitespace,
/// literal placeholders and a longer input. Order is part of certificate v1.
pub fn calibration_messages() -> Vec<DraftMessages> {
    vec![
        DraftMessages {
            system: "Follow the instruction.".into(),
            user: "Reply with OK.".into(),
        },
        DraftMessages {
            system: "日本語とfrançais。".into(),
            user: "雪 🧵 café e\u{301}\nReply OK.".into(),
        },
        DraftMessages {
            system: "  Preserve literal {{user}}.\r\n".into(),
            user: "\n {{system}} <test> \t".into(),
        },
        DraftMessages {
            system: "Token calibration only.".into(),
            user: "The river crossed the city. ".repeat(128),
        },
    ]
}

pub struct PinnedTokenizer {
    model: String,
    config: TokenizerConfig,
    encoder: tokenizers::Tokenizer,
    framing_hash: String,
}
impl PinnedTokenizer {
    pub fn load(model: &str, config: TokenizerConfig) -> Result<Self> {
        ensure!(
            !model.is_empty() && !config.model_revision.is_empty(),
            "tokenizer model/revision required"
        );
        ensure!(
            config.tolerance_tokens <= 16,
            "calibration tolerance exceeds 16 tokens"
        );
        let bytes = std::fs::read(&config.tokenizer_json).context("read pinned tokenizer.json")?;
        ensure!(
            bytes.len() <= 128 * 1024 * 1024,
            "tokenizer.json exceeds asset limit"
        );
        ensure!(
            sha256(&bytes) == config.tokenizer_sha256,
            "tokenizer checksum mismatch"
        );
        let encoder = tokenizers::Tokenizer::from_bytes(&bytes)
            .map_err(|e| anyhow::anyhow!("invalid tokenizer.json: {e}"))?;
        ensure!(
            encoder.get_truncation().is_none(),
            "truncating tokenizer is not permitted"
        );
        ensure!(
            encoder.get_padding().is_none(),
            "padding tokenizer is not permitted"
        );
        let framing_hash = sha256(&serde_json::to_vec(&config.framing)?);
        Ok(Self {
            model: model.into(),
            config,
            encoder,
            framing_hash,
        })
    }
    pub fn count_text(&self, text: &str) -> Result<usize> {
        self.encoder
            .encode(text, false)
            .map(|e| e.len())
            .map_err(|e| anyhow::anyhow!("tokenizer encode failed: {e}"))
    }
    pub fn count_messages(&self, messages: &DraftMessages) -> Result<usize> {
        self.count_text(&self.config.framing.render(messages))
    }
    pub fn certificate(
        &self,
        provider_identity: &str,
        samples: Vec<ProbeReceipt>,
    ) -> Result<CalibrationCertificate> {
        let cert = CalibrationCertificate {
            provider_identity: provider_identity.into(),
            engine: ENGINE_VERSION.into(),
            model: self.model.clone(),
            model_revision: self.config.model_revision.clone(),
            tokenizer_sha256: self.config.tokenizer_sha256.clone(),
            framing_sha256: self.framing_hash.clone(),
            tolerance_tokens: self.config.tolerance_tokens,
            samples,
        };
        self.validate_certificate(&cert, provider_identity)?;
        Ok(cert)
    }
    fn validate_certificate(
        &self,
        cert: &CalibrationCertificate,
        provider_identity: &str,
    ) -> Result<()> {
        ensure!(
            !provider_identity.is_empty()
                && cert.provider_identity == provider_identity
                && cert.engine == ENGINE_VERSION
                && cert.model == self.model
                && cert.model_revision == self.config.model_revision
                && cert.tokenizer_sha256 == self.config.tokenizer_sha256
                && cert.framing_sha256 == self.framing_hash
                && cert.tolerance_tokens == self.config.tolerance_tokens,
            "calibration certificate identity mismatch"
        );
        let messages = calibration_messages();
        ensure!(
            cert.samples.len() == messages.len(),
            "incomplete calibration coverage"
        );
        for (sample, message) in cert.samples.iter().zip(&messages) {
            ensure!(
                sample.model == self.model
                    && sample.messages_hash == messages_hash(message)?
                    && !sample.model_call_id.is_empty(),
                "calibration sample binding mismatch"
            );
            ensure!(
                sample.input_tokens > 0
                    && sample.output_tokens <= self.config.probe_mode.output_limit(),
                "invalid calibration count proof"
            );
            self.verify_count(message, sample.input_tokens)?;
        }
        Ok(())
    }
    pub fn verify_count(&self, messages: &DraftMessages, provider_count: usize) -> Result<()> {
        let local = self.count_messages(messages)?;
        ensure!(
            local.abs_diff(provider_count) <= self.config.tolerance_tokens as usize,
            "calibration mismatch: local={local} provider={provider_count} tolerance={}",
            self.config.tolerance_tokens
        );
        Ok(())
    }
    pub fn verify(
        self,
        certificate: &CalibrationCertificate,
        provider_identity: &str,
    ) -> Result<CalibratedTokenizer> {
        self.validate_certificate(certificate, provider_identity)?;
        let version = format!(
            "{ENGINE_VERSION}:{}:{}:tolerance={}",
            self.config.model_revision, self.framing_hash, self.config.tolerance_tokens
        );
        Ok(CalibratedTokenizer {
            inner: self,
            version,
        })
    }
    pub fn load_verified(
        model: &str,
        config: TokenizerConfig,
        provider_identity: &str,
    ) -> Result<CalibratedTokenizer> {
        let cert: CalibrationCertificate = serde_json::from_reader(
            std::fs::File::open(&config.calibration_file)
                .context("tokenizer calibration required; run spindle-tokenizer calibrate")?,
        )?;
        Self::load(model, config)?.verify(&cert, provider_identity)
    }
}
pub struct CalibratedTokenizer {
    inner: PinnedTokenizer,
    version: String,
}
impl CalibratedTokenizer {
    pub fn verify_count(&self, messages: &DraftMessages, provider_count: usize) -> Result<()> {
        self.inner.verify_count(messages, provider_count)
    }
    pub fn config(&self) -> &TokenizerConfig {
        &self.inner.config
    }
}
impl DraftTokenizer for CalibratedTokenizer {
    fn id(&self) -> &str {
        &self.inner.config.tokenizer_sha256
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn supports_model(&self, model: &str) -> bool {
        model == self.inner.model
    }
    fn count_text(&self, text: &str) -> Result<usize, String> {
        self.inner.count_text(text).map_err(|e| e.to_string())
    }
    fn count_messages(&self, messages: &DraftMessages) -> Result<usize, String> {
        self.inner
            .count_messages(messages)
            .and_then(|n| {
                n.checked_add(self.inner.config.tolerance_tokens as usize)
                    .context("token count overflow")
            })
            .map_err(|e| e.to_string())
    }
}
#[cfg(test)]
pub(crate) mod tests;

impl std::fmt::Debug for CalibratedTokenizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalibratedTokenizer")
            .field("model", &self.inner.model)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
