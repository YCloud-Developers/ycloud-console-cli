use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER};
use reqwest::StatusCode;
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use std::{fmt, path::Path};
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::pkce::PkcePair;
use crate::waba_assignment::{AssignmentRule, PhoneAssignment, PhoneIdsSearchRequest, PhoneNumber};

#[derive(Debug, Clone)]
pub struct DashboardClient {
    http: reqwest::Client,
    base_url: Url,
    timeout: Duration,
    invocation_id: String,
    invocation_mode: InvocationMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationMode {
    Interactive,
    Automation,
}

impl InvocationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Automation => "automation",
        }
    }

    fn retry_budget(self) -> RetryBudget {
        match self {
            Self::Interactive => RetryBudget::new(3, Duration::from_secs(5)),
            Self::Automation => RetryBudget::new(4, Duration::from_secs(20)),
        }
    }

    fn overall_timeout(self) -> Duration {
        match self {
            Self::Interactive => Duration::from_secs(30),
            Self::Automation => Duration::from_secs(60),
        }
    }
}

impl fmt::Display for InvocationMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy)]
struct RetryBudget {
    max_attempts: usize,
    max_wait: Duration,
}

impl RetryBudget {
    fn new(max_attempts: usize, max_wait: Duration) -> Self {
        Self {
            max_attempts,
            max_wait,
        }
    }
}

impl DashboardClient {
    pub fn new(base_url: String) -> Result<Self> {
        Self::new_with_mode(base_url, InvocationMode::Interactive)
    }

    pub fn new_with_timeout(base_url: String, timeout: Duration) -> Result<Self> {
        Self::build(base_url, InvocationMode::Interactive, timeout)
    }

    pub fn new_with_mode(base_url: String, invocation_mode: InvocationMode) -> Result<Self> {
        Self::build(base_url, invocation_mode, invocation_mode.overall_timeout())
    }

    fn build(base_url: String, invocation_mode: InvocationMode, timeout: Duration) -> Result<Self> {
        let mut parsed = Url::parse(&base_url).context("dashboard url must be absolute")?;
        if parsed.path() == "/" {
            parsed.set_path("");
        }
        Ok(Self {
            http: reqwest::Client::builder()
                .build()
                .context("failed to build dashboard HTTP client")?,
            base_url: parsed,
            timeout,
            invocation_id: random_identifier("inv"),
            invocation_mode,
        })
    }

    pub fn base_url(&self) -> String {
        self.base_url.as_str().trim_end_matches('/').to_string()
    }

    pub fn authorize_url(
        &self,
        profile: &str,
        permissions: &[String],
        state: &str,
        pkce: &PkcePair,
        redirect_uri: Option<&str>,
    ) -> Result<Url> {
        let mut url = self.join("/api/cli/auth/authorize")?;
        let mut query = url.query_pairs_mut();
        query
            .append_pair("responseType", "code")
            .append_pair("profile", profile)
            .append_pair("state", state)
            .append_pair("codeChallenge", &pkce.code_challenge)
            .append_pair("codeChallengeMethod", "S256");
        if !permissions.is_empty() {
            query.append_pair("permissions", &permissions.join(","));
        }
        if let Some(redirect_uri) = redirect_uri {
            query.append_pair("redirectUri", redirect_uri);
        }
        drop(query);
        Ok(url)
    }

    pub async fn exchange_token(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<ApiEnvelope<TokenData>> {
        self.post_json(
            "/api/cli/auth/token",
            None,
            &TokenExchangeRequest {
                grant_type: "authorization_code",
                code,
                code_verifier,
            },
        )
        .await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<ApiEnvelope<TokenData>> {
        self.post_json(
            "/api/cli/auth/refresh",
            None,
            &RefreshRequest {
                grant_type: "refresh_token",
                refresh_token,
            },
        )
        .await
    }

    pub async fn revoke(&self, access_token: &str, record_id: &str) -> Result<ApiEnvelope<bool>> {
        self.post_json(
            "/api/cli/auth/revoke",
            Some(access_token),
            &RevokeRequest { record_id },
        )
        .await
    }

    pub async fn whoami(&self, access_token: &str) -> Result<ApiEnvelope<WhoamiData>> {
        self.get_json_safe("/api/cli/v1/whoami", Some(access_token))
            .await
    }

    pub async fn tenants(&self, access_token: &str) -> Result<ApiEnvelope<TenantsData>> {
        self.get_json_safe("/api/cli/v1/tenants", Some(access_token))
            .await
    }

    pub async fn contacts_search(
        &self,
        access_token: &str,
        request: ContactsSearchRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json_safe("/api/cli/v1/contacts/search", Some(access_token), &request)
            .await
    }

    pub async fn contacts_metadata(
        &self,
        access_token: &str,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.get_json_safe("/api/cli/v1/contacts/metadata", Some(access_token))
            .await
    }

    pub async fn integrations_status(
        &self,
        access_token: &str,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.get_json_safe("/api/cli/v1/integrations/status", Some(access_token))
            .await
    }

    pub async fn whatsapp_analytics_outline(
        &self,
        access_token: &str,
        request: AnalyticsRangeRequest,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        let primary = format!(
            "/api/cli/v1/whatsapp/analytics/outline?startTime={}&endTime={}",
            millis_to_rfc3339(request.start_time)?,
            millis_to_rfc3339(request.end_time)?
        );
        self.get_json_safe(&primary, Some(access_token)).await
    }

    pub async fn whatsapp_delivery_analytics(
        &self,
        access_token: &str,
        request: &AnalyticsOverviewRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        let stable = AnalyticsV1Request::try_from(*request)?;
        self.post_json_safe(
            "/api/cli/v1/whatsapp/analytics/delivery",
            Some(access_token),
            &stable,
        )
        .await
    }

    pub async fn whatsapp_message_detail(
        &self,
        access_token: &str,
        request: &AnalyticsOverviewRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        let stable = AnalyticsV1Request::try_from(*request)?;
        self.post_json_safe(
            "/api/cli/v1/whatsapp/analytics/message-detail",
            Some(access_token),
            &stable,
        )
        .await
    }

    pub async fn whatsapp_failure_reason_share(
        &self,
        access_token: &str,
        request: &AnalyticsOverviewRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        let stable = AnalyticsV1Request::try_from(*request)?;
        self.post_json_safe(
            "/api/cli/v1/whatsapp/analytics/failure-reasons",
            Some(access_token),
            &stable,
        )
        .await
    }

    pub async fn whatsapp_logs_search(
        &self,
        access_token: &str,
        request: &AnalyticsLogsRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json_safe(
            "/api/cli/v1/whatsapp/messages/search",
            Some(access_token),
            request,
        )
        .await
    }

    pub async fn calling_logs_search(
        &self,
        access_token: &str,
        request: &AnalyticsCallingLogsRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json_safe(
            "/api/cli/v1/calling/logs/search",
            Some(access_token),
            request,
        )
        .await
    }

    pub async fn conversations_search(
        &self,
        access_token: &str,
        request: &serde_json::Value,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json_safe(
            "/api/cli/v1/inbox/conversations/search",
            Some(access_token),
            request,
        )
        .await
    }

    pub async fn whatsapp_phone_numbers(
        &self,
        access_token: &str,
        waba_id: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<ApiEnvelope<Vec<PhoneNumber>>> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        if let Some(waba_id) = waba_id {
            query.append_pair("wabaId", waba_id);
        }
        if let Some(cursor) = cursor {
            query.append_pair("cursor", cursor);
        }
        query.append_pair("limit", &limit.to_string());
        let path = format!("/api/cli/v1/whatsapp/phone-numbers?{}", query.finish());
        self.get_json_safe(&path, Some(access_token)).await
    }

    pub async fn inbox_phone_assignments(
        &self,
        access_token: &str,
        request: &PhoneIdsSearchRequest<'_>,
    ) -> Result<ApiEnvelope<Vec<PhoneAssignment>>> {
        self.post_json_safe(
            "/api/cli/v1/inbox/phone-assignments/search",
            Some(access_token),
            request,
        )
        .await
    }

    pub async fn inbox_assignment_rules(
        &self,
        access_token: &str,
        request: &PhoneIdsSearchRequest<'_>,
    ) -> Result<ApiEnvelope<Vec<AssignmentRule>>> {
        self.post_json_safe(
            "/api/cli/v1/inbox/assignment-rules/search",
            Some(access_token),
            request,
        )
        .await
    }

    pub async fn create_conversation_export(
        &self,
        access_token: &str,
        idempotency_key: &str,
        request: &serde_json::Value,
    ) -> Result<ApiEnvelope<ExportTask>> {
        self.post_json_idempotent(
            "/api/cli/v1/inbox/conversation-exports",
            access_token,
            idempotency_key,
            request,
        )
        .await
    }

    pub async fn create_contact_export(
        &self,
        access_token: &str,
        idempotency_key: &str,
        request: &serde_json::Value,
    ) -> Result<ApiEnvelope<ExportTask>> {
        self.post_json_idempotent(
            "/api/cli/v1/contact-exports",
            access_token,
            idempotency_key,
            request,
        )
        .await
    }

    pub async fn query_export(
        &self,
        access_token: &str,
        task_id: &str,
    ) -> Result<ApiEnvelope<ExportTask>> {
        self.post_json_safe(
            "/api/cli/v1/exports/query",
            Some(access_token),
            &serde_json::json!({"taskId": task_id}),
        )
        .await
    }

    pub async fn retry_export(
        &self,
        access_token: &str,
        task_id: &str,
        idempotency_key: &str,
    ) -> Result<ApiEnvelope<ExportTask>> {
        self.post_json_idempotent(
            "/api/cli/v1/exports/retry",
            access_token,
            idempotency_key,
            &serde_json::json!({"taskId": task_id}),
        )
        .await
    }

    pub async fn export_artifact_url(
        &self,
        access_token: &str,
        task_id: &str,
        artifact_type: &str,
        artifact_id: Option<&str>,
        part_number: Option<u32>,
    ) -> Result<ApiEnvelope<ArtifactUrl>> {
        self.post_json_safe(
            "/api/cli/v1/exports/artifact-url",
            Some(access_token),
            &serde_json::json!({
                "taskId": task_id,
                "artifactType": artifact_type,
                "artifactId": artifact_id,
                "partNumber": part_number
            }),
        )
        .await
    }

    pub async fn download_to_file(&self, signed_url: &str, path: &Path) -> Result<DownloadReceipt> {
        let response = self
            .http
            .get(Url::parse(signed_url).context("artifact URL must be absolute")?)
            .send()
            .await
            .map_err(map_request_error)?;
        if !response.status().is_success() {
            anyhow::bail!("artifact download failed with HTTP {}", response.status());
        }
        let mut response = response;
        let mut file = tokio::fs::File::create(path)
            .await
            .with_context(|| format!("failed to create {}", path.display()))?;
        let mut digest = Sha256::new();
        let mut size = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(map_request_error)? {
            file.write_all(&chunk)
                .await
                .with_context(|| format!("failed to write {}", path.display()))?;
            digest.update(&chunk);
            size += chunk.len() as u64;
        }
        file.flush().await?;
        Ok(DownloadReceipt {
            size,
            checksum_sha256: format!("{:x}", digest.finalize()),
        })
    }

    fn join(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("failed to build dashboard api url for {path}"))
    }

    async fn get_json_safe<T: DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
    ) -> Result<ApiEnvelope<T>> {
        let request = async {
            let budget = self.invocation_mode.retry_budget();
            let mut attempts = 0usize;
            let mut waited = Duration::ZERO;
            loop {
                attempts += 1;
                let response = self
                    .http
                    .get(self.join(path)?)
                    .headers(self.attempt_headers(token)?)
                    .send()
                    .await
                    .map_err(map_request_error)?;
                match decode_response(response).await? {
                    ResponseOutcome::Success(envelope) => return Ok(envelope),
                    ResponseOutcome::Failure(failure) => {
                        let Some(delay) = retry_delay(&failure, attempts, waited, budget) else {
                            return Err(failure.into_error());
                        };
                        tokio::time::sleep(delay).await;
                        waited += delay;
                    }
                }
            }
        };
        tokio::time::timeout(self.timeout, request)
            .await
            .map_err(|_| anyhow::anyhow!("dashboard API request timed out"))?
    }

    async fn post_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        token: Option<&str>,
        body: &B,
    ) -> Result<ApiEnvelope<T>> {
        let request = async {
            let response = self
                .http
                .post(self.join(path)?)
                .headers(self.attempt_headers(token)?)
                .json(body)
                .send()
                .await
                .map_err(map_request_error)?;
            parse_response(response).await
        };
        tokio::time::timeout(self.timeout, request)
            .await
            .map_err(|_| anyhow::anyhow!("dashboard API request timed out"))?
    }

    async fn post_json_safe<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        token: Option<&str>,
        body: &B,
    ) -> Result<ApiEnvelope<T>> {
        let request = async {
            let budget = self.invocation_mode.retry_budget();
            let mut attempts = 0usize;
            let mut waited = Duration::ZERO;
            loop {
                attempts += 1;
                let response = self
                    .http
                    .post(self.join(path)?)
                    .headers(self.attempt_headers(token)?)
                    .json(body)
                    .send()
                    .await
                    .map_err(map_request_error)?;
                match decode_response(response).await? {
                    ResponseOutcome::Success(envelope) => return Ok(envelope),
                    ResponseOutcome::Failure(failure) => {
                        let Some(delay) = retry_delay(&failure, attempts, waited, budget) else {
                            return Err(failure.into_error());
                        };
                        tokio::time::sleep(delay).await;
                        waited += delay;
                    }
                }
            }
        };
        tokio::time::timeout(self.timeout, request)
            .await
            .map_err(|_| anyhow::anyhow!("dashboard API request timed out"))?
    }

    async fn post_json_idempotent<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        access_token: &str,
        idempotency_key: &str,
        body: &B,
    ) -> Result<ApiEnvelope<T>> {
        let request = async {
            let mut headers = self.attempt_headers(Some(access_token))?;
            headers.insert(
                "idempotency-key",
                HeaderValue::from_str(idempotency_key).context("invalid idempotency key")?,
            );
            let response = self
                .http
                .post(self.join(path)?)
                .headers(headers)
                .json(body)
                .send()
                .await
                .map_err(map_request_error)?;
            parse_response(response).await
        };
        tokio::time::timeout(self.timeout, request)
            .await
            .map_err(|_| anyhow::anyhow!("dashboard API request timed out"))?
    }

    fn attempt_headers(&self, token: Option<&str>) -> Result<HeaderMap> {
        headers(
            token,
            &random_identifier("req"),
            &self.invocation_id,
            self.invocation_mode,
        )
    }
}

fn millis_to_rfc3339(value: i64) -> Result<String> {
    chrono::DateTime::from_timestamp_millis(value)
        .map(|date_time| date_time.to_rfc3339_opts(SecondsFormat::Millis, true))
        .with_context(|| format!("analytics time is outside the RFC 3339 range: {value}"))
}

fn map_request_error(error: reqwest::Error) -> anyhow::Error {
    if error.is_timeout() {
        anyhow::anyhow!("dashboard API request timed out")
    } else {
        error.into()
    }
}

fn headers(
    token: Option<&str>,
    request_id: &str,
    invocation_id: &str,
    invocation_mode: InvocationMode,
) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "x-request-id",
        HeaderValue::from_str(request_id).context("invalid generated request id")?,
    );
    headers.insert(
        "x-ycloud-invocation-id",
        HeaderValue::from_str(invocation_id).context("invalid generated invocation id")?,
    );
    headers.insert(
        "x-ycloud-invocation-mode",
        HeaderValue::from_static(invocation_mode.as_str()),
    );
    if let Some(token) = token {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .context("invalid access token header value")?,
        );
    }
    Ok(headers)
}

async fn parse_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<ApiEnvelope<T>> {
    match decode_response(response).await? {
        ResponseOutcome::Success(envelope) => Ok(envelope),
        ResponseOutcome::Failure(failure) => Err(failure.into_error()),
    }
}

enum ResponseOutcome<T> {
    Success(ApiEnvelope<T>),
    Failure(DashboardApiError),
}

#[derive(Debug, Clone)]
pub struct DashboardApiError {
    status: StatusCode,
    code: String,
    message: String,
    retryable: bool,
    details: Option<serde_json::Value>,
    request_id: Option<String>,
    trace_id: Option<String>,
    retry_after: Option<Duration>,
}

impl DashboardApiError {
    #[cfg(test)]
    pub(crate) fn for_test(status: StatusCode, error: ApiError) -> Self {
        Self {
            status,
            code: error.code,
            message: error.message,
            retryable: error.retryable,
            details: error.details,
            request_id: None,
            trace_id: None,
            retry_after: None,
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn details(&self) -> Option<&serde_json::Value> {
        self.details.as_ref()
    }

    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    fn into_error(self) -> anyhow::Error {
        self.into()
    }
}

impl fmt::Display for DashboardApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "request failed with HTTP {}: {}: {} (requestId={}, traceId={})",
            self.status,
            self.code,
            self.message,
            self.request_id.as_deref().unwrap_or_default(),
            self.trace_id.as_deref().unwrap_or_default()
        )
    }
}

impl std::error::Error for DashboardApiError {}

async fn decode_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<ResponseOutcome<T>> {
    let status = response.status();
    let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
    let text = response.text().await.map_err(map_request_error)?;
    if !status.is_success() {
        if let Ok(envelope) = serde_json::from_str::<ApiEnvelope<serde_json::Value>>(&text) {
            if let Some(error) = envelope.error {
                return Ok(ResponseOutcome::Failure(DashboardApiError {
                    status,
                    code: error.code,
                    message: error.message,
                    retryable: error.retryable,
                    details: error.details,
                    request_id: envelope.request_id,
                    trace_id: envelope.trace_id,
                    retry_after,
                }));
            }
        }
        return Ok(ResponseOutcome::Failure(DashboardApiError {
            status,
            code: "http_error".to_string(),
            message: status
                .canonical_reason()
                .unwrap_or("request failed")
                .to_string(),
            retryable: false,
            details: None,
            request_id: None,
            trace_id: None,
            retry_after,
        }));
    }
    let envelope: ApiEnvelope<T> =
        serde_json::from_str(&text).context("failed to parse dashboard response")?;
    if envelope.code != 0 {
        let (code, message, retryable, details) = envelope.error.as_ref().map_or_else(
            || {
                (
                    envelope.code.to_string(),
                    envelope.message.clone().unwrap_or_default(),
                    false,
                    None,
                )
            },
            |error| {
                (
                    error.code.clone(),
                    error.message.clone(),
                    error.retryable,
                    error.details.clone(),
                )
            },
        );
        return Ok(ResponseOutcome::Failure(DashboardApiError {
            status,
            code,
            message,
            retryable,
            details,
            request_id: envelope.request_id,
            trace_id: envelope.trace_id,
            retry_after,
        }));
    }
    Ok(ResponseOutcome::Success(envelope))
}

fn retry_delay(
    failure: &DashboardApiError,
    attempts: usize,
    waited: Duration,
    budget: RetryBudget,
) -> Option<Duration> {
    if failure.status != StatusCode::TOO_MANY_REQUESTS
        || failure.code != "rate_limited"
        || !failure.retryable
    {
        return None;
    }
    if attempts >= budget.max_attempts {
        return None;
    }
    let delay = failure.retry_after.unwrap_or_else(|| {
        let ceiling_ms = 250u64.saturating_mul(1u64 << attempts.saturating_sub(1).min(6));
        Duration::from_millis(rand::thread_rng().gen_range(0..=ceiling_ms))
    });
    if waited.saturating_add(delay) > budget.max_wait {
        return None;
    }
    Some(delay)
}

fn parse_retry_after(value: Option<&HeaderValue>) -> Option<Duration> {
    let value = value?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let deadline = chrono::DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    let millis = (deadline - Utc::now()).num_milliseconds().max(0) as u64;
    Some(Duration::from_millis(millis))
}

fn random_identifier(prefix: &str) -> String {
    format!("{prefix}-{:032x}", rand::random::<u128>())
}

#[derive(Debug, Deserialize)]
pub struct ApiEnvelope<T> {
    pub code: i64,
    #[serde(alias = "msg")]
    pub message: Option<String>,
    pub data: Option<T>,
    #[serde(default)]
    pub error: Option<ApiError>,
    #[serde(rename = "requestId", default)]
    pub request_id: Option<String>,
    #[serde(rename = "traceId", default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub pagination: Option<ApiPagination>,
    #[serde(default)]
    pub warnings: Vec<ApiWarning>,
}

#[derive(Debug, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ApiPagination {
    #[serde(rename = "nextCursor", default)]
    pub next_cursor: Option<String>,
    #[serde(rename = "hasMore")]
    pub has_more: bool,
    #[serde(default)]
    pub total: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ApiWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExportTask {
    pub task_id: String,
    pub task_type: String,
    pub status: String,
    #[serde(default)]
    pub progress: u32,
    #[serde(default)]
    pub record_count: Option<u64>,
    #[serde(default)]
    pub previous_task_id: Option<String>,
    #[serde(default)]
    pub truncated: Option<bool>,
    #[serde(default)]
    pub truncation_reason: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<ExportArtifact>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
}

impl ExportTask {
    pub fn terminal(&self) -> bool {
        matches!(
            self.status.to_ascii_uppercase().as_str(),
            "FINISHED" | "FAILED" | "PARTIAL_SUCCESS"
        )
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExportArtifact {
    #[serde(default)]
    pub artifact_id: Option<String>,
    pub r#type: String,
    #[serde(default)]
    pub part_number: Option<u32>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    pub status: String,
    #[serde(default)]
    pub record_count: Option<u64>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub checksum_sha256: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactUrl {
    pub task_id: String,
    #[serde(default)]
    pub artifact_id: Option<String>,
    pub artifact_type: String,
    #[serde(default)]
    pub part_number: Option<u32>,
    pub file_name: String,
    pub url: String,
    pub expires_at: i64,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub checksum_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadReceipt {
    pub size: u64,
    pub checksum_sha256: String,
}

impl<T> ApiEnvelope<T> {
    pub fn require_data(self, label: &str) -> Result<T> {
        self.data
            .with_context(|| format!("{label} response missing data"))
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct TokenData {
    #[serde(rename = "tokenType")]
    pub token_type: String,
    #[serde(rename = "accessToken")]
    pub access_token: String,
    #[serde(rename = "refreshToken")]
    pub refresh_token: String,
    #[serde(rename = "recordId", deserialize_with = "deserialize_string_or_number")]
    pub record_id: String,
    #[serde(rename = "requestedPermissions")]
    pub requested_permissions: Vec<String>,
    #[serde(rename = "permissionModelVersion")]
    pub permission_model_version: u32,
}

fn deserialize_string_or_number<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(value) => Ok(value),
        serde_json::Value::Number(value) => Ok(value.to_string()),
        _ => Err(serde::de::Error::custom("expected string or number")),
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct WhoamiData {
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(rename = "tenantId")]
    pub tenant_id: String,
    #[serde(rename = "requestedPermissions", default)]
    pub requested_permissions: Vec<String>,
    #[serde(rename = "effectivePermissions", default)]
    pub effective_permissions: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct TenantsData {
    #[serde(default)]
    pub tenants: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct TokenExchangeRequest<'a> {
    #[serde(rename = "grantType")]
    grant_type: &'a str,
    code: &'a str,
    #[serde(rename = "codeVerifier")]
    code_verifier: &'a str,
}

#[derive(Debug, Serialize)]
struct RefreshRequest<'a> {
    #[serde(rename = "grantType")]
    grant_type: &'a str,
    #[serde(rename = "refreshToken")]
    refresh_token: &'a str,
}

#[derive(Debug, Serialize)]
struct RevokeRequest<'a> {
    #[serde(rename = "recordId")]
    record_id: &'a str,
}

#[derive(Debug, Serialize)]
pub struct ContactsSearchRequest<'a> {
    #[serde(rename = "pageNo")]
    pub page_no: u32,
    #[serde(rename = "pageSize")]
    pub page_size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<&'a str>,
}

#[derive(Debug, Copy, Clone)]
pub struct AnalyticsRangeRequest {
    pub start_time: i64,
    pub end_time: i64,
}

#[derive(Debug, Serialize, Copy, Clone)]
pub struct AnalyticsOverviewRequest<'a> {
    #[serde(rename = "startTime")]
    pub start_time: i64,
    #[serde(rename = "endTime")]
    pub end_time: i64,
    pub timezone: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<&'a str>,
    #[serde(rename = "regionCode", skip_serializing_if = "Option::is_none")]
    pub region_code: Option<&'a str>,
    #[serde(rename = "messageCategory", skip_serializing_if = "Option::is_none")]
    pub message_category: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct AnalyticsV1Request<'a> {
    #[serde(rename = "startTime")]
    start_time: String,
    #[serde(rename = "endTime")]
    end_time: String,
    timezone: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<&'a str>,
    #[serde(rename = "regionCode", skip_serializing_if = "Option::is_none")]
    region_code: Option<&'a str>,
    #[serde(rename = "messageCategory", skip_serializing_if = "Option::is_none")]
    message_category: Option<&'a str>,
}

impl<'a> TryFrom<AnalyticsOverviewRequest<'a>> for AnalyticsV1Request<'a> {
    type Error = anyhow::Error;

    fn try_from(request: AnalyticsOverviewRequest<'a>) -> Result<Self> {
        Ok(Self {
            start_time: millis_to_rfc3339(request.start_time)?,
            end_time: millis_to_rfc3339(request.end_time)?,
            timezone: request.timezone,
            from: request.from,
            region_code: request.region_code,
            message_category: request.message_category,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct AnalyticsLogsRequest<'a> {
    pub direction: &'a str,
    #[serde(rename = "startTime")]
    pub start_time: i64,
    #[serde(rename = "endTime")]
    pub end_time: i64,
    #[serde(rename = "pageNo")]
    pub page_no: u32,
    #[serde(rename = "pageSize")]
    pub page_size: u32,
    pub timezone: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<&'a str>,
    #[serde(rename = "businessPhones", skip_serializing_if = "Vec::is_empty")]
    pub business_phones: Vec<&'a str>,
    #[serde(rename = "toRegionCodes", skip_serializing_if = "Vec::is_empty")]
    pub to_region_codes: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smb: Option<bool>,
    #[serde(rename = "pricingCategory", skip_serializing_if = "Vec::is_empty")]
    pub pricing_category: Vec<&'a str>,
}

#[derive(Debug, Serialize)]
pub struct AnalyticsCallingLogsRequest<'a> {
    #[serde(rename = "startTime")]
    pub start_time: i64,
    #[serde(rename = "endTime")]
    pub end_time: i64,
    #[serde(rename = "pageNo")]
    pub page_no: u32,
    #[serde(rename = "pageSize")]
    pub page_size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub directions: Vec<&'a str>,
    #[serde(rename = "regionCodes", skip_serializing_if = "Vec::is_empty")]
    pub region_codes: Vec<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub status: Vec<&'a str>,
    #[serde(rename = "phoneNumberIds", skip_serializing_if = "Vec::is_empty")]
    pub phone_number_ids: Vec<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pkce::challenge_for_verifier;

    #[test]
    fn authorize_url_matches_backend_contract() {
        let client = DashboardClient::new("http://127.0.0.1:8036".to_string()).unwrap();
        let pkce = challenge_for_verifier("verifier");
        let url = client
            .authorize_url(
                "analytics-read",
                &["yc.integration.status.read".to_string()],
                "state-1",
                &pkce,
                None,
            )
            .unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

        assert_eq!(url.path(), "/api/cli/auth/authorize");
        assert_eq!(query.get("responseType"), Some(&"code".to_string()));
        assert_eq!(query.get("profile"), Some(&"analytics-read".to_string()));
        assert_eq!(
            query.get("permissions"),
            Some(&"yc.integration.status.read".to_string())
        );
        assert_eq!(query.get("scope"), None);
        assert_eq!(query.get("state"), Some(&"state-1".to_string()));
        assert_eq!(query.get("codeChallengeMethod"), Some(&"S256".to_string()));
        assert_eq!(query.get("codeChallenge"), Some(&pkce.code_challenge));
        assert_eq!(query.get("redirectUri"), None);
    }

    #[test]
    fn authorize_url_includes_redirect_uri_when_present() {
        let client = DashboardClient::new("https://dashboard.example".to_string()).unwrap();
        let pkce = challenge_for_verifier("verifier");

        let url = client
            .authorize_url(
                "basic",
                &[],
                "state-1",
                &pkce,
                Some("http://127.0.0.1:39123/callback"),
            )
            .unwrap();
        let query: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        assert_eq!(
            query.get("redirectUri"),
            Some(&"http://127.0.0.1:39123/callback".to_string())
        );
    }

    #[test]
    fn token_data_accepts_numeric_record_id() {
        let token: TokenData = serde_json::from_value(serde_json::json!({
            "tokenType": "Bearer",
            "accessToken": "YCLI.access",
            "refreshToken": "YCLI.refresh",
            "recordId": 1272676752573050880u64,
            "requestedPermissions": ["yc.identity.current.read"],
            "permissionModelVersion": 1
        }))
        .unwrap();

        assert_eq!(token.record_id, "1272676752573050880");
    }

    #[test]
    fn token_data_requires_permission_snapshot_and_model_version() {
        for missing_field in ["requestedPermissions", "permissionModelVersion"] {
            let mut value = serde_json::json!({
                "tokenType": "Bearer",
                "accessToken": "YCLI.access",
                "refreshToken": "YCLI.refresh",
                "recordId": "record-1",
                "requestedPermissions": ["yc.identity.current.read"],
                "permissionModelVersion": 1
            });
            value.as_object_mut().unwrap().remove(missing_field);

            assert!(serde_json::from_value::<TokenData>(value).is_err());
        }
    }

    #[test]
    fn api_envelope_accepts_click_msg_field() {
        let envelope: ApiEnvelope<serde_json::Value> = serde_json::from_value(serde_json::json!({
            "code": -1,
            "msg": "forbidden"
        }))
        .unwrap();

        assert_eq!(envelope.message.as_deref(), Some("forbidden"));
        assert!(envelope.pagination.is_none());
        assert!(envelope.warnings.is_empty());
    }

    #[test]
    fn api_envelope_parses_stable_extensions() {
        let envelope: ApiEnvelope<serde_json::Value> = serde_json::from_value(serde_json::json!({
            "code": 403,
            "msg": "Permission denied",
            "data": null,
            "error": {
                "code": "permission_denied",
                "message": "Permission denied",
                "retryable": false,
                "details": {"requiredPermission": "yc.integration.status.read"}
            },
            "requestId": "request-1",
            "traceId": "trace-1",
            "pagination": {"nextCursor": "cursor-2", "hasMore": true, "total": 42},
            "warnings": [{"code": "partial", "message": "Partial data"}],
            "futureField": true
        }))
        .unwrap();

        assert_eq!(envelope.error.as_ref().unwrap().code, "permission_denied");
        assert_eq!(
            envelope.error.as_ref().unwrap().details.as_ref().unwrap()["requiredPermission"],
            "yc.integration.status.read"
        );
        assert_eq!(
            envelope.pagination.as_ref().unwrap().next_cursor.as_deref(),
            Some("cursor-2")
        );
        assert!(envelope.pagination.as_ref().unwrap().has_more);
        assert_eq!(envelope.pagination.as_ref().unwrap().total, Some(42));
        assert_eq!(envelope.warnings[0].code, "partial");
        assert_eq!(envelope.warnings[0].message, "Partial data");
    }
}
