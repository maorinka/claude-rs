//! Remote Control environments API client.
//!
//! Ports the general HTTP contract from TS `src/bridge/bridgeApi.ts`: shared
//! headers, safe path-id validation, OAuth bearer handling, trusted-device
//! header support, and the bridge environment/work/session endpoints. Runtime
//! orchestration still lives above this layer.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const BETA_HEADER: &str = "environments-2025-11-01";
pub const SESSION_BETA_HEADER: &str = "ccr-byoc-2025-07-29";
pub const BRIDGE_LOGIN_INSTRUCTION: &str = "Remote Control is only available with claude.ai subscriptions. Please use `/login` to sign in with your claude.ai account.";
pub const REMOTE_CONTROL_DISCONNECTED_MSG: &str = "Remote Control disconnected.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFatalError {
    pub message: String,
    pub status: u16,
    pub error_type: Option<String>,
}

impl fmt::Display for BridgeFatalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BridgeFatalError {}

#[derive(Debug, thiserror::Error)]
pub enum BridgeApiError {
    #[error("{0}")]
    Fatal(BridgeFatalError),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SpawnMode {
    SingleSession,
    Worktree,
    SameDir,
}

#[derive(Debug, Clone)]
pub struct BridgeRuntimeConfig {
    pub dir: String,
    pub machine_name: String,
    pub branch: String,
    pub git_repo_url: Option<String>,
    pub max_sessions: u32,
    pub spawn_mode: SpawnMode,
    pub verbose: bool,
    pub sandbox: bool,
    pub bridge_id: String,
    pub worker_type: String,
    pub environment_id: String,
    pub reuse_environment_id: Option<String>,
    pub api_base_url: String,
    pub session_ingress_url: String,
    pub debug_file: Option<String>,
    pub session_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkData {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub environment_id: String,
    pub state: String,
    pub data: WorkData,
    pub secret: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionResponseEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub response: PermissionResponseEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionResponseEnvelope {
    pub subtype: String,
    pub request_id: String,
    pub response: Value,
}

#[derive(Debug, Clone)]
pub struct CreateBridgeSessionRequest {
    pub environment_id: String,
    pub organization_uuid: String,
    pub title: Option<String>,
    pub events: Vec<Value>,
    pub git_repo_url: Option<String>,
    pub branch: String,
    pub model: String,
    pub permission_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSessionInfo {
    pub environment_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BridgeApiClient {
    http: reqwest::Client,
    base_url: String,
    access_token: Option<String>,
    runner_version: String,
    trusted_device_token: Option<String>,
}

impl BridgeApiClient {
    pub fn new(
        http: reqwest::Client,
        base_url: impl Into<String>,
        access_token: Option<String>,
        runner_version: impl Into<String>,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            access_token,
            runner_version: runner_version.into(),
            trusted_device_token: None,
        }
    }

    pub fn with_trusted_device_token(mut self, token: Option<String>) -> Self {
        self.trusted_device_token = token;
        self
    }

    fn resolve_auth(&self) -> std::result::Result<&str, BridgeApiError> {
        self.access_token
            .as_deref()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| BridgeApiError::Other(BRIDGE_LOGIN_INSTRUCTION.to_string()))
    }

    fn apply_headers(
        &self,
        request: reqwest::RequestBuilder,
        token: &str,
    ) -> reqwest::RequestBuilder {
        let mut request = request
            .bearer_auth(token)
            .header("content-type", "application/json")
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", BETA_HEADER)
            .header("x-environment-runner-version", &self.runner_version);
        if let Some(token) = self.trusted_device_token.as_deref() {
            if !token.is_empty() {
                request = request.header("X-Trusted-Device-Token", token);
            }
        }
        request
    }

    fn apply_session_headers(
        &self,
        request: reqwest::RequestBuilder,
        token: &str,
        organization_uuid: &str,
    ) -> reqwest::RequestBuilder {
        request
            .bearer_auth(token)
            .header("content-type", "application/json")
            .header("anthropic-beta", SESSION_BETA_HEADER)
            .header("x-organization-uuid", organization_uuid)
    }

    pub async fn register_bridge_environment(
        &self,
        config: &BridgeRuntimeConfig,
    ) -> std::result::Result<(String, String), BridgeApiError> {
        let token = self.resolve_auth()?;
        let body = serde_json::json!({
            "machine_name": config.machine_name,
            "directory": config.dir,
            "branch": config.branch,
            "git_repo_url": config.git_repo_url,
            "max_sessions": config.max_sessions,
            "metadata": {"worker_type": config.worker_type},
        });
        let mut body = body.as_object().cloned().unwrap();
        if let Some(id) = config.reuse_environment_id.as_deref() {
            body.insert("environment_id".to_string(), Value::String(id.to_string()));
        }
        let resp = self
            .apply_headers(
                self.http
                    .post(format!("{}/v1/environments/bridge", self.base_url)),
                token,
            )
            .json(&body)
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("Registration: {err}")))?;
        let data = response_json(resp).await?;
        handle_error_status(data.status, &data.body, "Registration")?;
        Ok((
            data.body
                .get("environment_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            data.body
                .get("environment_secret")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ))
    }

    /// TS `createBridgeSession` parity. Returns `Ok(None)` for the same
    /// non-fatal cases TS logs and swallows: missing auth/org, non-2xx status,
    /// or a response without an `id`.
    pub async fn create_bridge_session(
        &self,
        request: &CreateBridgeSessionRequest,
    ) -> std::result::Result<Option<String>, BridgeApiError> {
        let Some(token) = self
            .access_token
            .as_deref()
            .filter(|token| !token.is_empty())
        else {
            return Ok(None);
        };
        if request.organization_uuid.is_empty() {
            return Ok(None);
        }

        let body = build_create_session_body(request);
        let resp = self
            .apply_session_headers(
                self.http.post(format!("{}/v1/sessions", self.base_url)),
                token,
                &request.organization_uuid,
            )
            .json(&body)
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("CreateBridgeSession: {err}")))?;
        let status = resp.status().as_u16();
        let data = response_json_from_status(resp, status).await?;
        if status != 200 && status != 201 {
            return Ok(None);
        }
        Ok(data
            .body
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string))
    }

    pub async fn get_bridge_session(
        &self,
        session_id: &str,
        organization_uuid: &str,
    ) -> std::result::Result<Option<BridgeSessionInfo>, BridgeApiError> {
        validate_bridge_id(session_id, "sessionId")?;
        let Some(token) = self
            .access_token
            .as_deref()
            .filter(|token| !token.is_empty())
        else {
            return Ok(None);
        };
        if organization_uuid.is_empty() {
            return Ok(None);
        }
        let resp = self
            .apply_session_headers(
                self.http
                    .get(format!("{}/v1/sessions/{}", self.base_url, session_id)),
                token,
                organization_uuid,
            )
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("GetBridgeSession: {err}")))?;
        let status = resp.status().as_u16();
        let data = response_json_from_status(resp, status).await?;
        if status != 200 {
            return Ok(None);
        }
        Ok(Some(BridgeSessionInfo {
            environment_id: data
                .body
                .get("environment_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            title: data
                .body
                .get("title")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        }))
    }

    /// TS `updateBridgeSessionTitle` parity: best-effort title sync. Errors
    /// and non-200 statuses are swallowed because they must not break the
    /// local CLI session.
    pub async fn update_bridge_session_title(
        &self,
        session_id: &str,
        title: &str,
        organization_uuid: &str,
    ) -> std::result::Result<(), BridgeApiError> {
        let compat_id = crate::bridge::session_id_compat::to_compat_session_id(session_id, true);
        validate_bridge_id(&compat_id, "sessionId")?;
        let Some(token) = self
            .access_token
            .as_deref()
            .filter(|token| !token.is_empty())
        else {
            return Ok(());
        };
        if organization_uuid.is_empty() {
            return Ok(());
        }
        let resp = self
            .apply_session_headers(
                self.http
                    .patch(format!("{}/v1/sessions/{}", self.base_url, compat_id)),
                token,
                organization_uuid,
            )
            .json(&serde_json::json!({ "title": title }))
            .send()
            .await;
        if let Ok(resp) = resp {
            let _ = resp.bytes().await;
        }
        Ok(())
    }

    pub async fn poll_for_work(
        &self,
        environment_id: &str,
        environment_secret: &str,
        reclaim_older_than_ms: Option<u64>,
    ) -> std::result::Result<Option<WorkResponse>, BridgeApiError> {
        validate_bridge_id(environment_id, "environmentId")?;
        let mut req = self.http.get(format!(
            "{}/v1/environments/{}/work/poll",
            self.base_url, environment_id
        ));
        if let Some(ms) = reclaim_older_than_ms {
            req = req.query(&[("reclaim_older_than_ms", ms)]);
        }
        let resp = self
            .apply_headers(req, environment_secret)
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("Poll: {err}")))?;
        let data = response_json(resp).await?;
        handle_error_status(data.status, &data.body, "Poll")?;
        if data.body.is_null() {
            return Ok(None);
        }
        serde_json::from_value(data.body)
            .map(Some)
            .map_err(|err| BridgeApiError::Other(format!("Poll: invalid response: {err}")))
    }

    pub async fn acknowledge_work(
        &self,
        environment_id: &str,
        work_id: &str,
        session_token: &str,
    ) -> std::result::Result<(), BridgeApiError> {
        self.post_empty_session_token(
            &format!("/v1/environments/{environment_id}/work/{work_id}/ack"),
            environment_id,
            Some(work_id),
            session_token,
            "Acknowledge",
        )
        .await
    }

    pub async fn stop_work(
        &self,
        environment_id: &str,
        work_id: &str,
        force: bool,
    ) -> std::result::Result<(), BridgeApiError> {
        validate_bridge_id(environment_id, "environmentId")?;
        validate_bridge_id(work_id, "workId")?;
        let token = self.resolve_auth()?;
        let resp = self
            .apply_headers(
                self.http.post(format!(
                    "{}/v1/environments/{}/work/{}/stop",
                    self.base_url, environment_id, work_id
                )),
                token,
            )
            .json(&serde_json::json!({ "force": force }))
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("StopWork: {err}")))?;
        let data = response_json(resp).await?;
        handle_error_status(data.status, &data.body, "StopWork")
    }

    pub async fn deregister_environment(
        &self,
        environment_id: &str,
    ) -> std::result::Result<(), BridgeApiError> {
        validate_bridge_id(environment_id, "environmentId")?;
        let token = self.resolve_auth()?;
        let resp = self
            .apply_headers(
                self.http.delete(format!(
                    "{}/v1/environments/bridge/{}",
                    self.base_url, environment_id
                )),
                token,
            )
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("Deregister: {err}")))?;
        let data = response_json(resp).await?;
        handle_error_status(data.status, &data.body, "Deregister")
    }

    pub async fn archive_session(
        &self,
        session_id: &str,
    ) -> std::result::Result<(), BridgeApiError> {
        validate_bridge_id(session_id, "sessionId")?;
        let token = self.resolve_auth()?;
        let resp = self
            .apply_headers(
                self.http.post(format!(
                    "{}/v1/sessions/{}/archive",
                    self.base_url, session_id
                )),
                token,
            )
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("ArchiveSession: {err}")))?;
        let status = resp.status().as_u16();
        if status == 409 {
            return Ok(());
        }
        let data = response_json_from_status(resp, status).await?;
        handle_error_status(data.status, &data.body, "ArchiveSession")
    }

    /// TS `archiveBridgeSession` parity for the org-scoped Sessions API.
    /// Best effort: missing auth/org, non-200, and network failures are not
    /// fatal to the local CLI shutdown path.
    pub async fn archive_bridge_session(
        &self,
        session_id: &str,
        organization_uuid: &str,
    ) -> std::result::Result<(), BridgeApiError> {
        validate_bridge_id(session_id, "sessionId")?;
        let Some(token) = self
            .access_token
            .as_deref()
            .filter(|token| !token.is_empty())
        else {
            return Ok(());
        };
        if organization_uuid.is_empty() {
            return Ok(());
        }
        let resp = self
            .apply_session_headers(
                self.http.post(format!(
                    "{}/v1/sessions/{}/archive",
                    self.base_url, session_id
                )),
                token,
                organization_uuid,
            )
            .json(&serde_json::json!({}))
            .send()
            .await;
        if let Ok(resp) = resp {
            let _ = resp.bytes().await;
        }
        Ok(())
    }

    pub async fn reconnect_session(
        &self,
        environment_id: &str,
        session_id: &str,
    ) -> std::result::Result<(), BridgeApiError> {
        validate_bridge_id(environment_id, "environmentId")?;
        validate_bridge_id(session_id, "sessionId")?;
        let token = self.resolve_auth()?;
        let resp = self
            .apply_headers(
                self.http.post(format!(
                    "{}/v1/environments/{}/bridge/reconnect",
                    self.base_url, environment_id
                )),
                token,
            )
            .json(&serde_json::json!({ "session_id": session_id }))
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("ReconnectSession: {err}")))?;
        let data = response_json(resp).await?;
        handle_error_status(data.status, &data.body, "ReconnectSession")
    }

    pub async fn heartbeat_work(
        &self,
        environment_id: &str,
        work_id: &str,
        session_token: &str,
    ) -> std::result::Result<(bool, String), BridgeApiError> {
        validate_bridge_id(environment_id, "environmentId")?;
        validate_bridge_id(work_id, "workId")?;
        let resp = self
            .apply_headers(
                self.http.post(format!(
                    "{}/v1/environments/{}/work/{}/heartbeat",
                    self.base_url, environment_id, work_id
                )),
                session_token,
            )
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("Heartbeat: {err}")))?;
        let data = response_json(resp).await?;
        handle_error_status(data.status, &data.body, "Heartbeat")?;
        Ok((
            data.body
                .get("lease_extended")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            data.body
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ))
    }

    pub async fn send_permission_response_event(
        &self,
        session_id: &str,
        event: &PermissionResponseEvent,
        session_token: &str,
    ) -> std::result::Result<(), BridgeApiError> {
        validate_bridge_id(session_id, "sessionId")?;
        let resp = self
            .apply_headers(
                self.http.post(format!(
                    "{}/v1/sessions/{}/events",
                    self.base_url, session_id
                )),
                session_token,
            )
            .json(&serde_json::json!({ "events": [event] }))
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("SendPermissionResponseEvent: {err}")))?;
        let data = response_json(resp).await?;
        handle_error_status(data.status, &data.body, "SendPermissionResponseEvent")
    }

    async fn post_empty_session_token(
        &self,
        path: &str,
        environment_id: &str,
        work_id: Option<&str>,
        session_token: &str,
        context: &str,
    ) -> std::result::Result<(), BridgeApiError> {
        validate_bridge_id(environment_id, "environmentId")?;
        if let Some(work_id) = work_id {
            validate_bridge_id(work_id, "workId")?;
        }
        let resp = self
            .apply_headers(
                self.http.post(format!("{}{}", self.base_url, path)),
                session_token,
            )
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|err| BridgeApiError::Other(format!("{context}: {err}")))?;
        let data = response_json(resp).await?;
        handle_error_status(data.status, &data.body, context)
    }
}

fn build_create_session_body(request: &CreateBridgeSessionRequest) -> Value {
    let mut body = serde_json::json!({
        "events": request.events,
        "session_context": {
            "sources": bridge_git_sources(request.git_repo_url.as_deref(), &request.branch),
            "outcomes": bridge_git_outcomes(request.git_repo_url.as_deref(), &request.branch),
            "model": request.model,
        },
        "environment_id": request.environment_id,
        "source": "remote-control",
    });
    let obj = body.as_object_mut().expect("body is object");
    if let Some(title) = request.title.as_deref() {
        obj.insert("title".to_string(), Value::String(title.to_string()));
    }
    if let Some(mode) = request.permission_mode.as_deref() {
        obj.insert(
            "permission_mode".to_string(),
            Value::String(mode.to_string()),
        );
    }
    body
}

fn bridge_git_sources(git_repo_url: Option<&str>, branch: &str) -> Vec<Value> {
    let Some(repo) = normalize_github_repo(git_repo_url) else {
        return Vec::new();
    };
    vec![serde_json::json!({
        "type": "git_repository",
        "url": repo.url,
        "revision": branch,
    })]
}

fn bridge_git_outcomes(git_repo_url: Option<&str>, branch: &str) -> Vec<Value> {
    let Some(repo) = normalize_github_repo(git_repo_url) else {
        return Vec::new();
    };
    vec![serde_json::json!({
        "type": "git_repository",
        "git_info": {
            "type": "github",
            "repo": repo.owner_repo,
            "branches": [format!("claude/{}", if branch.is_empty() { "task" } else { branch })],
        },
    })]
}

struct NormalizedGithubRepo {
    url: String,
    owner_repo: String,
}

fn normalize_github_repo(raw: Option<&str>) -> Option<NormalizedGithubRepo> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let trimmed = raw.trim_end_matches(".git");
    let owner_repo = if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        rest.to_string()
    } else if let Ok(url) = url::Url::parse(trimmed) {
        if url.host_str()? != "github.com" {
            return None;
        }
        url.path().trim_start_matches('/').to_string()
    } else if trimmed.split('/').count() == 2 {
        trimmed.to_string()
    } else {
        return None;
    };
    let mut parts = owner_repo.split('/');
    let owner = parts.next()?.trim();
    let name = parts.next()?.trim();
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return None;
    }
    let owner_repo = format!("{owner}/{name}");
    Some(NormalizedGithubRepo {
        url: format!("https://github.com/{owner_repo}"),
        owner_repo,
    })
}

pub fn validate_bridge_id(id: &str, label: &str) -> std::result::Result<(), BridgeApiError> {
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(BridgeApiError::Other(format!(
            "Invalid {label}: contains unsafe characters"
        )));
    }
    Ok(())
}

pub fn is_expired_error_type(error_type: Option<&str>) -> bool {
    error_type
        .map(|ty| ty.contains("expired") || ty.contains("lifetime"))
        .unwrap_or(false)
}

pub fn is_suppressible_403(err: &BridgeFatalError) -> bool {
    if err.status != 403 {
        return false;
    }
    let text = format!(
        "{} {}",
        err.message,
        err.error_type.clone().unwrap_or_default()
    )
    .to_ascii_lowercase();
    text.contains("external_poll_sessions")
        || text.contains("environments:manage")
        || text.contains("permission")
}

fn handle_error_status(
    status: u16,
    data: &Value,
    context: &str,
) -> std::result::Result<(), BridgeApiError> {
    if status == 200 || status == 204 {
        return Ok(());
    }
    let detail = extract_error_detail(data);
    let error_type = extract_error_type(data);
    match status {
        401 => Err(BridgeApiError::Fatal(BridgeFatalError {
            message: format!(
                "{context}: Authentication failed (401){}. {BRIDGE_LOGIN_INSTRUCTION}",
                detail
                    .as_deref()
                    .map(|d| format!(": {d}"))
                    .unwrap_or_default()
            ),
            status,
            error_type,
        })),
        403 => Err(BridgeApiError::Fatal(BridgeFatalError {
            message: if is_expired_error_type(error_type.as_deref()) {
                "Remote Control session has expired. Please restart with `claude remote-control` or /remote-control.".to_string()
            } else {
                format!(
                    "{context}: Access denied (403){}. Check your organization permissions.",
                    detail
                        .as_deref()
                        .map(|d| format!(": {d}"))
                        .unwrap_or_default()
                )
            },
            status,
            error_type,
        })),
        404 => Err(BridgeApiError::Fatal(BridgeFatalError {
            message: detail.unwrap_or_else(|| {
                format!("{context}: Not found (404). Remote Control may not be available for this organization.")
            }),
            status,
            error_type,
        })),
        410 => Err(BridgeApiError::Fatal(BridgeFatalError {
            message: detail.unwrap_or_else(|| {
                "Remote Control session has expired. Please restart with `claude remote-control` or /remote-control.".to_string()
            }),
            status,
            error_type: error_type.or_else(|| Some("environment_expired".to_string())),
        })),
        429 => Err(BridgeApiError::Other(format!(
            "{context}: Rate limited (429). Polling too frequently."
        ))),
        _ => Err(BridgeApiError::Other(format!(
            "{context}: Failed with status {status}{}",
            detail
                .as_deref()
                .map(|d| format!(": {d}"))
                .unwrap_or_default()
        ))),
    }
}

struct JsonResponse {
    status: u16,
    body: Value,
}

async fn response_json(
    resp: reqwest::Response,
) -> std::result::Result<JsonResponse, BridgeApiError> {
    let status = resp.status().as_u16();
    response_json_from_status(resp, status).await
}

async fn response_json_from_status(
    resp: reqwest::Response,
    status: u16,
) -> std::result::Result<JsonResponse, BridgeApiError> {
    let text = resp
        .text()
        .await
        .map_err(|err| BridgeApiError::Other(format!("reading response: {err}")))?;
    let body = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text))
    };
    Ok(JsonResponse { status, body })
}

fn extract_error_detail(data: &Value) -> Option<String> {
    data.pointer("/error/message")
        .or_else(|| data.get("message"))
        .or_else(|| data.get("detail"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn extract_error_type(data: &Value) -> Option<String> {
    data.pointer("/error/type")
        .or_else(|| data.get("type"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

#[allow(dead_code)]
fn _assert_send_sync() -> Result<()> {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<BridgeApiClient>();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bridge_ids_like_ts_safe_pattern() {
        assert!(validate_bridge_id("session_abc-123", "sessionId").is_ok());
        assert!(validate_bridge_id("", "sessionId").is_err());
        assert!(validate_bridge_id("../admin", "sessionId").is_err());
        assert!(validate_bridge_id("session.abc", "sessionId").is_err());
        assert!(validate_bridge_id("session/abc", "sessionId").is_err());
    }

    #[test]
    fn error_statuses_match_ts_bridge_api_messages() {
        let err = handle_error_status(
            401,
            &serde_json::json!({"error": {"message": "bad token", "type": "auth"}}),
            "Poll",
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("Authentication failed (401): bad token"));

        let err = handle_error_status(
            410,
            &serde_json::json!({"error": {"type": "environment_expired"}}),
            "Poll",
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("Remote Control session has expired"));

        let err = handle_error_status(429, &Value::Null, "Poll").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Poll: Rate limited (429). Polling too frequently."
        );
    }

    #[test]
    fn expired_and_suppressible_helpers_match_ts_edges() {
        assert!(is_expired_error_type(Some("environment_expired")));
        assert!(is_expired_error_type(Some("session_lifetime_exceeded")));
        assert!(!is_expired_error_type(Some("permission_denied")));

        let err = BridgeFatalError {
            message: "missing environments:manage".into(),
            status: 403,
            error_type: None,
        };
        assert!(is_suppressible_403(&err));
    }

    #[test]
    fn create_session_body_matches_ts_shape() {
        let body = build_create_session_body(&CreateBridgeSessionRequest {
            environment_id: "env_1".into(),
            organization_uuid: "org".into(),
            title: Some("Title".into()),
            events: vec![serde_json::json!({"type": "event", "data": {"type": "user"}})],
            git_repo_url: Some("git@github.com:owner/repo.git".into()),
            branch: "main".into(),
            model: "claude-opus-4-6".into(),
            permission_mode: Some("acceptEdits".into()),
        });
        assert_eq!(body["title"], "Title");
        assert_eq!(body["environment_id"], "env_1");
        assert_eq!(body["source"], "remote-control");
        assert_eq!(body["permission_mode"], "acceptEdits");
        assert_eq!(body["session_context"]["model"], "claude-opus-4-6");
        assert_eq!(
            body["session_context"]["sources"][0]["url"],
            "https://github.com/owner/repo"
        );
        assert_eq!(
            body["session_context"]["outcomes"][0]["git_info"]["repo"],
            "owner/repo"
        );
        assert_eq!(
            body["session_context"]["outcomes"][0]["git_info"]["branches"][0],
            "claude/main"
        );
    }

    #[test]
    fn github_repo_normalization_matches_ts_supported_inputs() {
        assert_eq!(
            normalize_github_repo(Some("https://github.com/a/b.git"))
                .unwrap()
                .owner_repo,
            "a/b"
        );
        assert_eq!(
            normalize_github_repo(Some("git@github.com:a/b.git"))
                .unwrap()
                .url,
            "https://github.com/a/b"
        );
        assert_eq!(
            normalize_github_repo(Some("a/b")).unwrap().owner_repo,
            "a/b"
        );
        assert!(normalize_github_repo(Some("https://gitlab.com/a/b")).is_none());
    }
}
