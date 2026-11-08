use super::*;
use spindle_core::drafting::extraction::{
    AcceptedTextExtractionRequestV1, AcceptedTextExtractor, CompatibilityExtractionV1,
    ExtractionFailure,
};
use std::{
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    pin::Pin,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedTaskV1 {
    schema_version: u32,
    prompt_version: String,
    task_id: String,
    request_sha256: String,
    result: Option<CompatibilityExtractionV1>,
}
struct TaskLock(PathBuf);
impl Drop for TaskLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
enum Preparation {
    Cached(Box<CompatibilityExtractionV1>),
    Reserved(Box<TaskReservation>),
}
struct TaskReservation {
    path: PathBuf,
    cached: CachedTaskV1,
    _lock: TaskLock,
}
fn failed(code: &str, retryable: bool) -> ExtractionFailure {
    ExtractionFailure {
        code: code.into(),
        retryable,
    }
}
fn request_hash(request: &AcceptedTextExtractionRequestV1) -> Result<String, ExtractionFailure> {
    serde_json::to_string(request)
        .map(|s| exact_text_sha256(&s))
        .map_err(|_| failed("invalid_request", false))
}
fn validate_request(request: &AcceptedTextExtractionRequestV1) -> Result<(), ExtractionFailure> {
    let accepted = &request.accepted;
    if request.task_id.trim().is_empty()
        || accepted.packet_hash.trim().is_empty()
        || accepted.receipt.generation_id.trim().is_empty()
        || accepted.receipt.agent_id.trim().is_empty()
        || exact_text_sha256(accepted.text.replace("\r\n", "\n").trim())
            != accepted.receipt.normalized_output_sha256
    {
        return Err(failed("invalid_receipt_binding", false));
    }
    let mut ids = BTreeSet::new();
    for entity in &request.known_entities {
        if entity.id.trim().is_empty()
            || entity.kind.trim().is_empty()
            || entity.display_name.trim().is_empty()
            || !ids.insert(&entity.id)
        {
            return Err(failed("invalid_entity_context", false));
        }
    }
    Ok(())
}
fn binding(request: &AcceptedTextExtractionRequestV1) -> ClaimBindingV1 {
    let accepted = &request.accepted;
    ClaimBindingV1 {
        schema_version: CLAIM_SCHEMA_VERSION,
        project_id: accepted.project_id.clone(),
        branch_id: accepted.branch_id.clone(),
        scene_id: accepted.scene_id.clone(),
        revision_id: accepted.artifact_id.clone(),
        accepted_text_sha256: accepted.text_sha256.clone(),
        position: accepted.placement.clone(),
        extraction_run_id: format!("{}:{EXTRACT_PROMPT_VERSION}", request.task_id),
        model: "pending".into(),
        agent_id: None,
        model_call_id: None,
        prompt_version: EXTRACT_PROMPT_VERSION.into(),
    }
}
/// Revalidate a durable report against the complete frozen P1 request on load.
pub(crate) fn validate_audit_for_request(
    audit: &CanonExtractionAuditV1,
    request: &AcceptedTextExtractionRequestV1,
) -> Result<(), ExtractionFailure> {
    validate_request(request)?;
    if audit.task_id != request.task_id || audit.request_sha256 != request_hash(request)? {
        return Err(failed("audit_request_mismatch", false));
    }
    let report = &audit.report;
    let mut expected = binding(request);
    expected.model = report.binding.model.clone();
    expected.agent_id = report.binding.agent_id.clone();
    expected.model_call_id = report.binding.model_call_id.clone();
    if report.binding != expected
        || report
            .claims
            .iter()
            .any(|c| c.source != ClaimSource::Independent || c.binding != report.binding)
    {
        return Err(failed("audit_identity_mismatch", false));
    }
    let payload = ExtractionPayloadV1 {
        schema_version: report.binding.schema_version,
        accepted_text_sha256: report.binding.accepted_text_sha256.clone(),
        summary: report.summary.clone(),
        tone: report.tone.clone(),
        coverage: report.coverage.clone(),
        claims: report.claims.iter().map(|c| c.proposed.clone()).collect(),
    };
    let known = request
        .known_entities
        .iter()
        .map(|e| e.id.clone())
        .collect();
    bind_extraction(
        payload,
        report.binding.clone(),
        &request.accepted.text,
        &known,
    )
    .map(|_| ())
    .map_err(|_| failed("invalid_audit_claims", false))
}
fn shadow_projection(audit: CanonExtractionAuditV1) -> CompatibilityExtractionV1 {
    let report = &audit.report;
    let unknown = report
        .coverage
        .values()
        .filter(|s| matches!(s, FamilyCoverage::Unknown { .. }))
        .count();
    CompatibilityExtractionV1 {
        accepted_text_sha256: report.binding.accepted_text_sha256.clone(),
        summary: report.summary.clone(),
        tone: report.tone.clone(),
        character_states: Vec::new(),
        canonical_facts: Vec::new(),
        relationship_updates: Vec::new(),
        beats: Vec::new(),
        continuity_notes: vec![format!(
            "Independent extraction shadow: {} proposed claims; families with unknown coverage: {unknown}; no canon changes applied.",
            report.claims.len()
        )],
        canon_audit: Some(audit),
    }
}
pub(crate) fn validate_shadow_projection(
    result: &CompatibilityExtractionV1,
    request: &AcceptedTextExtractionRequestV1,
) -> Result<(), ExtractionFailure> {
    let audit = result
        .canon_audit
        .as_ref()
        .ok_or_else(|| failed("cache_missing_audit", false))?;
    validate_audit_for_request(audit, request)?;
    let expected = shadow_projection(audit.clone());
    if serde_json::to_vec(result).map_err(|_| failed("cache_invalid", false))?
        != serde_json::to_vec(&expected).map_err(|_| failed("cache_invalid", false))?
    {
        return Err(failed("cache_projection_mismatch", false));
    }
    Ok(())
}
/// Implements P1's consumer port. Its configured directory is a local derived
/// cache; filenames are hashes of task IDs, never supplied paths or scene text.
pub struct IndependentCanonExtractor {
    router: ModelRouter,
    cache_directory: PathBuf,
}
impl IndependentCanonExtractor {
    pub fn new(router: ModelRouter, cache_directory: impl Into<PathBuf>) -> Self {
        Self {
            router,
            cache_directory: cache_directory.into(),
        }
    }
    fn read_cache(
        &self,
        path: &Path,
        request: &AcceptedTextExtractionRequestV1,
        hash: &str,
    ) -> Result<Option<CompatibilityExtractionV1>, ExtractionFailure> {
        let metadata = match std::fs::metadata(path) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(failed("cache_unavailable", true)),
        };
        if metadata.len() > 1_000_000 {
            return Err(failed("cache_invalid", false));
        }
        let cached: CachedTaskV1 = serde_json::from_reader(
            std::fs::File::open(path).map_err(|_| failed("cache_unavailable", true))?,
        )
        .map_err(|_| failed("cache_invalid", false))?;
        if cached.schema_version != CLAIM_SCHEMA_VERSION
            || cached.prompt_version != EXTRACT_PROMPT_VERSION
        {
            return Err(failed("extractor_version_changed", false));
        }
        if cached.task_id != request.task_id || cached.request_sha256 != hash {
            return Err(failed("task_identity_changed", false));
        }
        if let Some(result) = &cached.result {
            validate_shadow_projection(result, request)?;
        }
        Ok(cached.result)
    }
    fn save_cache(&self, path: &Path, value: &CachedTaskV1) -> Result<(), ExtractionFailure> {
        let bytes = serde_json::to_vec(value).map_err(|_| failed("cache_unavailable", true))?;
        if bytes.len() > 1_000_000 {
            return Err(failed("cache_report_too_large", false));
        }
        let mut file = tempfile::NamedTempFile::new_in(&self.cache_directory)
            .map_err(|_| failed("cache_unavailable", true))?;
        file.write_all(&bytes)
            .map_err(|_| failed("cache_unavailable", true))?;
        file.flush()
            .map_err(|_| failed("cache_unavailable", true))?;
        file.as_file()
            .sync_all()
            .map_err(|_| failed("cache_unavailable", true))?;
        file.persist(path)
            .map_err(|_| failed("cache_unavailable", true))?;
        Ok(())
    }
    fn prepare_task(
        &self,
        request: &AcceptedTextExtractionRequestV1,
    ) -> Result<Preparation, ExtractionFailure> {
        validate_request(request)?;
        let binding = binding(request);
        validate_binding(&binding, &request.accepted.text)
            .map_err(|_| failed("invalid_accepted_text", false))?;
        let hash = request_hash(request)?;
        let filename = exact_text_sha256(&request.task_id);
        let path = self.cache_directory.join(format!("{filename}.json"));
        if let Some(result) = self.read_cache(&path, request, &hash)? {
            return Ok(Preparation::Cached(Box::new(result)));
        }
        std::fs::create_dir_all(&self.cache_directory)
            .map_err(|_| failed("cache_unavailable", true))?;
        let lock_path = self.cache_directory.join(format!("{filename}.lock"));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|e| {
                failed(
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        "extraction_busy_or_recovery_needed"
                    } else {
                        "cache_unavailable"
                    },
                    true,
                )
            })?;
        let lock = TaskLock(lock_path);
        if let Some(result) = self.read_cache(&path, request, &hash)? {
            return Ok(Preparation::Cached(Box::new(result)));
        }
        let cached = CachedTaskV1 {
            schema_version: CLAIM_SCHEMA_VERSION,
            prompt_version: EXTRACT_PROMPT_VERSION.into(),
            task_id: request.task_id.clone(),
            request_sha256: hash.clone(),
            result: None,
        };
        self.save_cache(&path, &cached)?;
        Ok(Preparation::Reserved(Box::new(TaskReservation {
            path,
            cached,
            _lock: lock,
        })))
    }
    fn finish_task(
        &self,
        mut reservation: TaskReservation,
        request: &AcceptedTextExtractionRequestV1,
        report: ExtractionReportV1,
    ) -> Result<CompatibilityExtractionV1, ExtractionFailure> {
        let result = shadow_projection(CanonExtractionAuditV1 {
            task_id: request.task_id.clone(),
            request_sha256: reservation.cached.request_sha256.clone(),
            report,
        });
        validate_shadow_projection(&result, request)?;
        reservation.cached.result = Some(result.clone());
        self.save_cache(&reservation.path, &reservation.cached)?;
        Ok(result)
    }
    /// Accept a host's independent report for a pending task through the same
    /// evidence gate and durable cache. Completed reports remain immutable.
    pub fn submit_host_report(
        &self,
        request: &AcceptedTextExtractionRequestV1,
        payload: ExtractionPayloadV1,
    ) -> Result<CompatibilityExtractionV1, ExtractionFailure> {
        validate_request(request)?;
        let mut binding = binding(request);
        binding.model = "host".into();
        let known = request
            .known_entities
            .iter()
            .map(|e| e.id.clone())
            .collect();
        let report = accept_host_extraction(payload, binding, &request.accepted.text, &known)
            .map_err(|_| failed("invalid_host_claims", false))?;
        match self.prepare_task(request)? {
            Preparation::Cached(result) => {
                if serde_json::to_vec(
                    &result
                        .canon_audit
                        .as_ref()
                        .expect("validated cache has audit")
                        .report,
                )
                .map_err(|_| failed("cache_invalid", false))?
                    != serde_json::to_vec(&report)
                        .map_err(|_| failed("invalid_host_claims", false))?
                {
                    return Err(failed("task_already_completed", false));
                }
                Ok(*result)
            }
            Preparation::Reserved(reservation) => self.finish_task(*reservation, request, report),
        }
    }
    async fn run(
        &self,
        request: &AcceptedTextExtractionRequestV1,
    ) -> Result<CompatibilityExtractionV1, ExtractionFailure> {
        let reservation = match self.prepare_task(request)? {
            Preparation::Cached(result) => return Ok(*result),
            Preparation::Reserved(reservation) => reservation,
        };
        let binding = binding(request);
        let context = CanonExtractionContext {
            rating: request.accepted.rating.clone(),
            packet_hash: request.accepted.packet_hash.clone(),
            known_entities: request
                .known_entities
                .iter()
                .map(|e| (e.id.clone(), vec![e.display_name.clone()]))
                .collect(),
            preceding_ratified_canon: request.selected_canon.clone(),
        };
        let report =
            extract_shadow_claims(&self.router, &binding, &request.accepted.text, &context)
                .await
                .map_err(|error| {
                    failed(
                        &format!(
                            "extraction_{}_{}",
                            match error.state {
                                ExtractionFailureState::Pending => "pending",
                                ExtractionFailureState::Failed => "failed",
                            },
                            error.code.as_str()
                        ),
                        error.retryable,
                    )
                })?;
        self.finish_task(*reservation, request, report)
    }
}
impl AcceptedTextExtractor for IndependentCanonExtractor {
    fn extract<'a>(
        &'a self,
        request: &'a AcceptedTextExtractionRequestV1,
    ) -> Pin<
        Box<dyn Future<Output = Result<CompatibilityExtractionV1, ExtractionFailure>> + Send + 'a>,
    > {
        Box::pin(self.run(request))
    }
}
