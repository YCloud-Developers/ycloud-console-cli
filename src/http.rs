use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use url::Url;

use crate::pkce::PkcePair;

#[derive(Debug, Clone)]
pub struct DashboardClient {
    http: reqwest::Client,
    base_url: Url,
}

impl DashboardClient {
    pub fn new(base_url: String) -> Result<Self> {
        let mut parsed = Url::parse(&base_url).context("dashboard url must be absolute")?;
        if parsed.path() == "/" {
            parsed.set_path("");
        }
        Ok(Self {
            http: reqwest::Client::new(),
            base_url: parsed,
        })
    }

    pub fn base_url(&self) -> String {
        self.base_url.as_str().trim_end_matches('/').to_string()
    }

    pub fn authorize_url(&self, scope: &str, state: &str, pkce: &PkcePair) -> Result<Url> {
        let mut url = self.join("/api/cli/auth/authorize")?;
        url.query_pairs_mut()
            .append_pair("responseType", "code")
            .append_pair("scope", scope)
            .append_pair("state", state)
            .append_pair("codeChallenge", &pkce.code_challenge)
            .append_pair("codeChallengeMethod", "S256");
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
        self.get_json("/api/cli/auth/whoami", Some(access_token))
            .await
    }

    pub async fn tenants(&self, access_token: &str) -> Result<ApiEnvelope<TenantsData>> {
        self.get_json("/api/cli/auth/tenants/list", Some(access_token))
            .await
    }

    pub async fn contacts_search(
        &self,
        access_token: &str,
        request: ContactsSearchRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json("/api/contacts/search", Some(access_token), &request)
            .await
    }

    pub async fn whatsapp_analytics_outline(
        &self,
        access_token: &str,
        request: AnalyticsRangeRequest,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        let path = format!(
            "/api/whatsapp/analytics/outline?startTime={}&endTime={}",
            request.start_time, request.end_time
        );
        self.get_json(&path, Some(access_token)).await
    }

    pub async fn whatsapp_delivery_analytics(
        &self,
        access_token: &str,
        request: &AnalyticsOverviewRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json(
            "/api/whatsapp/analytics/deliveryAnalytics",
            Some(access_token),
            request,
        )
        .await
    }

    pub async fn whatsapp_message_detail(
        &self,
        access_token: &str,
        request: &AnalyticsOverviewRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json(
            "/api/whatsapp/analytics/messageDetail",
            Some(access_token),
            request,
        )
        .await
    }

    pub async fn whatsapp_failure_reason_share(
        &self,
        access_token: &str,
        request: &AnalyticsOverviewRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json(
            "/api/whatsapp/analytics/failureReasonShare",
            Some(access_token),
            request,
        )
        .await
    }

    pub async fn whatsapp_logs_search(
        &self,
        access_token: &str,
        request: &AnalyticsLogsRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json("/api/whatsapp/message/search", Some(access_token), request)
            .await
    }

    pub async fn calling_logs_search(
        &self,
        access_token: &str,
        request: &AnalyticsCallingLogsRequest<'_>,
    ) -> Result<ApiEnvelope<serde_json::Value>> {
        self.post_json("/api/calling/logs/search", Some(access_token), request)
            .await
    }

    fn join(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("failed to build dashboard api url for {path}"))
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
    ) -> Result<ApiEnvelope<T>> {
        let headers = headers(token)?;
        let response = self
            .http
            .get(self.join(path)?)
            .headers(headers)
            .send()
            .await?;
        parse_response(response).await
    }

    async fn post_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        token: Option<&str>,
        body: &B,
    ) -> Result<ApiEnvelope<T>> {
        let headers = headers(token)?;
        let response = self
            .http
            .post(self.join(path)?)
            .headers(headers)
            .json(body)
            .send()
            .await?;
        parse_response(response).await
    }
}

fn headers(token: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
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
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("request failed with HTTP {status}: {text}");
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
    Ok(envelope)
}

#[derive(Debug, Deserialize)]
pub struct ApiEnvelope<T> {
    pub code: i64,
    #[serde(alias = "msg")]
    pub message: Option<String>,
    pub data: Option<T>,
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
    #[serde(default)]
    pub permissions: Vec<String>,
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

#[derive(Debug, Serialize)]
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
            .authorize_url("developers", "state-1", &pkce)
            .unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

        assert_eq!(url.path(), "/api/cli/auth/authorize");
        assert_eq!(query.get("responseType"), Some(&"code".to_string()));
        assert_eq!(query.get("scope"), Some(&"developers".to_string()));
        assert_eq!(query.get("state"), Some(&"state-1".to_string()));
        assert_eq!(query.get("codeChallengeMethod"), Some(&"S256".to_string()));
        assert_eq!(query.get("codeChallenge"), Some(&pkce.code_challenge));
    }

    #[test]
    fn token_data_accepts_numeric_record_id() {
        let token: TokenData = serde_json::from_value(serde_json::json!({
            "tokenType": "Bearer",
            "accessToken": "YCLI.access",
            "refreshToken": "YCLI.refresh",
            "recordId": 1272676752573050880u64
        }))
        .unwrap();

        assert_eq!(token.record_id, "1272676752573050880");
    }

    #[test]
    fn api_envelope_accepts_click_msg_field() {
        let envelope: ApiEnvelope<serde_json::Value> = serde_json::from_value(serde_json::json!({
            "code": -1,
            "msg": "forbidden"
        }))
        .unwrap();

        assert_eq!(envelope.message.as_deref(), Some("forbidden"));
    }
}
