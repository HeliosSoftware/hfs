//! Terminology functions for FHIRPath %terminologies object
//!
//! This module implements the %terminologies functions defined in the FHIRPath specification,
//! enabling interaction with FHIR terminology servers for ValueSet expansion, code validation,
//! and concept mapping operations.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::{Builder, Handle, Runtime, RuntimeFlavor};

use serde_json::Value;

use crate::evaluator::EvaluationContext;
use crate::terminology_client::{TerminologyClient, is_transient};
use helios_fhir::FhirVersion;
use helios_fhirpath_support::{EvaluationError, EvaluationResult};

/// Name of the shared runtime's worker threads; tests assert on it.
const RUNTIME_THREAD_NAME: &str = "fhirpath-terminology";

lazy_static::lazy_static! {
    /// Lazy static for async runtime
    /// Used to execute async terminology operations in sync context
    static ref RUNTIME: Runtime = Builder::new_multi_thread()
        .enable_all()
        .thread_name(RUNTIME_THREAD_NAME)
        .build()
        .expect("Failed to create tokio runtime");
}

/// Helper function to execute async operations in both sync and async contexts
///
/// Both paths drive the future on the one shared [`RUNTIME`]; no runtime is built per
/// call. Inside a caller's runtime the future is spawned onto the shared runtime and the
/// calling thread waits on a channel, using `block_in_place` where the caller's runtime
/// is multi-threaded (it panics on a current-thread runtime, where plain blocking is safe
/// because the future runs on the shared runtime's own workers).
///
/// On a current-thread caller runtime the plain `recv()` blocks that runtime's only thread
/// for the whole request, so other tasks on it stall. Such callers should evaluate on
/// `spawn_blocking`.
///
/// On both paths a panicking request future is returned as an `Err`, not a panic.
fn block_on_async<F, T>(future: F) -> Result<T, EvaluationError>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    // Not in async context, safe to use the static runtime directly
    let Ok(current) = Handle::try_current() else {
        return RUNTIME
            .block_on(RUNTIME.spawn(future))
            .map_err(|_| request_task_failed());
    };

    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    RUNTIME.spawn(async move {
        let _ = tx.send(future.await);
    });

    let received = if matches!(current.runtime_flavor(), RuntimeFlavor::MultiThread) {
        tokio::task::block_in_place(|| rx.recv())
    } else {
        rx.recv()
    };
    // A dropped sender means the spawned future panicked or was cancelled before sending
    received.map_err(|_| request_task_failed())
}

/// Error for a request future that panicked or was cancelled before returning a result.
fn request_task_failed() -> EvaluationError {
    EvaluationError::InvalidOperation(
        "Terminology request task panicked or was cancelled before returning a result \
         (internal error)"
            .to_string(),
    )
}

/// Default cap on terminology server calls per [`TerminologySession`].
const DEFAULT_MAX_CALLS: usize = 1000;

/// Environment variable overriding [`DEFAULT_MAX_CALLS`].
const MAX_CALLS_ENV: &str = "FHIRPATH_TERMINOLOGY_MAX_CALLS";

/// Reads the terminology call cap from `FHIRPATH_TERMINOLOGY_MAX_CALLS`.
///
/// `None` means the cap is disabled.
fn max_calls_from_env() -> Option<usize> {
    parse_max_calls(std::env::var(MAX_CALLS_ENV).ok().as_deref())
}

/// Parses a `FHIRPATH_TERMINOLOGY_MAX_CALLS` value.
///
/// An unset variable gives the default; `0` disables the cap (`None`); an unparseable
/// value falls back to the default, so a typo never removes the bound.
fn parse_max_calls(raw: Option<&str>) -> Option<usize> {
    match raw.map(|v| v.trim().parse::<usize>()) {
        None | Some(Err(_)) => Some(DEFAULT_MAX_CALLS),
        Some(Ok(0)) => None,
        Some(Ok(n)) => Some(n),
    }
}

/// Error returned when a session has used up its terminology call budget.
fn call_limit_exceeded(limit: usize) -> EvaluationError {
    EvaluationError::TerminologyCallLimit(format!(
        "Terminology call limit reached: this evaluation has already made {limit} \
         terminology server call(s), the maximum allowed by {MAX_CALLS_ENV} (default \
         {DEFAULT_MAX_CALLS}; 0 disables the limit). Repeated identical lookups are \
         answered from cache and do not count; raise the limit or reduce the number of \
         distinct lookups."
    ))
}

/// Identifies one terminology lookup: the operation, the server, every
/// request-shaping argument and the extra parameters.
#[derive(Clone, PartialEq, Eq, Hash)]
struct LookupKey {
    operation: &'static str,
    server: String,
    fhir_version: FhirVersion,
    args: Vec<Option<String>>,
    /// Extra parameters sorted by name; absent and empty maps are both empty.
    params: Vec<(String, String)>,
}

impl LookupKey {
    fn new(
        operation: &'static str,
        server: &str,
        fhir_version: FhirVersion,
        args: Vec<Option<String>>,
        params: Option<&HashMap<String, String>>,
    ) -> Self {
        let mut params: Vec<(String, String)> = params
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        params.sort();
        Self {
            operation,
            server: server.to_string(),
            fhir_version,
            args,
            params,
        }
    }
}

/// A failed lookup: its error text, and whether sending the request again could succeed.
#[derive(Clone)]
struct Failure {
    message: String,
    transient: bool,
}

/// One lookup's outcome, filled once by whichever caller reaches it first; concurrent
/// callers of the same lookup wait on the cell instead of repeating the request.
type AnswerCell = Arc<std::sync::OnceLock<Result<Value, Failure>>>;

#[derive(Default)]
struct SessionState {
    /// Logical remote calls made so far: cache misses, including a retry of a lookup that
    /// failed transiently; the client's own transport retries count once.
    remote_calls: usize,
    /// Lookups started so far with their outcome once known; a transient failure is removed
    /// once read, so the next identical lookup retries.
    answers: HashMap<LookupKey, AnswerCell>,
    /// Client for the server URL and FHIR version in use, built on first need.
    client: Option<(String, FhirVersion, Arc<TerminologyClient>)>,
}

/// Request-scoped terminology state shared by an `EvaluationContext`, its clones and its
/// child contexts.
///
/// Identical lookups are answered once (concurrent identical lookups wait for the one in
/// flight), the number of remote calls is capped (`FHIRPATH_TERMINOLOGY_MAX_CALLS`), and
/// the HTTP client is built once. The session is dropped with the last context holding it,
/// so nothing outlives the request. Transient failures (timeouts, connection errors,
/// 408/429/5xx except 501) are not kept: the next identical lookup retries. Callers that
/// build several contexts for one request share a session with
/// [`EvaluationContext::set_terminology_session`].
#[derive(Default)]
pub struct TerminologySession {
    max_calls: std::sync::OnceLock<Option<usize>>,
    state: parking_lot::Mutex<SessionState>,
}

impl std::fmt::Debug for TerminologySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminologySession").finish_non_exhaustive()
    }
}

impl TerminologySession {
    #[cfg(test)]
    pub(crate) fn with_max_calls(limit: Option<usize>) -> Self {
        let session = Self::default();
        let _ = session.max_calls.set(limit);
        session
    }

    /// Drops a transiently failed lookup so the next identical lookup sends a new request,
    /// charged to the budget like any cache miss (`remote_calls` is never decremented). Only
    /// removes `cell` itself, so a newer cell for the same key is left alone.
    fn forget(&self, key: &LookupKey, cell: &AnswerCell) {
        let mut state = self.state.lock();
        if state
            .answers
            .get(key)
            .is_some_and(|stored| Arc::ptr_eq(stored, cell))
        {
            state.answers.remove(key);
        }
    }

    fn max_calls(&self) -> Option<usize> {
        *self.max_calls.get_or_init(max_calls_from_env)
    }

    /// Finds the lookup's cell, or reserves one remote call for a new one, and hands back
    /// the cell with the client to fill it with. A known lookup is not charged to the
    /// budget. The lock is never held across the network call.
    fn begin(
        &self,
        key: &LookupKey,
        server_url: &str,
        fhir_version: FhirVersion,
    ) -> Result<(AnswerCell, Arc<TerminologyClient>), EvaluationError> {
        let limit = self.max_calls();
        let mut state = self.state.lock();
        let known = state.answers.get(key).cloned();
        if known.is_none()
            && let Some(limit) = limit
            && state.remote_calls >= limit
        {
            return Err(call_limit_exceeded(limit));
        }

        let client = match &state.client {
            Some((url, version, client)) if url == server_url && *version == fhir_version => {
                client.clone()
            }
            _ => {
                let client = Arc::new(TerminologyClient::new(server_url.to_string(), fhir_version));
                state.client = Some((server_url.to_string(), fhir_version, client.clone()));
                client
            }
        };
        let cell = match known {
            Some(cell) => cell,
            None => {
                state.remote_calls += 1;
                let cell = AnswerCell::default();
                state.answers.insert(key.clone(), Arc::clone(&cell));
                cell
            }
        };
        Ok((cell, client))
    }
}

/// Terminology functions accessible via %terminologies
pub struct TerminologyFunctions {
    server_url: String,
    fhir_version: FhirVersion,
    session: Arc<TerminologySession>,
}

/// Error returned when a terminology operation is attempted with no server configured.
///
/// Terminology operations transmit codes taken from the resource under evaluation,
/// so there is no default server to fall back on — the caller must name one.
fn no_terminology_server() -> EvaluationError {
    EvaluationError::TerminologyError(
        "No terminology server is configured. Terminology operations (%terminologies.* \
         and memberOf()) send codes from the evaluated resource to a terminology server, \
         so no default server is used. Set the FHIRPATH_TERMINOLOGY_SERVER environment \
         variable, or pass --terminology-server (fhirpath-cli/fhirpath-server). The HFS \
         server and SQL-on-FHIR tools propagate HFS_TERMINOLOGY_SERVER and \
         SOF_TERMINOLOGY_SERVER respectively."
            .to_string(),
    )
}

impl TerminologyFunctions {
    /// Creates a new terminology functions instance
    ///
    /// # Errors
    ///
    /// Returns [`EvaluationError::TerminologyError`] if no terminology server is
    /// configured on the context or via `FHIRPATH_TERMINOLOGY_SERVER`.
    pub fn new(context: &EvaluationContext) -> Result<Self, EvaluationError> {
        let server_url = context
            .get_terminology_server_url()
            .ok_or_else(no_terminology_server)?;

        Ok(Self {
            server_url,
            fhir_version: context.fhir_version,
            session: context.terminology_session.clone(),
        })
    }

    /// Runs a lookup through the session: a known outcome is returned as is (waiting for
    /// it if another caller has the request in flight), otherwise one remote call is made.
    /// A success or a deterministic failure (4xx, invalid body, invalid URL) is kept; a
    /// transient failure (timeout, connection error, 408/429/5xx except 501) is handed to
    /// the callers that waited on it and then dropped, so the next identical lookup makes a
    /// new remote call, charged to the budget. Exceeding the budget is an `Err` and is not
    /// kept. Waiters use `block_in_place` on a multi-thread runtime so they do not hold a
    /// worker of the caller's runtime.
    fn cached<F, Fut>(
        &self,
        key: LookupKey,
        request: F,
    ) -> Result<Result<Value, String>, EvaluationError>
    where
        F: FnOnce(Arc<TerminologyClient>) -> Fut,
        Fut: std::future::Future<Output = crate::error::FhirPathResult<Value>> + Send + 'static,
    {
        let (cell, client) = self
            .session
            .begin(&key, &self.server_url, self.fhir_version)?;
        let answer = match cell.get() {
            Some(answer) => answer.clone(),
            None => {
                // Whichever caller gets here first makes the request; the others wait for it.
                let filling = Arc::clone(&cell);
                let fill = move || {
                    filling
                        .get_or_init(|| match block_on_async(request(client)) {
                            Ok(Ok(value)) => Ok(value),
                            Ok(Err(e)) => Err(Failure {
                                transient: is_transient(&e),
                                message: e.to_string(),
                            }),
                            Err(e) => Err(Failure {
                                transient: false,
                                message: e.to_string(),
                            }),
                        })
                        .clone()
                };
                let multi_thread = Handle::try_current()
                    .is_ok_and(|h| matches!(h.runtime_flavor(), RuntimeFlavor::MultiThread));
                if multi_thread {
                    tokio::task::block_in_place(fill)
                } else {
                    fill()
                }
            }
        };
        match answer {
            Ok(value) => Ok(Ok(value)),
            Err(failure) => {
                if failure.transient {
                    self.session.forget(&key, &cell);
                }
                Ok(Err(failure.message))
            }
        }
    }

    /// Expands a ValueSet
    ///
    /// Usage: %terminologies.expand(valueSet, params)
    pub fn expand(
        &self,
        value_set: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract ValueSet URL
        let value_set_url = match value_set {
            EvaluationResult::String(url, _, _) => url.clone(),
            _ => {
                return Err(EvaluationError::TypeError(
                    "expand() requires a ValueSet URL as string".to_string(),
                ));
            }
        };

        // Extract parameters if provided
        let params_map = extract_params_map(params)?;

        let key = LookupKey::new(
            "expand",
            &self.server_url,
            self.fhir_version,
            vec![Some(value_set_url.clone())],
            params_map.as_ref(),
        );
        let result = self.cached(key, move |client| async move {
            client.expand(&value_set_url, params_map).await
        })?;

        match result {
            Ok(value) => json_to_evaluation_result(value),
            Err(e) => Err(EvaluationError::TerminologyError(format!(
                "ValueSet expansion failed: {}",
                e
            ))),
        }
    }

    /// Looks up details for a code
    ///
    /// Usage: %terminologies.lookup(coded, params)
    pub fn lookup(
        &self,
        coded: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract system and code from Coding
        let (system, code) = extract_coding(coded)?;

        // Extract parameters if provided
        let params_map = extract_params_map(params)?;

        let key = LookupKey::new(
            "lookup",
            &self.server_url,
            self.fhir_version,
            vec![Some(system.clone()), Some(code.clone())],
            params_map.as_ref(),
        );
        let result = self.cached(key, move |client| async move {
            client.lookup(&system, &code, params_map).await
        })?;

        match result {
            Ok(value) => json_to_evaluation_result(value),
            Err(e) => Err(EvaluationError::TerminologyError(format!(
                "Code lookup failed: {}",
                e
            ))),
        }
    }

    /// Validates a code against a ValueSet
    ///
    /// Usage: %terminologies.validateVS(valueSet, coded, params)
    pub fn validate_vs(
        &self,
        value_set: &EvaluationResult,
        coded: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract ValueSet URL
        let value_set_url = match value_set {
            EvaluationResult::String(url, _, _) => url.clone(),
            _ => {
                return Err(EvaluationError::TypeError(
                    "validateVS() requires a ValueSet URL as string".to_string(),
                ));
            }
        };

        // Extract coding information
        let (system, code, display) = extract_coding_with_display(coded)?;

        // Extract parameters
        let params_map = extract_params_map(params)?;

        let system_opt = if system.is_empty() {
            None
        } else {
            Some(system.clone())
        };

        let key = LookupKey::new(
            "validate_vs",
            &self.server_url,
            self.fhir_version,
            vec![
                Some(value_set_url.clone()),
                system_opt.clone(),
                Some(code.clone()),
                display.clone(),
            ],
            params_map.as_ref(),
        );
        let result = self.cached(key, move |client| async move {
            let system_ref = system_opt.as_deref();
            let display_ref = display.as_deref();
            client
                .validate_vs(&value_set_url, system_ref, &code, display_ref, params_map)
                .await
        })?;

        match result {
            Ok(value) => json_to_evaluation_result(value),
            Err(e) => Err(EvaluationError::TerminologyError(format!(
                "ValueSet validation failed: {}",
                e
            ))),
        }
    }

    /// Validates a code against a CodeSystem
    ///
    /// Usage: %terminologies.validateCS(codeSystem, coded, params)
    pub fn validate_cs(
        &self,
        code_system: &EvaluationResult,
        coded: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract CodeSystem URL
        let code_system_url = match code_system {
            EvaluationResult::String(url, _, _) => url.clone(),
            _ => {
                return Err(EvaluationError::TypeError(
                    "validateCS() requires a CodeSystem URL as string".to_string(),
                ));
            }
        };

        // Extract code and display
        let (_system, code, display) = extract_coding_with_display(coded)?;

        // Extract parameters
        let params_map = extract_params_map(params)?;

        let key = LookupKey::new(
            "validate_cs",
            &self.server_url,
            self.fhir_version,
            vec![
                Some(code_system_url.clone()),
                Some(code.clone()),
                display.clone(),
            ],
            params_map.as_ref(),
        );
        let result = self.cached(key, move |client| async move {
            let display_ref = display.as_deref();
            client
                .validate_cs(&code_system_url, &code, display_ref, params_map)
                .await
        })?;

        match result {
            Ok(value) => json_to_evaluation_result(value),
            Err(e) => Err(EvaluationError::TerminologyError(format!(
                "CodeSystem validation failed: {}",
                e
            ))),
        }
    }

    /// Checks if one code subsumes another
    ///
    /// Usage: %terminologies.subsumes(system, coded1, coded2, params)
    pub fn subsumes(
        &self,
        system: &EvaluationResult,
        coded1: &EvaluationResult,
        coded2: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract system URL
        let system_url = match system {
            EvaluationResult::String(url, _, _) => url.clone(),
            _ => {
                return Err(EvaluationError::TypeError(
                    "subsumes() requires a system URL as string".to_string(),
                ));
            }
        };

        // Extract codes
        let (_sys1, code1) = extract_coding(coded1)?;
        let (_sys2, code2) = extract_coding(coded2)?;

        // Extract parameters
        let params_map = extract_params_map(params)?;

        let key = LookupKey::new(
            "subsumes",
            &self.server_url,
            self.fhir_version,
            vec![
                Some(system_url.clone()),
                Some(code1.clone()),
                Some(code2.clone()),
            ],
            params_map.as_ref(),
        );
        let result = self.cached(key, move |client| async move {
            client
                .subsumes(&system_url, &code1, &code2, params_map)
                .await
        })?;

        match result {
            Ok(value) => {
                // Extract the 'outcome' parameter value
                if let Some(parameters) = value.get("parameter").and_then(|p| p.as_array()) {
                    for param in parameters {
                        if param.get("name").and_then(|n| n.as_str()) == Some("outcome") {
                            if let Some(code) = param.get("valueCode").and_then(|c| c.as_str()) {
                                return Ok(EvaluationResult::string(code.to_string()));
                            }
                        }
                    }
                }
                Err(EvaluationError::TerminologyError(
                    "subsumes() result missing outcome parameter".to_string(),
                ))
            }
            Err(e) => Err(EvaluationError::TerminologyError(format!(
                "Subsumes check failed: {}",
                e
            ))),
        }
    }

    /// Translates a code using a ConceptMap
    ///
    /// Usage: %terminologies.translate(conceptMap, code, params)
    pub fn translate(
        &self,
        concept_map: &EvaluationResult,
        code: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract ConceptMap URL
        let concept_map_url = match concept_map {
            EvaluationResult::String(url, _, _) => url.clone(),
            _ => {
                return Err(EvaluationError::TypeError(
                    "translate() requires a ConceptMap URL as string".to_string(),
                ));
            }
        };

        // Extract coding
        let (system, code_str) = extract_coding(code)?;

        // Extract target system from params if provided
        let mut params_map = extract_params_map(params)?;
        let target_system = params_map.as_mut().and_then(|m| m.remove("targetSystem"));

        let key = LookupKey::new(
            "translate",
            &self.server_url,
            self.fhir_version,
            vec![
                Some(concept_map_url.clone()),
                Some(system.clone()),
                Some(code_str.clone()),
                target_system.clone(),
            ],
            params_map.as_ref(),
        );
        let result = self.cached(key, move |client| async move {
            let target_system_ref = target_system.as_deref();
            client
                .translate(
                    &concept_map_url,
                    &system,
                    &code_str,
                    target_system_ref,
                    params_map,
                )
                .await
        })?;

        match result {
            Ok(value) => json_to_evaluation_result(value),
            Err(e) => Err(EvaluationError::TerminologyError(format!(
                "Translation failed: {}",
                e
            ))),
        }
    }
}

/// Extracts system and code from a Coding or CodeableConcept
fn extract_coding(coded: &EvaluationResult) -> Result<(String, String), EvaluationError> {
    match coded {
        // Direct code string
        EvaluationResult::String(code, _, _) => Ok((String::new(), code.clone())),

        // Coding object
        EvaluationResult::Object { map, .. } => {
            let system = map
                .get("system")
                .and_then(|v| match v {
                    EvaluationResult::String(s, _, _) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();

            let code = map
                .get("code")
                .and_then(|v| match v {
                    EvaluationResult::String(c, _, _) => Some(c.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    EvaluationError::TypeError("Coding must have a 'code' element".to_string())
                })?;

            Ok((system, code))
        }

        _ => Err(EvaluationError::TypeError(
            "Expected string code or Coding object".to_string(),
        )),
    }
}

/// Extracts system, code, and display from a Coding
fn extract_coding_with_display(
    coded: &EvaluationResult,
) -> Result<(String, String, Option<String>), EvaluationError> {
    match coded {
        // Direct code string
        EvaluationResult::String(code, _, _) => Ok((String::new(), code.clone(), None)),

        // Coding object
        EvaluationResult::Object { map, .. } => {
            let system = map
                .get("system")
                .and_then(|v| match v {
                    EvaluationResult::String(s, _, _) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();

            let code = map
                .get("code")
                .and_then(|v| match v {
                    EvaluationResult::String(c, _, _) => Some(c.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    EvaluationError::TypeError("Coding must have a 'code' element".to_string())
                })?;

            let display = map.get("display").and_then(|v| match v {
                EvaluationResult::String(d, _, _) => Some(d.clone()),
                _ => None,
            });

            Ok((system, code, display))
        }

        _ => Err(EvaluationError::TypeError(
            "Expected string code or Coding object".to_string(),
        )),
    }
}

/// Extracts parameters map from Parameters resource or object
fn extract_params_map(
    params: Option<&EvaluationResult>,
) -> Result<Option<HashMap<String, String>>, EvaluationError> {
    match params {
        None => Ok(None),
        Some(EvaluationResult::Object { map, .. }) => {
            let mut params_map = HashMap::new();

            // Check if it's a Parameters resource
            if let Some(EvaluationResult::Collection { items, .. }) = map.get("parameter") {
                // Extract parameters from Parameters resource format
                for item in items {
                    if let EvaluationResult::Object { map: param_map, .. } = item {
                        if let (Some(name), Some(value)) = (
                            param_map.get("name").and_then(|n| match n {
                                EvaluationResult::String(s, _, _) => Some(s),
                                _ => None,
                            }),
                            extract_parameter_value(param_map),
                        ) {
                            params_map.insert(name.clone(), value);
                        }
                    }
                }
            } else {
                // Treat as simple key-value map
                for (key, value) in map {
                    if let EvaluationResult::String(v, _, _) = value {
                        params_map.insert(key.clone(), v.clone());
                    }
                }
            }

            Ok(Some(params_map))
        }
        Some(_) => Err(EvaluationError::TypeError(
            "Parameters must be an object or Parameters resource".to_string(),
        )),
    }
}

/// Extracts value from a parameter element
fn extract_parameter_value(param_map: &HashMap<String, EvaluationResult>) -> Option<String> {
    // Check for various value[x] types
    for (key, value) in param_map {
        if key.starts_with("value") {
            match value {
                EvaluationResult::String(s, _, _) => return Some(s.clone()),
                EvaluationResult::Boolean(b, _, _) => return Some(b.to_string()),
                EvaluationResult::Integer(i, _, _) => return Some(i.to_string()),
                EvaluationResult::Decimal(d, _, _) => return Some(d.to_string()),
                _ => {}
            }
        }
    }
    None
}

/// Converts JSON Value to EvaluationResult
fn json_to_evaluation_result(value: Value) -> Result<EvaluationResult, EvaluationError> {
    match value {
        Value::Null => Ok(EvaluationResult::Empty),
        Value::Bool(b) => Ok(EvaluationResult::boolean(b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(EvaluationResult::integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(EvaluationResult::decimal(
                    rust_decimal::Decimal::from_f64_retain(f)
                        .unwrap_or(rust_decimal::Decimal::ZERO),
                ))
            } else {
                Ok(EvaluationResult::string(n.to_string()))
            }
        }
        Value::String(s) => Ok(EvaluationResult::string(s)),
        Value::Array(arr) => {
            let items: Result<Vec<_>, _> = arr.into_iter().map(json_to_evaluation_result).collect();
            Ok(EvaluationResult::Collection {
                items: items?,
                has_undefined_order: false,
                type_info: None,
            })
        }
        Value::Object(obj) => {
            let mut map = HashMap::new();
            for (key, val) in obj {
                map.insert(key, json_to_evaluation_result(val)?);
            }
            Ok(EvaluationResult::Object {
                map,
                type_info: None,
            })
        }
    }
}

/// memberOf function implementation for Coding/CodeableConcept
///
/// Usage: coding.memberOf(valueSetUrl)
pub fn member_of(
    coding: &EvaluationResult,
    value_set_url: &str,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    let terminology = TerminologyFunctions::new(context)?;

    // Call validateVS and extract the result
    let validation_result = terminology.validate_vs(
        &EvaluationResult::string(value_set_url.to_string()),
        coding,
        None,
    )?;

    // Extract the 'result' parameter from the Parameters response
    if let EvaluationResult::Object { map, .. } = validation_result {
        if let Some(EvaluationResult::Collection { items, .. }) = map.get("parameter") {
            for item in items {
                if let EvaluationResult::Object { map: param_map, .. } = item {
                    if param_map.get("name").and_then(|n| match n {
                        EvaluationResult::String(s, _, _) => Some(s.as_str()),
                        _ => None,
                    }) == Some("result")
                    {
                        // Return the boolean value
                        if let Some(EvaluationResult::Boolean(result, type_info, _)) =
                            param_map.get("valueBoolean")
                        {
                            return Ok(EvaluationResult::Boolean(*result, type_info.clone(), None));
                        }
                    }
                }
            }
        }
    }

    // If we couldn't extract the result, return false
    Ok(EvaluationResult::boolean(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_coding_from_string() {
        let code = EvaluationResult::string("12345".to_string());
        let (system, code_str) = extract_coding(&code).unwrap();
        assert_eq!(system, "");
        assert_eq!(code_str, "12345");
    }

    #[test]
    fn test_extract_coding_from_object() {
        let mut map = HashMap::new();
        map.insert(
            "system".to_string(),
            EvaluationResult::string("http://loinc.org".to_string()),
        );
        map.insert(
            "code".to_string(),
            EvaluationResult::string("1234-5".to_string()),
        );
        map.insert(
            "display".to_string(),
            EvaluationResult::string("Test Code".to_string()),
        );

        let coding = EvaluationResult::Object {
            map,
            type_info: None,
        };

        let (system, code, display) = extract_coding_with_display(&coding).unwrap();
        assert_eq!(system, "http://loinc.org");
        assert_eq!(code, "1234-5");
        assert_eq!(display, Some("Test Code".to_string()));
    }

    const VS: &str = "http://example.org/fhir/ValueSet/test";

    /// Starts a stub terminology server answering `$validate-code` (ValueSet and CodeSystem)
    /// and `$expand`.
    async fn terminology_stub() -> wiremock::MockServer {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ValueSet/$validate-code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resourceType": "Parameters",
                "parameter": [{"name": "result", "valueBoolean": true}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/CodeSystem/$validate-code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resourceType": "Parameters",
                "parameter": [{"name": "result", "valueBoolean": true}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ValueSet/$expand"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resourceType": "ValueSet",
                "expansion": {
                    "total": 1,
                    "contains": [{"system": "http://example.org/cs", "code": "a"}]
                }
            })))
            .mount(&server)
            .await;
        server
    }

    fn context_for(server: &wiremock::MockServer) -> EvaluationContext {
        let mut ctx = EvaluationContext::new_empty_with_default_version();
        ctx.set_terminology_server(server.uri());
        ctx
    }

    fn is_member(code: &str, ctx: &EvaluationContext) -> Result<EvaluationResult, EvaluationError> {
        member_of(&EvaluationResult::string(code.to_string()), VS, ctx)
    }

    async fn request_count(server: &wiremock::MockServer) -> usize {
        server.received_requests().await.unwrap().len()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repeated_identical_lookups_hit_the_server_once() {
        let server = terminology_stub().await;
        let mut ctx = context_for(&server);
        ctx.terminology_session = Arc::new(TerminologySession::with_max_calls(None));

        for _ in 0..3 {
            assert!(matches!(
                is_member("a", &ctx),
                Ok(EvaluationResult::Boolean(true, ..))
            ));
        }
        assert!(matches!(
            is_member("a", &ctx.clone()),
            Ok(EvaluationResult::Boolean(true, ..))
        ));
        assert!(matches!(
            is_member("a", &ctx.create_child_context()),
            Ok(EvaluationResult::Boolean(true, ..))
        ));

        // select() lambdas run in child contexts that share the session
        crate::evaluate_expression(
            "('a' | 'b').select($this.memberOf('http://example.org/fhir/ValueSet/test')).allTrue() \
             and 'a'.memberOf('http://example.org/fhir/ValueSet/test')",
            &ctx,
        )
        .unwrap();
        // one request for 'a', one for 'b'
        assert_eq!(request_count(&server).await, 2);

        assert!(is_member("c", &ctx).is_ok());
        assert_eq!(request_count(&server).await, 3);

        // same code against a different value set is a different lookup
        let other_vs = format!("{VS}-other");
        assert!(member_of(&EvaluationResult::string("a".to_string()), &other_vs, &ctx).is_ok());
        assert_eq!(request_count(&server).await, 4);

        crate::evaluate_expression(
            "%terminologies.expand('http://example.org/fhir/ValueSet/test').exists() \
             and %terminologies.expand('http://example.org/fhir/ValueSet/test').exists()",
            &ctx,
        )
        .unwrap();
        assert_eq!(request_count(&server).await, 5);
    }

    #[tokio::test]
    async fn member_of_works_inside_a_current_thread_runtime() {
        let server = terminology_stub().await;
        let mut ctx = context_for(&server);
        ctx.terminology_session = Arc::new(TerminologySession::with_max_calls(None));

        assert!(matches!(
            is_member("x", &ctx),
            Ok(EvaluationResult::Boolean(true, ..))
        ));
        assert!(matches!(
            is_member("y", &ctx),
            Ok(EvaluationResult::Boolean(true, ..))
        ));
        assert_eq!(request_count(&server).await, 2);
    }

    #[test]
    fn max_calls_parsing() {
        assert_eq!(parse_max_calls(None), Some(DEFAULT_MAX_CALLS));
        assert_eq!(parse_max_calls(Some("25")), Some(25));
        assert_eq!(parse_max_calls(Some(" 7 ")), Some(7));
        assert_eq!(parse_max_calls(Some("0")), None);
        assert_eq!(parse_max_calls(Some("lots")), Some(DEFAULT_MAX_CALLS));
        assert_eq!(parse_max_calls(Some("-1")), Some(DEFAULT_MAX_CALLS));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exceeding_the_call_limit_is_an_evaluation_error() {
        let server = terminology_stub().await;
        let mut ctx = context_for(&server);
        ctx.terminology_session = Arc::new(TerminologySession::with_max_calls(Some(2)));

        assert!(matches!(
            is_member("a", &ctx),
            Ok(EvaluationResult::Boolean(true, ..))
        ));
        assert!(matches!(
            is_member("b", &ctx),
            Ok(EvaluationResult::Boolean(true, ..))
        ));
        // the failure is neither cached nor charged, so asking again fails the same way
        for _ in 0..2 {
            match is_member("c", &ctx) {
                Err(EvaluationError::TerminologyCallLimit(msg)) => {
                    assert!(msg.contains("FHIRPATH_TERMINOLOGY_MAX_CALLS"), "{msg}");
                    assert!(msg.contains('2'), "{msg}");
                }
                other => panic!("expected the call limit error, got {other:?}"),
            }
        }
        // cache hits are still answered after the cap is reached
        assert!(matches!(
            is_member("a", &ctx),
            Ok(EvaluationResult::Boolean(true, ..))
        ));
        assert_eq!(request_count(&server).await, 2);

        // through the evaluator it is an error, not a panic
        let mut ctx2 = context_for(&server);
        ctx2.terminology_session = Arc::new(TerminologySession::with_max_calls(Some(1)));
        let err = crate::evaluate_expression(
            "'p'.memberOf('http://example.org/fhir/ValueSet/test') \
             and 'q'.memberOf('http://example.org/fhir/ValueSet/test')",
            &ctx2,
        )
        .unwrap_err();
        assert!(err.contains("FHIRPATH_TERMINOLOGY_MAX_CALLS"), "{err}");

        let typed = crate::evaluate_expression_typed(
            "'p'.memberOf('http://example.org/fhir/ValueSet/test') \
             and 'q'.memberOf('http://example.org/fhir/ValueSet/test')",
            &ctx2,
        )
        .unwrap_err();
        assert!(typed.is_terminology_call_limit() && typed.is_terminology_error());
        assert!(typed.to_string().contains("FHIRPATH_TERMINOLOGY_MAX_CALLS"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn contexts_can_share_one_session() {
        let server = terminology_stub().await;
        let mut ctx1 = context_for(&server);
        ctx1.set_terminology_session(Arc::new(TerminologySession::with_max_calls(None)));
        let mut ctx2 = context_for(&server);
        ctx2.set_terminology_session(ctx1.terminology_session());
        assert!(Arc::ptr_eq(
            &ctx1.terminology_session(),
            &ctx2.terminology_session()
        ));

        assert!(is_member("a", &ctx1).is_ok());
        assert!(is_member("a", &ctx2).is_ok());
        assert_eq!(request_count(&server).await, 1);

        // an unshared context has its own cache
        let ctx3 = context_for(&server);
        assert!(is_member("a", &ctx3).is_ok());
        assert_eq!(request_count(&server).await, 2);

        // an unset cap reads the environment, like a session no test configured
        assert_eq!(
            TerminologySession::default().max_calls(),
            max_calls_from_env()
        );
    }

    fn params_of(pairs: &[(&str, &str)]) -> EvaluationResult {
        EvaluationResult::Object {
            map: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), EvaluationResult::string(v.to_string())))
                .collect(),
            type_info: None,
        }
    }

    fn coding(code: &str, display: Option<&str>) -> EvaluationResult {
        let mut map = HashMap::new();
        map.insert(
            "system".to_string(),
            EvaluationResult::string("http://example.org/cs".to_string()),
        );
        map.insert(
            "code".to_string(),
            EvaluationResult::string(code.to_string()),
        );
        if let Some(display) = display {
            map.insert(
                "display".to_string(),
                EvaluationResult::string(display.to_string()),
            );
        }
        EvaluationResult::Object {
            map,
            type_info: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cache_key_covers_params_and_display() {
        let server = terminology_stub().await;
        let mut ctx = context_for(&server);
        ctx.set_terminology_session(Arc::new(TerminologySession::with_max_calls(None)));
        let terminology = TerminologyFunctions::new(&ctx).unwrap();
        let vs = EvaluationResult::string(VS.to_string());

        let p1 = params_of(&[("count", "5")]);
        let p2 = params_of(&[("count", "6")]);
        terminology.expand(&vs, Some(&p1)).unwrap();
        terminology.expand(&vs, Some(&p1)).unwrap();
        assert_eq!(request_count(&server).await, 1);
        terminology.expand(&vs, Some(&p2)).unwrap();
        assert_eq!(request_count(&server).await, 2);

        // a call differing only in display is a different request
        let bare = coding("a", None);
        let shown = coding("a", Some("Alpha"));
        terminology.validate_vs(&vs, &bare, None).unwrap();
        terminology.validate_vs(&vs, &bare, None).unwrap();
        assert_eq!(request_count(&server).await, 3);
        terminology.validate_vs(&vs, &shown, None).unwrap();
        assert_eq!(request_count(&server).await, 4);

        let cs = EvaluationResult::string("http://example.org/cs".to_string());
        terminology.validate_cs(&cs, &bare, None).unwrap();
        terminology.validate_cs(&cs, &shown, None).unwrap();
        terminology.validate_cs(&cs, &shown, None).unwrap();
        assert_eq!(request_count(&server).await, 6);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_identical_lookups_make_one_request() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ValueSet/$validate-code"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({
                        "resourceType": "Parameters",
                        "parameter": [{"name": "result", "valueBoolean": true}]
                    }))
                    .set_delay(std::time::Duration::from_millis(300)),
            )
            .mount(&server)
            .await;

        let session = Arc::new(TerminologySession::with_max_calls(None));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let mut ctx = context_for(&server);
                ctx.set_terminology_session(Arc::clone(&session));
                std::thread::spawn(move || is_member("a", &ctx))
            })
            .collect();
        for thread in threads {
            let result = thread.join().unwrap();
            assert!(
                matches!(result, Ok(EvaluationResult::Boolean(true, ..))),
                "{result:?}"
            );
        }
        assert_eq!(request_count(&server).await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_panicking_request_is_an_error_not_a_panic() {
        let result: Result<(), EvaluationError> = block_on_async(async { panic!("boom") });
        assert!(result.is_err());
    }

    #[test]
    fn a_panicking_request_outside_a_runtime_is_an_error() {
        let result: Result<(), EvaluationError> = block_on_async(async { panic!("boom") });
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cache_key_distinguishes_lookup_subsumes_and_translate() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        for (route, parameter) in [
            (
                "/CodeSystem/$lookup",
                serde_json::json!({"name": "name", "valueString": "Test"}),
            ),
            (
                "/CodeSystem/$subsumes",
                serde_json::json!({"name": "outcome", "valueCode": "subsumes"}),
            ),
            (
                "/ConceptMap/$translate",
                serde_json::json!({"name": "result", "valueBoolean": true}),
            ),
        ] {
            Mock::given(method("POST"))
                .and(path(route))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "resourceType": "Parameters",
                    "parameter": [parameter]
                })))
                .mount(&server)
                .await;
        }

        let mut ctx = context_for(&server);
        ctx.set_terminology_session(Arc::new(TerminologySession::with_max_calls(None)));
        let terminology = TerminologyFunctions::new(&ctx).unwrap();
        let cs = EvaluationResult::string("http://example.org/cs".to_string());
        let cm = EvaluationResult::string("http://example.org/cm".to_string());
        let (a, b) = (coding("a", None), coding("b", None));

        terminology.lookup(&a, None).unwrap();
        terminology.lookup(&a, None).unwrap();
        assert_eq!(request_count(&server).await, 1);

        terminology.subsumes(&cs, &a, &b, None).unwrap();
        terminology.subsumes(&cs, &a, &b, None).unwrap();
        assert_eq!(request_count(&server).await, 2);
        terminology.subsumes(&cs, &b, &a, None).unwrap();
        assert_eq!(request_count(&server).await, 3);

        let to_x = params_of(&[("targetSystem", "http://example.org/x")]);
        let to_y = params_of(&[("targetSystem", "http://example.org/y")]);
        terminology.translate(&cm, &a, Some(&to_x)).unwrap();
        terminology.translate(&cm, &a, Some(&to_x)).unwrap();
        assert_eq!(request_count(&server).await, 4);
        terminology.translate(&cm, &a, Some(&to_y)).unwrap();
        assert_eq!(request_count(&server).await, 5);
    }

    /// A stub whose `$validate-code` always answers `status` with a short body.
    async fn status_stub(status: u16) -> wiremock::MockServer {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ValueSet/$validate-code"))
            .respond_with(ResponseTemplate::new(status).set_body_string("stub failure"))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_lookups_are_answered_once() {
        // 4xx is deterministic and is not retried by the client (502/503/504/530 are)
        for status in [400u16, 404, 422] {
            let server = status_stub(status).await;
            let mut ctx = context_for(&server);
            ctx.terminology_session = Arc::new(TerminologySession::with_max_calls(None));

            let first = is_member("a", &ctx).unwrap_err().to_string();
            let second = is_member("a", &ctx).unwrap_err().to_string();
            assert!(first.contains("ValueSet validation failed"), "{first}");
            assert!(first.contains(&status.to_string()), "{first}");
            assert_eq!(first, second);
            assert_eq!(request_count(&server).await, 1, "status {status}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_success_body_is_answered_once() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ValueSet/$validate-code"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let mut ctx = context_for(&server);
        ctx.terminology_session = Arc::new(TerminologySession::with_max_calls(None));

        assert!(is_member("a", &ctx).is_err());
        assert!(is_member("a", &ctx).is_err());
        assert_eq!(request_count(&server).await, 1);
    }

    /// A stub whose first `$validate-code` answer is a 500 and every later one a success.
    async fn flaky_stub() -> wiremock::MockServer {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};

        let server = wiremock::MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ValueSet/$validate-code"))
            .respond_with(ResponseTemplate::new(500).set_body_string("try later"))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/ValueSet/$validate-code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resourceType": "Parameters",
                "parameter": [{"name": "result", "valueBoolean": true}]
            })))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn transient_failures_are_retried_by_the_next_lookup() {
        let server = flaky_stub().await;
        let mut ctx = context_for(&server);
        ctx.terminology_session = Arc::new(TerminologySession::with_max_calls(None));

        match is_member("a", &ctx) {
            Err(EvaluationError::TerminologyError(msg)) => assert!(msg.contains("500"), "{msg}"),
            other => panic!("expected a terminology error, got {other:?}"),
        }
        assert_eq!(request_count(&server).await, 1);
        assert!(matches!(
            is_member("a", &ctx),
            Ok(EvaluationResult::Boolean(true, ..))
        ));
        assert_eq!(request_count(&server).await, 2);
        // the success is kept
        assert!(matches!(
            is_member("a", &ctx),
            Ok(EvaluationResult::Boolean(true, ..))
        ));
        assert_eq!(request_count(&server).await, 2);
        assert_eq!(ctx.terminology_session().state.lock().remote_calls, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn server_errors_are_resent_and_count_against_the_cap() {
        let server = status_stub(500).await;
        let mut ctx = context_for(&server);
        ctx.terminology_session = Arc::new(TerminologySession::with_max_calls(Some(2)));

        for _ in 0..2 {
            assert!(matches!(
                is_member("a", &ctx),
                Err(EvaluationError::TerminologyError(_))
            ));
        }
        assert!(matches!(
            is_member("a", &ctx),
            Err(EvaluationError::TerminologyCallLimit(_))
        ));
        assert_eq!(request_count(&server).await, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn waiters_share_a_transient_failure_without_resending() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ValueSet/$validate-code"))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_string("down")
                    .set_delay(std::time::Duration::from_millis(300)),
            )
            .mount(&server)
            .await;

        let session = Arc::new(TerminologySession::with_max_calls(None));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let mut ctx = context_for(&server);
                ctx.set_terminology_session(Arc::clone(&session));
                std::thread::spawn(move || is_member("a", &ctx))
            })
            .collect();
        for thread in threads {
            let result = thread.join().unwrap();
            assert!(result.is_err(), "{result:?}");
        }
        assert_eq!(request_count(&server).await, 1);

        let mut ctx = context_for(&server);
        ctx.set_terminology_session(session);
        assert!(is_member("a", &ctx).is_err());
        assert_eq!(request_count(&server).await, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connection_errors_are_not_cached() {
        // bind to find a free port, then drop the listener so nothing answers there
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut ctx = EvaluationContext::new_empty_with_default_version();
        ctx.set_terminology_server(format!("http://127.0.0.1:{port}"));
        ctx.terminology_session = Arc::new(TerminologySession::with_max_calls(None));

        assert!(is_member("a", &ctx).is_err());
        assert!(is_member("a", &ctx).is_err());
        let session = ctx.terminology_session();
        let state = session.state.lock();
        assert_eq!(state.remote_calls, 2);
        assert!(state.answers.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn terminology_errors_are_typed() {
        let server = status_stub(404).await;
        let mut ctx = context_for(&server);
        ctx.terminology_session = Arc::new(TerminologySession::with_max_calls(None));

        assert!(matches!(
            is_member("a", &ctx),
            Err(EvaluationError::TerminologyError(_))
        ));
        assert!(no_terminology_server().is_terminology_error());
        assert!(!no_terminology_server().is_terminology_call_limit());

        // an argument-shape error is not a terminology error
        let shape = crate::evaluate_expression_typed("1.memberOf(1)", &ctx).unwrap_err();
        assert!(!shape.is_terminology_error(), "{shape}");
        // neither is a parse error
        let parse = crate::evaluate_expression_typed("1 +", &ctx).unwrap_err();
        assert!(matches!(parse, crate::ExpressionError::Parse(_)));
        assert!(!parse.is_terminology_error());
    }

    /// Name of the thread that polled a future handed to `block_on_async`.
    fn runtime_thread_name() -> Option<String> {
        block_on_async(async { std::thread::current().name().map(str::to_owned) }).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn async_callers_run_requests_on_the_shared_runtime() {
        assert_eq!(runtime_thread_name().as_deref(), Some(RUNTIME_THREAD_NAME));
    }

    #[tokio::test]
    async fn current_thread_callers_run_requests_on_the_shared_runtime() {
        assert_eq!(runtime_thread_name().as_deref(), Some(RUNTIME_THREAD_NAME));
    }

    #[test]
    fn block_on_async_works_outside_a_runtime() {
        assert_eq!(block_on_async(async { 42 }).unwrap(), 42);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zero_cap_means_unlimited() {
        let server = terminology_stub().await;
        let mut ctx = context_for(&server);
        ctx.terminology_session = Arc::new(TerminologySession::with_max_calls(None));

        for code in ["a", "b", "c", "d"] {
            assert!(is_member(code, &ctx).is_ok());
        }
        assert_eq!(request_count(&server).await, 4);
    }
}
