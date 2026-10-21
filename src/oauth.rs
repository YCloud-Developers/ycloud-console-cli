use crate::{
    cli::{LoginArgs, PermissionProfile},
    config::{AuthConfig, Config, DashboardConfig, OAuthSession},
    http::DashboardClient,
    pkce::{self, PkcePair},
};
use anyhow::{bail, Context, Result};
use reqwest::{redirect::Policy, Response};
use serde::Deserialize;
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;

pub const CLIENT_ID: &str = "ycloud-console-cli";
const BODY_LIMIT: usize = 65536;

pub struct OAuthClient {
    http: reqwest::Client,
    issuer: String,
    client_id: String,
    resource: String,
    business_base: String,
}

#[derive(Deserialize)]
struct ServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    revocation_endpoint: String,
    code_challenge_methods_supported: Vec<String>,
    grant_types_supported: Vec<String>,
    response_types_supported: Vec<String>,
    token_endpoint_auth_methods_supported: Vec<String>,
    authorization_response_iss_parameter_supported: bool,
}

#[derive(Deserialize)]
struct ResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    scopes_supported: Vec<String>,
    bearer_methods_supported: Vec<String>,
}

pub struct OAuthMetadata {
    issuer: String,
    authorization_endpoint: String,
    pub token_endpoint: String,
    pub revocation_endpoint: String,
    pub scopes: Vec<String>,
}

#[derive(Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub scope: String,
}

impl std::fmt::Debug for OAuthToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthToken")
            .field("credentials", &"[redacted]")
            .finish()
    }
}

impl OAuthClient {
    pub fn new(issuer: &str) -> Result<Self> {
        Self::configured(issuer, CLIENT_ID, &format!("{issuer}/api/cli"), issuer)
    }

    pub fn configured(
        issuer: &str,
        client_id: &str,
        resource: &str,
        business_base: &str,
    ) -> Result<Self> {
        let issuer_uri = origin(issuer)?;
        let business_uri = origin(business_base)?;
        let resource_uri = safe_url(resource)?;
        if issuer != issuer_uri.as_str().trim_end_matches('/')
            || business_base != business_uri.as_str().trim_end_matches('/')
            || resource_uri.origin() != business_uri.origin()
            || resource_uri.path() != "/api/cli"
            || resource_uri.query().is_some()
            || resource != format!("{business_base}/api/cli")
            || client_id.is_empty()
            || client_id.len() > 128
            || !client_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            bail!("Invalid OAuth issuer/client/resource/business binding");
        }
        Ok(Self {
            http: reqwest::Client::builder()
                .redirect(Policy::none())
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .build()
                .context("failed to build OAuth client")?,
            issuer: issuer.to_string(),
            client_id: client_id.to_string(),
            resource: resource.to_string(),
            business_base: business_base.to_string(),
        })
    }

    pub fn resource(&self) -> String {
        self.resource.clone()
    }

    pub fn require_session(&self, session: &OAuthSession) -> Result<()> {
        if self.issuer != session.issuer
            || self.resource != session.resource
            || self.client_id != session.client_id
        {
            bail!("OAuth session binding changed; log in again");
        }
        match session.version {
            1 if session.business_base_url.is_none()
                && session.token_endpoint.is_none()
                && session.revocation_endpoint.is_none()
                && self.business_base == self.issuer
                && self.resource == format!("{}/api/cli", self.issuer) => {}
            2 if session.business_base_url.as_deref() == Some(&self.business_base) => {
                self.endpoint(
                    session
                        .token_endpoint
                        .as_deref()
                        .context("Missing OAuth token endpoint; log in again")?,
                )?;
                self.endpoint(
                    session
                        .revocation_endpoint
                        .as_deref()
                        .context("Missing OAuth revoke endpoint; log in again")?,
                )?;
            }
            _ => bail!(
                "Unsupported or inconsistent OAuth session configuration; upgrade or log in again"
            ),
        }
        Ok(())
    }

    fn endpoint(&self, endpoint: &str) -> Result<Url> {
        let endpoint_uri = safe_url(endpoint)?;
        if endpoint_uri.origin() != origin(&self.issuer)?.origin() {
            bail!("OAuth endpoint is outside the trusted issuer origin");
        }
        Ok(endpoint_uri)
    }

    pub async fn discover(&self) -> Result<OAuthMetadata> {
        let server: ServerMetadata = self.get("/.well-known/oauth-authorization-server").await?;
        if server.issuer != self.issuer
            || !contains(&server.code_challenge_methods_supported, "S256")
            || !contains(&server.grant_types_supported, "authorization_code")
            || !contains(&server.grant_types_supported, "refresh_token")
            || !contains(&server.response_types_supported, "code")
            || !contains(&server.token_endpoint_auth_methods_supported, "none")
            || !server.authorization_response_iss_parameter_supported
        {
            bail!("OAuth authorization-server metadata does not match the YCloud contract");
        }
        self.endpoint(&server.authorization_endpoint)?;
        self.endpoint(&server.token_endpoint)?;
        self.endpoint(&server.revocation_endpoint)?;
        let resource: ResourceMetadata = decode(
            self.http
                .get(format!(
                    "{}/.well-known/oauth-protected-resource/api/cli",
                    self.issuer
                ))
                .send()
                .await
                .map_err(|_| anyhow::anyhow!("OAuth resource discovery failed"))?,
        )
        .await?;
        if resource.resource != self.resource()
            || !contains(&resource.authorization_servers, &self.issuer)
            || !contains(&resource.bearer_methods_supported, "header")
            || resource.scopes_supported.is_empty()
            || resource.scopes_supported.len() > 100
            || resource.scopes_supported.iter().any(|s| !valid_scope(s))
        {
            bail!("OAuth CLI resource metadata does not match this issuer/resource");
        }
        Ok(OAuthMetadata {
            issuer: self.issuer.clone(),
            authorization_endpoint: server.authorization_endpoint,
            token_endpoint: server.token_endpoint,
            revocation_endpoint: server.revocation_endpoint,
            scopes: resource.scopes_supported,
        })
    }

    pub fn authorize_url(
        &self,
        metadata: &OAuthMetadata,
        scopes: &[String],
        state: &str,
        pair: &PkcePair,
        redirect: &str,
    ) -> Result<Url> {
        if metadata.issuer != self.issuer
            || scopes.is_empty()
            || scopes.iter().any(|s| !metadata.scopes.contains(s))
        {
            bail!("Requested OAuth scopes are not advertised by the CLI resource");
        }
        let mut url = self.endpoint(&metadata.authorization_endpoint)?;
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect),
            ("scope", &scopes.join(" ")),
            ("state", state),
            ("code_challenge", &pair.code_challenge),
            ("code_challenge_method", "S256"),
            ("resource", &self.resource()),
        ]);
        Ok(url)
    }

    pub async fn exchange(
        &self,
        token_endpoint: &str,
        code: &str,
        verifier: &str,
        redirect: &str,
        scopes: &[String],
    ) -> Result<OAuthToken> {
        self.token(
            token_endpoint,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", self.client_id.as_str()),
                ("resource", &self.resource()),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", redirect),
            ],
            scopes,
        )
        .await
    }

    pub async fn refresh(
        &self,
        token_endpoint: &str,
        refresh: &str,
        scopes: &[String],
    ) -> Result<OAuthToken> {
        self.token(
            token_endpoint,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &self.client_id),
                ("resource", &self.resource),
                ("refresh_token", refresh),
            ],
            scopes,
        )
        .await
    }
    pub async fn revoke(&self, revocation_endpoint: &str, refresh: &str) -> Result<()> {
        let response = self
            .http
            .post(self.endpoint(revocation_endpoint)?)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("token", refresh),
                ("token_type_hint", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|_| {
                anyhow::anyhow!("OAuth revocation request failed; remote revocation unconfirmed")
            })?;
        if response.status().as_u16() != 200 {
            bail!(
                "OAuth revocation failed (HTTP {}); remote revocation unconfirmed",
                response.status().as_u16()
            );
        }
        Ok(())
    }
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self
            .http
            .get(format!("{}{}", self.issuer, path))
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("OAuth discovery request failed"))?;
        decode(response).await
    }
    async fn token(
        &self,
        endpoint: &str,
        form: &[(&str, &str)],
        requested: &[String],
    ) -> Result<OAuthToken> {
        // One-time code exchange and refresh rotation are never automatically retried.
        let response = self
            .http
            .post(self.endpoint(endpoint)?)
            .form(form)
            .send()
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "OAuth token request failed; do not retry a potentially consumed credential"
                )
            })?;
        let token: OAuthToken = decode(response).await?;
        if !token.token_type.eq_ignore_ascii_case("Bearer")
            || !valid_credential(&token.access_token)
            || !valid_credential(&token.refresh_token)
            || token.expires_in == 0
            || token.expires_in > 3600
        {
            bail!("OAuth token response violates the token contract");
        }
        let returned: Vec<_> = token.scope.split(' ').collect();
        if returned.is_empty()
            || returned
                .iter()
                .any(|s| !valid_scope(s) || !requested.iter().any(|r| r == s))
            || returned.iter().collect::<BTreeSet<_>>().len() != returned.len()
        {
            bail!("OAuth token response contains invalid or expanded scopes");
        }
        Ok(token)
    }
}

async fn decode<T: serde::de::DeserializeOwned>(mut response: Response) -> Result<T> {
    if response.status().as_u16() != 200 {
        bail!("OAuth request failed (HTTP {})", response.status().as_u16());
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if content_type.split(';').next().map(str::trim) != Some("application/json") {
        bail!("OAuth response is not JSON");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("OAuth response read failed"))?
    {
        if bytes.len() + chunk.len() > BODY_LIMIT {
            bail!("OAuth response exceeds the size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("OAuth response has an invalid structure"))
}
fn contains(values: &[String], value: &str) -> bool {
    values.iter().any(|v| v == value)
}
fn valid_scope(value: &str) -> bool {
    value.starts_with("yc.")
        && value.len() <= 128
        && value.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_')
        })
}
fn valid_credential(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && value.bytes().all(|b| b.is_ascii_graphic())
}
fn requested_scopes(args: &LoginArgs, metadata: &OAuthMetadata) -> Result<Vec<String>> {
    let basic = ["yc.identity.current.read", "yc.tenant.list.read"];
    let initial: Vec<&str> = match args.profile {
        PermissionProfile::Basic => basic.to_vec(),
        PermissionProfile::ContactsRead => [
            basic.as_slice(),
            &["yc.contact.metadata.read", "yc.contact.record.read"],
        ]
        .concat(),
        PermissionProfile::AnalyticsRead => [
            basic.as_slice(),
            &[
                "yc.campaign.record.read",
                "yc.whatsapp.analytics.read",
                "yc.whatsapp.message.read",
                "yc.calling.log.read",
            ],
        ]
        .concat(),
        PermissionProfile::IntegrationsRead => {
            [basic.as_slice(), &["yc.integration.status.read"]].concat()
        }
        PermissionProfile::Readonly => metadata
            .scopes
            .iter()
            .filter(|s| {
                s.ends_with(".read")
                    && !matches!(
                        s.as_str(),
                        "yc.whatsapp.phone.read"
                            | "yc.inbox.phone-assignment.read"
                            | "yc.inbox.assignment-rule.read"
                    )
            })
            .map(String::as_str)
            .collect(),
        PermissionProfile::Custom => vec![],
    };
    let mut scopes: BTreeSet<String> = initial.into_iter().map(str::to_string).collect();
    scopes.extend(args.permissions.clone());
    if scopes.is_empty()
        || scopes
            .iter()
            .any(|s| !valid_scope(s) || !metadata.scopes.contains(s))
    {
        bail!("Requested OAuth profile/permissions are not supported by this CLI resource");
    }
    Ok(scopes.into_iter().collect())
}

fn login_issuer<'a>(args: &'a LoginArgs, business_base: &str) -> Result<&'a str> {
    match args.oauth_issuer.as_deref() {
        Some(issuer) => Ok(issuer),
        None if business_base == crate::cli::DEFAULT_DASHBOARD_URL => {
            Ok(crate::cli::DEFAULT_OAUTH_ISSUER)
        }
        None => bail!("Specify --oauth-issuer for a custom --dashboard-url; legacy login is not supported by this CLI"),
    }
}

pub(crate) fn require_oauth_session(config: &Config) -> Result<&OAuthSession> {
    config.auth.oauth.as_ref().context(
        "This profile uses legacy CLI authentication. Run `ycloud login` to authorize with standard OAuth; use --oauth-issuer with --dashboard-url for other environments. Existing legacy grants can be revoked in YCloud authorization management",
    )
}

pub async fn login(dashboard: &DashboardClient, config_path: &Path, args: LoginArgs) -> Result<()> {
    let business_base = dashboard.base_url();
    let issuer = login_issuer(&args, &business_base)?;
    let resource = args
        .oauth_resource
        .clone()
        .unwrap_or_else(|| format!("{business_base}/api/cli"));
    let client = OAuthClient::configured(
        issuer,
        args.oauth_client_id.as_deref().unwrap_or(CLIENT_ID),
        &resource,
        &business_base,
    )?;
    let metadata = client.discover().await?;
    let scopes = requested_scopes(&args, &metadata)?;
    let pair = pkce::generate_pkce_pair();
    let state = pkce::generate_pkce_pair().code_verifier;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to bind OAuth callback listener")?;
    let redirect = format!("http://{}/oauth/callback", listener.local_addr()?);
    let authorize = client.authorize_url(&metadata, &scopes, &state, &pair, &redirect)?;
    println!("Opening browser for YCloud OAuth authorization:\n{authorize}");
    if !args.no_browser && crate::auth::open_browser(authorize.as_str()).is_err() {
        println!("Open the URL above in a YCloud browser.");
    }
    let code = tokio::time::timeout(
        Duration::from_secs(300),
        wait_callback(listener, &state, &client.issuer),
    )
    .await
    .map_err(|_| anyhow::anyhow!("OAuth browser callback timed out"))??;
    let token = client
        .exchange(
            &metadata.token_endpoint,
            &code,
            &pair.code_verifier,
            &redirect,
            &scopes,
        )
        .await?;
    let granted = token.scope.split(' ').map(str::to_string).collect();
    let mut config = Config {
        dashboard: DashboardConfig {
            base_url: client.business_base.clone(),
        },
        auth: AuthConfig {
            token_type: token.token_type,
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            record_id: String::new(),
            requested_permissions: granted,
            tenant_id: None,
            user_id: None,
            oauth: Some(OAuthSession {
                version: 2,
                issuer: client.issuer.clone(),
                client_id: client.client_id.clone(),
                resource: client.resource(),
                business_base_url: Some(client.business_base.clone()),
                token_endpoint: Some(metadata.token_endpoint),
                revocation_endpoint: Some(metadata.revocation_endpoint),
                refresh_uncertain: false,
                access_expires_at: now()? + token.expires_in,
            }),
        },
    };
    let business = dashboard.clone().bind_oauth(&client.resource)?;
    if let Ok(identity) = business
        .whoami(&config.auth.access_token)
        .await
        .and_then(|r| r.require_data("whoami"))
    {
        config.auth.tenant_id = Some(identity.tenant_id);
        config.auth.user_id = Some(identity.user_id);
    }
    config.save(config_path)?;
    println!(
        "OAuth login succeeded. Profile saved at {}.",
        config_path.display()
    );
    Ok(())
}

/// Upgrade only the original same-origin trial binding, using trusted discovery before credentials.
pub async fn prepare_session(
    config: &mut Config,
    config_path: &Path,
    business_base: &str,
) -> Result<OAuthClient> {
    *config = Config::load(config_path)?;
    upgrade_trial_session(config, config_path, business_base).await
}

async fn upgrade_trial_session(
    config: &mut Config,
    config_path: &Path,
    business_base: &str,
) -> Result<OAuthClient> {
    let client = validate_session(config, business_base)?;
    if config.auth.oauth.as_ref().is_some_and(|s| s.version == 1) {
        let metadata = client.discover().await?;
        let session = config
            .auth
            .oauth
            .as_mut()
            .context("missing OAuth session")?;
        session.version = 2;
        session.business_base_url = Some(business_base.to_string());
        session.token_endpoint = Some(metadata.token_endpoint);
        session.revocation_endpoint = Some(metadata.revocation_endpoint);
        config.save(config_path)?;
    }
    Ok(client)
}

/// Validate a complete profile snapshot without locking, discovery or writes.
/// Old trial bindings are upgraded only by the credential-writing paths.
pub(crate) fn validate_session(config: &Config, business_base: &str) -> Result<OAuthClient> {
    let session = require_oauth_session(config)?;
    if business_base != config.dashboard.base_url {
        bail!("OAuth business address override changed; log in again");
    }
    let client = OAuthClient::configured(
        &session.issuer,
        &session.client_id,
        &session.resource,
        business_base,
    )?;
    client.require_session(session)?;
    Ok(client)
}

pub async fn refresh(
    dashboard: &DashboardClient,
    config_path: &Path,
    _initial_config: Config,
) -> Result<()> {
    let mut config = Config::load(config_path)?;
    let client = upgrade_trial_session(&mut config, config_path, &dashboard.base_url()).await?;
    let session = require_oauth_session(&config)?;
    if session.refresh_uncertain || config.auth.refresh_token.is_empty() {
        bail!("Previous OAuth refresh was not confirmed; log in again instead of reusing the refresh token");
    }
    let endpoint = session
        .token_endpoint
        .clone()
        .context("missing token endpoint")?;
    // Persist before the one-time operation. A crash or unknown result must not replay the old token.
    config
        .auth
        .oauth
        .as_mut()
        .context("missing OAuth session")?
        .refresh_uncertain = true;
    config.save(config_path)?;
    let token = client
        .refresh(
            &endpoint,
            &config.auth.refresh_token,
            &config.auth.requested_permissions,
        )
        .await?;
    if token.refresh_token == config.auth.refresh_token {
        bail!("OAuth server did not rotate the refresh token; log in again");
    }
    let expires_at = now()? + token.expires_in;
    config.auth.token_type = token.token_type;
    config.auth.access_token = token.access_token;
    config.auth.refresh_token = token.refresh_token;
    config.auth.requested_permissions = token.scope.split(' ').map(str::to_string).collect();
    let session = config
        .auth
        .oauth
        .as_mut()
        .context("missing OAuth session")?;
    session.access_expires_at = expires_at;
    session.refresh_uncertain = false;
    config.save(config_path)?;
    println!("OAuth token refreshed.");
    Ok(())
}

pub async fn logout(
    dashboard: &DashboardClient,
    config_path: &Path,
    _initial_config: Config,
) -> Result<()> {
    let mut config = Config::load(config_path)?;
    require_oauth_session(&config)?;
    let result = async {
        let client = upgrade_trial_session(&mut config, config_path, &dashboard.base_url()).await?;
        let endpoint = config
            .auth
            .oauth
            .as_ref()
            .and_then(|s| s.revocation_endpoint.as_deref())
            .context("missing revocation endpoint")?;
        client.revoke(endpoint, &config.auth.refresh_token).await
    }
    .await;
    crate::config::remove(config_path)?;
    if result.is_err() {
        bail!("Local OAuth profile removed; remote revocation was not confirmed. Revoke the connection in YCloud authorization management.");
    }
    println!("Logged out. OAuth grant revoked and local profile removed.");
    Ok(())
}

fn safe_url(value: &str) -> Result<Url> {
    let uri = Url::parse(value).map_err(|_| anyhow::anyhow!("Invalid OAuth URL"))?;
    let explicit_loopback = std::env::var("YCLOUD_OAUTH_TEST_LOOPBACK").as_deref() == Ok("1")
        && uri.scheme() == "http"
        && uri.host_str() == Some("127.0.0.1");
    if (uri.scheme() != "https" && !explicit_loopback)
        || uri.host_str().is_none()
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.fragment().is_some()
        || value.len() > 2048
        || value
            .chars()
            .any(|c| c.is_ascii_control() || c.is_whitespace())
    {
        bail!("OAuth URLs require HTTPS without userinfo or fragments; loopback HTTP needs the explicit test switch");
    }
    Ok(uri)
}
fn origin(value: &str) -> Result<Url> {
    let uri = safe_url(value)?;
    if uri.path() != "/" || uri.query().is_some() {
        bail!("OAuth base URL must be an origin");
    }
    Ok(uri)
}

fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock precedes Unix epoch")?
        .as_secs())
}

// Valid callbacks bind both the request state and the authorization-server issuer.
fn callback(target: &str, state: &str, issuer: &str) -> Result<Option<String>> {
    if !target.starts_with("/oauth/callback?") || target.len() > 8192 || target.contains('#') {
        bail!("invalid OAuth callback path");
    }
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| anyhow::anyhow!("invalid OAuth callback"))?;
    let mut fields = HashMap::new();
    for (key, value) in url.query_pairs() {
        if fields
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            bail!("duplicate OAuth callback parameter");
        }
    }
    if fields.get("state").map(String::as_str) != Some(state)
        || fields.get("iss").map(String::as_str) != Some(issuer)
    {
        bail!("OAuth callback state or issuer mismatch");
    }
    match (fields.get("code"), fields.get("error")) {
        (Some(code), None) if valid_credential(code) => Ok(Some(code.clone())),
        (None, Some(_)) => Ok(None),
        _ => bail!("invalid OAuth callback result"),
    }
}

async fn wait_callback(listener: TcpListener, state: &str, issuer: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .context("OAuth callback accept failed")?;
        let read = tokio::time::timeout(Duration::from_secs(2), async {
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let n = stream.read(&mut buffer).await?;
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..n]);
                if bytes.len() > 8192 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "callback too large",
                    ));
                }
                if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Ok::<_, std::io::Error>(bytes)
        })
        .await;
        let outcome = match read {
            Ok(Ok(bytes)) => {
                let request = String::from_utf8(bytes).unwrap_or_default();
                let parts: Vec<_> = request
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .collect();
                if parts.len() == 3
                    && parts[0] == "GET"
                    && matches!(parts[2], "HTTP/1.1" | "HTTP/1.0")
                {
                    callback(parts[1], state, issuer)
                } else {
                    Err(anyhow::anyhow!("invalid OAuth callback request"))
                }
            }
            _ => Err(anyhow::anyhow!("invalid OAuth callback request")),
        };
        let page_state = match &outcome {
            Ok(Some(_)) => crate::oauth_callback::PageState::Received,
            Ok(None) => crate::oauth_callback::PageState::Denied,
            Err(_) => crate::oauth_callback::PageState::Rejected,
        };
        let response = crate::oauth_callback::response(page_state);
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            stream.write_all(response.as_bytes()),
        )
        .await;
        match outcome {
            Ok(Some(code)) => return Ok(code),
            Ok(None) => bail!("OAuth authorization was denied"),
            Err(_) => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_login_uses_the_open_platform_issuer_and_custom_origins_require_a_choice() {
        use clap::Parser;
        for flags in [vec![], vec!["--oauth"]] {
            let mut argv = vec!["ycloud", "login"];
            argv.extend(flags);
            let cli = crate::cli::Cli::try_parse_from(argv).unwrap();
            let crate::cli::Command::Login(args) = cli.command else {
                panic!("expected login");
            };
            assert_eq!(
                login_issuer(&args, "https://www.ycloud.com").unwrap(),
                "https://open-platform.ycloud.com"
            );
            assert!(login_issuer(&args, "https://www-prepare.ycloud.com").is_err());
        }
    }

    #[test]
    fn callback_requires_exact_state_issuer_and_unique_parameters() {
        let issuer = "https://www.ycloud.com";
        let good = "/oauth/callback?code=fixture&state=s&iss=https%3A%2F%2Fwww.ycloud.com";
        assert_eq!(
            callback(good, "s", issuer).unwrap().as_deref(),
            Some("fixture")
        );
        for target in [
            "/callback?code=fixture&state=s&iss=https%3A%2F%2Fwww.ycloud.com",
            "/oauth/callback?code=fixture&state=s",
            "/oauth/callback?code=fixture&state=s&iss=https%3A%2F%2Fattacker.example",
            "/oauth/callback?code=a&code=b&state=s&iss=https%3A%2F%2Fwww.ycloud.com",
            "/oauth/callback?code=a&error=denied&state=s&iss=https%3A%2F%2Fwww.ycloud.com",
        ] {
            assert!(callback(target, "s", issuer).is_err());
        }
        assert!(callback(good, "other", issuer).is_err());
    }
}
