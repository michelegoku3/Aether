//! DepotBox (`depotbox.lol`) package provider.
//!
//! Downloads go through the asynchronous three-step flow: start a job, poll its
//! status, then fetch the short-lived link. The direct endpoint is deliberately
//! not used: it keeps one connection open for the whole server-side build (the
//! documentation asks for a 10-15 minute read timeout), which would pin the
//! process-wide provider lane, report no progress and lose everything on a
//! dropped connection.
//!
//! Invariants:
//! - the API key travels only in the `X-API-Key` header, never in a URL or a log;
//! - a non-2xx answer is reported as a server error, never parsed as a payload;
//! - the status document is read field by field, so an optional field shaped
//!   differently from the documentation (live `logs` entries are objects, not
//!   strings) can never fail a download whose state is perfectly readable;
//! - polling stops at a terminal state or at [`JOB_DEADLINE`], so a stuck job
//!   cannot hang the download forever.

use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;

use crate::manifest::package::{ManifestPackage, ManifestPackageExtractor};
use crate::providers::http;

const BASE_URL: &str = "https://depotbox.lol";
/// The job start only queues work server-side, so it answers quickly.
const START_TIMEOUT_SECONDS: u64 = 30;
/// One status poll is a small JSON document.
const POLL_TIMEOUT_SECONDS: u64 = 30;
/// The finished ZIP is streamed by a separate request; large games take a while.
const DOWNLOAD_TIMEOUT_SECONDS: u64 = 300;
const POLL_INTERVAL: Duration = Duration::from_millis(2500);
/// The provider documents 10-15 minutes for the longest builds.
const JOB_DEADLINE: Duration = Duration::from_secs(900);

/// Remote job states documented by DepotBox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobPhase {
    Processing,
    Completed,
    Warning,
    Failed,
    Expired,
    Unknown,
}

/// One started remote job. The token is the credential for polling and for the
/// download link, so it is kept in memory only and never logged in full.
pub struct DepotBoxJob {
    token: String,
    app_id: u32,
}

#[derive(Debug, Deserialize)]
struct JobStart {
    #[serde(default)]
    token: String,
}

/// One polled status document. Only `status` is required; every other field is
/// advisory. `pub(crate)` so the parsing rules can be asserted directly in the
/// crate's tests against payloads that drift from the documentation.
#[derive(Debug, Default)]
pub(crate) struct JobStatus {
    pub(crate) status: String,
    pub(crate) message: Option<String>,
    pub(crate) logs: Option<Vec<String>>,
    pub(crate) download_link: Option<String>,
    pub(crate) final_user_zip_name: Option<String>,
    pub(crate) expiry_minutes: Option<u64>,
    pub(crate) failure_reason: Option<String>,
    pub(crate) game_name: Option<String>,
}

#[derive(Clone)]
pub struct DepotBoxClient {
    client: reqwest::Client,
}

impl DepotBoxClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(&api_key.into()) {
            headers.insert("X-API-Key", value);
        }
        Self {
            client: http::shared_client_with_headers(DOWNLOAD_TIMEOUT_SECONDS, headers),
        }
    }

    /// Whole package in one call: start the remote job, wait for it, fetch the ZIP.
    pub async fn download_lua_package(&self, app_id: u32) -> Result<ManifestPackage, String> {
        let job = self.start_job(app_id).await?;
        self.wait_for_package(&job).await
    }

    /// `POST /api/download` — queues the build and returns the poll token.
    pub async fn start_job(&self, app_id: u32) -> Result<DepotBoxJob, String> {
        let started = Instant::now();
        crate::desk_log_info!(
            "depotbox",
            "Starting DepotBox package job for {} at {}",
            crate::core::logger::format_appid(app_id),
            BASE_URL
        );
        let response = self
            .client
            .post(format!("{BASE_URL}/api/download"))
            .timeout(Duration::from_secs(START_TIMEOUT_SECONDS))
            .json(&serde_json::json!({ "appid": app_id.to_string() }))
            .send()
            .await
            .map_err(|error| {
                crate::desk_log_error!(
                    "depotbox",
                    "DepotBox start request for {} failed: {}",
                    crate::core::logger::format_appid(app_id),
                    error
                );
                format!("DepotBox start request failed: {error}")
            })?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let detail = describe_api_error(status.as_u16(), &body);
            crate::desk_log_error!(
                "depotbox",
                "DepotBox refused the job for {}: {}",
                crate::core::logger::format_appid(app_id),
                detail
            );
            return Err(format!(
                "DepotBox could not start the package job: {detail}"
            ));
        }
        let parsed: JobStart = serde_json::from_str(&body)
            .map_err(|error| format!("DepotBox returned an unreadable job response: {error}"))?;
        let token = parsed.token.trim().to_string();
        if token.is_empty() {
            return Err("DepotBox accepted the job but returned no token".to_string());
        }
        crate::desk_log_info!(
            "depotbox",
            "DepotBox job for {} queued (token {}…, {} ms)",
            crate::core::logger::format_appid(app_id),
            token.chars().take(8).collect::<String>(),
            started.elapsed().as_millis()
        );
        Ok(DepotBoxJob { token, app_id })
    }

    /// Polls until a terminal state, then downloads and parses the ZIP.
    pub async fn wait_for_package(&self, job: &DepotBoxJob) -> Result<ManifestPackage, String> {
        let started = Instant::now();
        let deadline = started + JOB_DEADLINE;
        let mut last_message = String::new();
        loop {
            if Instant::now() >= deadline {
                crate::desk_log_error!(
                    "depotbox",
                    "DepotBox job for {} exceeded the {} s deadline at phase '{}'",
                    crate::core::logger::format_appid(job.app_id),
                    JOB_DEADLINE.as_secs(),
                    last_message
                );
                return Err(format!(
                    "DepotBox did not finish the package for App ID {} within {} seconds. Start the download again.",
                    job.app_id,
                    JOB_DEADLINE.as_secs()
                ));
            }
            let status = self.poll(job).await?;
            match classify_status(&status.status) {
                JobPhase::Processing | JobPhase::Unknown => {
                    let message = status.message.clone().unwrap_or_default();
                    if message != last_message {
                        crate::desk_log_info!(
                            "depotbox",
                            "DepotBox job for {} at '{}' ({}): {}",
                            crate::core::logger::format_appid(job.app_id),
                            status.status,
                            started.elapsed().as_secs(),
                            if message.is_empty() {
                                "working"
                            } else {
                                message.as_str()
                            }
                        );
                        last_message = message;
                    }
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
                JobPhase::Failed => {
                    let reason = status
                        .failure_reason
                        .or(status.message)
                        .unwrap_or_else(|| "no reason reported".to_string());
                    crate::desk_log_error!(
                        "depotbox",
                        "DepotBox job for {} failed: {}",
                        crate::core::logger::format_appid(job.app_id),
                        reason
                    );
                    return Err(format!("DepotBox could not build the package: {reason}"));
                }
                JobPhase::Expired => {
                    crate::desk_log_error!(
                        "depotbox",
                        "DepotBox job for {} expired before it was fetched",
                        crate::core::logger::format_appid(job.app_id)
                    );
                    return Err(
                        "The DepotBox job expired before it was fetched. Start the download again."
                            .to_string(),
                    );
                }
                JobPhase::Completed | JobPhase::Warning => {
                    if classify_status(&status.status) == JobPhase::Warning {
                        crate::desk_log_warn!(
                            "depotbox",
                            "DepotBox finished the package for {} with warnings: {}",
                            crate::core::logger::format_appid(job.app_id),
                            status.message.as_deref().unwrap_or("none reported")
                        );
                    }
                    let link = status.download_link.clone().ok_or_else(|| {
                        "DepotBox reported completion without a download link".to_string()
                    })?;
                    crate::desk_log_info!(
                        "depotbox",
                        "DepotBox package for {} ({}) ready as {:?} after {} s (link valid {} min)",
                        crate::core::logger::format_appid(job.app_id),
                        status.game_name.as_deref().unwrap_or("unknown name"),
                        status.final_user_zip_name.as_deref().unwrap_or("<unnamed>"),
                        started.elapsed().as_secs(),
                        status
                            .expiry_minutes
                            .map_or_else(|| "?".to_string(), |minutes| minutes.to_string())
                    );
                    if let Some(logs) = &status.logs {
                        crate::desk_log_debug!(
                            "depotbox",
                            "DepotBox build log for {} ({}): {}",
                            crate::core::logger::format_appid(job.app_id),
                            logs.len(),
                            logs.join(" | ")
                        );
                    }
                    let bytes = self.download_zip(&link, job.app_id).await?;
                    let package = ManifestPackageExtractor::from_zip(job.app_id, &bytes).map_err(
                        |error| {
                            crate::desk_log_error!(
                                "depotbox",
                                "DepotBox ZIP for {} is not a usable package: {}",
                                crate::core::logger::format_appid(job.app_id),
                                error
                            );
                            error
                        },
                    )?;
                    crate::desk_log_info!(
                        "depotbox",
                        "DepotBox package for {} parsed: {} manifest(s), {} Lua bytes",
                        crate::core::logger::format_appid(job.app_id),
                        package.manifest_files.len(),
                        package.lua_content.len()
                    );
                    return Ok(package);
                }
            }
        }
    }

    async fn poll(&self, job: &DepotBoxJob) -> Result<JobStatus, String> {
        let response = self
            .client
            .get(format!("{BASE_URL}/api/status/{}", job.token))
            .timeout(Duration::from_secs(POLL_TIMEOUT_SECONDS))
            .send()
            .await
            .map_err(|error| format!("DepotBox status request failed: {error}"))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!(
                "DepotBox status check failed: {}",
                describe_api_error(status.as_u16(), &body)
            ));
        }
        parse_status(&body)
    }

    async fn download_zip(&self, link: &str, app_id: u32) -> Result<Vec<u8>, String> {
        let url = if link.starts_with("http://") || link.starts_with("https://") {
            link.to_string()
        } else {
            format!("{BASE_URL}{link}")
        };
        let started = Instant::now();
        let response = self
            .client
            .get(url)
            .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECONDS))
            .send()
            .await
            .map_err(|error| format!("Could not download the DepotBox package: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail = describe_api_error(status.as_u16(), &body);
            crate::desk_log_error!(
                "depotbox",
                "DepotBox package download for {} failed: {}",
                crate::core::logger::format_appid(app_id),
                detail
            );
            return Err(format!("Could not download the DepotBox package: {detail}"));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("Could not read the DepotBox package bytes: {error}"))?;
        crate::desk_log_info!(
            "depotbox",
            "Downloaded DepotBox package for {} ({} bytes in {} ms)",
            crate::core::logger::format_appid(app_id),
            bytes.len(),
            started.elapsed().as_millis()
        );
        Ok(bytes.to_vec())
    }
}

/// Parses one `GET /api/status/:token` body. A failure names the payload shape
/// so the next report says which field drifted, and never echoes values: the
/// document carries the job token inside `download_link`.
pub(crate) fn parse_status(body: &str) -> Result<JobStatus, String> {
    let payload: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| format!("DepotBox returned an unreadable status: {error}"))?;
    status_from_payload(&payload).map_err(|error| {
        format!(
            "DepotBox returned a status AetherDesk cannot read ({error}; fields: {})",
            describe_shape(&payload)
        )
    })
}

/// Reads a status document field by field. `status` must be a string: a missing
/// or nested state is a real protocol failure, while an advisory field shaped
/// differently is not.
fn status_from_payload(payload: &serde_json::Value) -> Result<JobStatus, String> {
    let status = payload
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|state| !state.is_empty())
        .ok_or_else(|| "no string 'status' field".to_string())?;
    Ok(JobStatus {
        status: status.to_string(),
        message: stringish(payload.get("message")),
        logs: log_lines(payload.get("logs")),
        download_link: stringish(payload.get("download_link")),
        final_user_zip_name: stringish(payload.get("finalUserZipName")),
        expiry_minutes: minutes(payload.get("expiry_minutes")),
        failure_reason: stringish(payload.get("failureReason")),
        game_name: stringish(payload.get("gameName")),
    })
}

/// Keys that hold the readable text of a log or message object, most specific
/// first: the live service wraps progress lines in objects instead of strings.
const MESSAGE_KEYS: [&str; 6] = ["message", "text", "msg", "line", "detail", "step"];

/// One JSON value as display text. Objects with none of [`MESSAGE_KEYS`] degrade
/// to their compact JSON form, which keeps the line informative without losing
/// the download.
fn stringish(value: Option<&serde_json::Value>) -> Option<String> {
    use serde_json::Value;
    let text = match value? {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Object(map) => MESSAGE_KEYS
            .iter()
            .find_map(|key| stringish(map.get(*key)))
            .or_else(|| serde_json::to_string(map).ok())?,
        Value::Array(_) | Value::Null => return None,
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Progress lines whatever the server sent: strings, numbers or objects. A
/// non-array value is accepted as a single line.
fn log_lines(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    use serde_json::Value;
    match value? {
        Value::Array(items) => {
            let lines: Vec<String> = items
                .iter()
                .filter_map(|item| stringish(Some(item)))
                .collect();
            (!lines.is_empty()).then_some(lines)
        }
        other => stringish(Some(other)).map(|line| vec![line]),
    }
}

/// `expiry_minutes` is documented as an integer but arrives as a string too.
fn minutes(value: Option<&serde_json::Value>) -> Option<u64> {
    use serde_json::Value;
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn json_type(value: &serde_json::Value) -> &'static str {
    use serde_json::Value;
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// `status:string, logs:array, …` — keys and types only, so a diagnostic line
/// can be shared without exposing the job token a status document may carry.
fn describe_shape(payload: &serde_json::Value) -> String {
    use serde_json::Value;
    match payload {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| format!("{key}:{}", json_type(value)))
            .collect::<Vec<_>>()
            .join(", "),
        other => json_type(other).to_string(),
    }
}

pub(crate) fn classify_status(status: &str) -> JobPhase {
    match status.trim().to_ascii_lowercase().as_str() {
        "processing" => JobPhase::Processing,
        "completed" => JobPhase::Completed,
        "completed_with_warnings" => JobPhase::Warning,
        "failed" => JobPhase::Failed,
        "invalid_or_expired" => JobPhase::Expired,
        _ => JobPhase::Unknown,
    }
}

/// Human-readable failure for a non-2xx answer. DepotBox always answers with
/// `{"error": "..."}`; the status code decides how to frame it because the
/// server message is not guaranteed to be user friendly.
pub(crate) fn describe_api_error(status: u16, body: &str) -> String {
    let server_message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.as_str())
                .map(str::to_string)
        })
        .filter(|message| !message.trim().is_empty());
    match status {
        401 => "no API key was sent — add the DepotBox key in Settings".to_string(),
        403 => server_message.unwrap_or_else(|| {
            "the DepotBox key was rejected (invalid, pending, suspended or not allowed from this IP)"
                .to_string()
        }),
        404 => "DepotBox has no depot keys for this App ID (HTTP 404)".to_string(),
        429 => server_message
            .unwrap_or_else(|| "the DepotBox rate limit was reached (HTTP 429)".to_string()),
        _ => server_message.unwrap_or_else(|| format!("DepotBox returned HTTP {status}")),
    }
}
