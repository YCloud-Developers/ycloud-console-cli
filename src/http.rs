use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER};
use reqwest::StatusCode;
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use std::fmt;
use std::time::Duration;
use url::Url;

use crate::pkce::PkcePair;

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
        self.get_json_with_fallback(
            "/api/cli/v1/whoami",
            "/api/cli/auth/whoami",
            Some(access_token),
        )
        .await
    }

    pub async fn tenants(&self, access_token: &str) -> Result<ApiEnvelope<TenantsData>> {
        self.get_json_with_fallback(
            "/api/cli/v1/tenants",
            "/api/cli/auth/tenants/list",
            Some(access_token),
        )
        .await
    }

    pub async fn contacts_search(
        &self,
        access_token: &str,
        request: ContactsSearchRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json_safe(
            "/api/cli/read/contacts/search",
            Some(access_token),
            &request,
        )
        .await
    }

    pub async fn contacts_metadata(
        &self,
        access_token: &str,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.get_json_with_fallback(
            "/api/cli/v1/contacts/metadata",
            "/api/cli/read/contacts/metadata",
            Some(access_token),
        )
        .await
    }

    pub async fn integrations_status(
        &self,
        access_token: &str,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.get_json_with_fallback(
            "/api/cli/v1/integrations/status",
            "/api/cli/read/integrations/status",
            Some(access_token),
        )
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
        let legacy = format!(
            "/api/cli/read/whatsapp/analytics/outline?startTime={}&endTime={}",
            request.start_time, request.end_time
        );
        self.get_json_with_fallback(&primary, &legacy, Some(access_token))
            .await
    }

    pub async fn whatsapp_delivery_analytics(
        &self,
        access_token: &str,
        request: &AnalyticsOverviewRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        let stable = AnalyticsV1Request::try_from(*request)?;
        self.post_json_with_fallback(
            "/api/cli/v1/whatsapp/analytics/delivery",
            &stable,
            "/api/cli/read/whatsapp/analytics/delivery",
            request,
            Some(access_token),
        )
        .await
    }

    pub async fn whatsapp_message_detail(
        &self,
        access_token: &str,
        request: &AnalyticsOverviewRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        let stable = AnalyticsV1Request::try_from(*request)?;
        self.post_json_with_fallback(
            "/api/cli/v1/whatsapp/analytics/message-detail",
            &stable,
            "/api/cli/read/whatsapp/analytics/message-detail",
            request,
            Some(access_token),
        )
        .await
    }

    pub async fn whatsapp_failure_reason_share(
        &self,
        access_token: &str,
        request: &AnalyticsOverviewRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        let stable = AnalyticsV1Request::try_from(*request)?;
        self.post_json_with_fallback(
            "/api/cli/v1/whatsapp/analytics/failure-reasons",
            &stable,
            "/api/cli/read/whatsapp/analytics/failure-reasons",
            request,
            Some(access_token),
        )
        .await
    }

    pub async fn whatsapp_logs_search(
        &self,
        access_token: &str,
        request: &AnalyticsLogsRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json_safe(
            "/api/cli/read/whatsapp/messages/search",
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
            "/api/cli/read/calling/logs/search",
            Some(access_token),
            request,
        )
        .await
    }

    fn join(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("failed to build dashboard api url for {path}"))
    }

    async fn get_json_with_fallback<T: DeserializeOwned>(
        &self,
        primary_path: &str,
        legacy_path: &str,
        token: Option<&str>,
    ) -> Result<ApiEnvelope<T>> {
        let request = async {
            let budget = self.invocation_mode.retry_budget();
            let mut attempts = 0usize;
            let mut waited = Duration::ZERO;
            let mut path = primary_path;
            loop {
                attempts += 1;
                let response = self
                    .http
                    .get(self.join(path)?)
                    .headers(self.attempt_headers(token)?)
                    .send()
                    .await
                    .map_err(map_request_error)?;
                if path == primary_path && should_fallback(response.status()) {
                    path = legacy_path;
                    continue;
                }
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

    async fn post_json_with_fallback<
        T: DeserializeOwned,
        P: Serialize + ?Sized,
        L: Serialize + ?Sized,
    >(
        &self,
        primary_path: &str,
        primary_body: &P,
        legacy_path: &str,
        legacy_body: &L,
        token: Option<&str>,
    ) -> Result<ApiEnvelope<T>> {
        let request = async {
            let budget = self.invocation_mode.retry_budget();
            let mut attempts = 0usize;
            let mut waited = Duration::ZERO;
            let mut legacy = false;
            loop {
                attempts += 1;
                let response = if legacy {
                    self.http
                        .post(self.join(legacy_path)?)
                        .headers(self.attempt_headers(token)?)
                        .json(legacy_body)
                        .send()
                        .await
                        .map_err(map_request_error)?
                } else {
                    self.http
                        .post(self.join(primary_path)?)
                        .headers(self.attempt_headers(token)?)
                        .json(primary_body)
                        .send()
                        .await
                        .map_err(map_request_error)?
                };
                if !legacy && should_fallback(response.status()) {
                    legacy = true;
                    continue;
                }
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

    fn attempt_headers(&self, token: Option<&str>) -> Result<HeaderMap> {
        headers(
            token,
            &random_identifier("req"),
            &self.invocation_id,
            self.invocation_mode,
        )
    }
}

fn should_fallback(status: StatusCode) -> bool {
    status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED
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
    Failure(RateLimitFailure),
}

#[derive(Debug)]
struct RateLimitFailure {
    status: StatusCode,
    code: String,
    message: String,
    request_id: Option<String>,
    trace_id: Option<String>,
    retry_after: Option<Duration>,
}

impl RateLimitFailure {
    fn into_error(self) -> anyhow::Error {
        let retry_after = self
            .retry_after
            .map(|value| format!(", retryAfterSeconds={}", value.as_secs()))
            .unwrap_or_default();
        anyhow::anyhow!(
            "request failed with HTTP {}: {}: {} (requestId={}, traceId={}{}); retry budget exhausted or retry is not safe",
            self.status,
            self.code,
            self.message,
            self.request_id.unwrap_or_default(),
            self.trace_id.unwrap_or_default(),
            retry_after
        )
    }
}

async fn decode_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<ResponseOutcome<T>> {
    let status = response.status();
    let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
    let text = response.text().await.map_err(map_request_error)?;
    if !status.is_success() {
        if let Ok(envelope) = serde_json::from_str::<ApiEnvelope<serde_json::Value>>(&text) {
            if let Some(error) = envelope.error {
                if status == StatusCode::TOO_MANY_REQUESTS
                    && error.code == "rate_limited"
                    && error.retryable
                {
                    return Ok(ResponseOutcome::Failure(RateLimitFailure {
                        status,
                        code: error.code,
                        message: error.message,
                        request_id: envelope.request_id,
                        trace_id: envelope.trace_id,
                        retry_after,
                    }));
                }
                anyhow::bail!(
                    "request failed with HTTP {status}: {}: {} (requestId={}, traceId={})",
                    error.code,
                    error.message,
                    envelope.request_id.unwrap_or_default(),
                    envelope.trace_id.unwrap_or_default()
                );
            }
        }
        anyhow::bail!("request failed with HTTP {status}");
    }
    let envelope: ApiEnvelope<T> =
        serde_json::from_str(&text).context("failed to parse dashboard response")?;
    if envelope.code != 0 {
        anyhow::bail!(
            "dashboard api rejected request: code={}, message={}",
            envelope.code,
            envelope.message.clone().unwrap_or_default()
        );
    }
    Ok(ResponseOutcome::Success(envelope))
}

fn retry_delay(
    failure: &RateLimitFailure,
    attempts: usize,
    waited: Duration,
    budget: RetryBudget,
) -> Option<Duration> {
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
