//! Self-hosted sampler / FTPO status.
//!
//! Spindle's `draft` route is an LLM chat completion (HTTP or builtin-local
//! stub), not a preference-pair trainer. A half-baked in-process sampler is
//! out of scope; operators who want FTPO run it out of band.

/// Documented status of an in-process self-hosted sampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerStatus {
    kind: &'static str,
}

impl SamplerStatus {
    pub const OUT_OF_BAND: Self = Self {
        kind: "out_of_band",
    };

    pub fn as_str(self) -> &'static str {
        self.kind
    }

    pub fn in_process(self) -> bool {
        false
    }
}

pub fn self_hosted_sampler_status() -> SamplerStatus {
    SamplerStatus::OUT_OF_BAND
}
