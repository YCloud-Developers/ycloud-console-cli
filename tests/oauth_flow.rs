use serde_json::json;
use wiremock::{
    matchers::{body_string_contains, header, method, path},
    Mock, MockServer, ResponseTemplate,
};
use ycloud_console_cli::{oauth::OAuthClient, pkce};

fn local_client(server: &MockServer) -> anyhow::Result<OAuthClient> {
    std::env::set_var("YCLOUD_OAUTH_TEST_LOOPBACK", "1");
    OAuthClient::new(&server.uri())
}

async fn discovery(server: &MockServer) {
    discovery_for_resource(server, &format!("{}/api/cli", server.uri())).await;
}

async fn discovery_for_resource(server: &MockServer, resource: &str) {
    std::env::set_var("YCLOUD_OAUTH_TEST_LOOPBACK", "1");
    let issuer = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/api/oauth/v1/authorize"),
            "token_endpoint": format!("{issuer}/api/oauth/v1/token"),
            "revocation_endpoint": format!("{issuer}/api/oauth/v1/revoke"),
            "code_challenge_methods_supported": ["S256"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "response_types_supported": ["code"],
            "token_endpoint_auth_methods_supported": ["none"],
            "authorization_response_iss_parameter_supported": true
        })))
        .mount(server)
        .await;
    Mock::given(method("GET")).and(path("/.well-known/oauth-protected-resource/api/cli"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource":resource, "authorization_servers":[issuer],
            "scopes_supported":["yc.identity.current.read", "yc.tenant.list.read", "yc.contact.metadata.read"],
            "bearer_methods_supported":["header"]
        }))).mount(server).await;
}

#[tokio::test]
async fn separate_issuer_publishes_web_resource_without_business_discovery() {
    let issuer = MockServer::start().await;
    let business = MockServer::start().await;
    let resource = format!("{}/api/cli", business.uri());
    discovery_for_resource(&issuer, &resource).await;
    let client = OAuthClient::configured(
        &issuer.uri(),
        "ycloud-console-cli",
        &resource,
        &business.uri(),
    )
    .unwrap();
    let metadata = client.discover().await.unwrap();
    let pair = pkce::generate_pkce_pair();
    let target = client
        .authorize_url(
            &metadata,
            &["yc.identity.current.read".to_string()],
            "state-fixture",
            &pair,
            "http://127.0.0.1:12345/oauth/callback",
        )
        .unwrap();
    assert_eq!(
        target.origin(),
        url::Url::parse(&issuer.uri()).unwrap().origin()
    );
    assert!(target
        .query_pairs()
        .any(|(k, v)| k == "resource" && v == resource));
    assert!(business.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn discovery_and_authorize_bind_client_resource_and_pkce() {
    let server = MockServer::start().await;
    discovery(&server).await;
    let client = local_client(&server).unwrap();
    let metadata = client.discover().await.unwrap();
    let pair = pkce::generate_pkce_pair();
    let scopes = vec!["yc.identity.current.read".to_string()];
    let url = client
        .authorize_url(
            &metadata,
            &scopes,
            "state-fixture",
            &pair,
            "http://127.0.0.1:12345/oauth/callback",
        )
        .unwrap();
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], "ycloud-console-cli");
    assert_eq!(query["resource"], format!("{}/api/cli", server.uri()));
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["code_challenge"], pair.code_challenge);
    assert!(!query.contains_key("profile"));
    assert!(!query.contains_key("code_verifier"));
}

#[tokio::test]
async fn exchange_uses_form_and_exact_redirect_without_secret_or_retry() {
    let server = MockServer::start().await;
    discovery(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/oauth/v1/token"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("client_id=ycloud-console-cli"))
        .and(body_string_contains(
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A12345%2Foauth%2Fcallback",
        ))
        .and(body_string_contains("code_verifier=verifier-fixture"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token":"opaque-fixture", "refresh_token":"refresh-fixture",
            "token_type":"Bearer", "expires_in":3600, "scope":"yc.identity.current.read"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let client = local_client(&server).unwrap();
    let token = client
        .exchange(
            &client.discover().await.unwrap().token_endpoint,
            "code-fixture",
            "verifier-fixture",
            "http://127.0.0.1:12345/oauth/callback",
            &["yc.identity.current.read".into()],
        )
        .await
        .unwrap();
    assert_eq!(token.access_token, "opaque-fixture");
    let requests = server.received_requests().await.unwrap();
    let post = requests
        .iter()
        .find(|r| r.method.as_str() == "POST")
        .unwrap();
    assert!(!post.headers.contains_key("authorization"));
    assert!(!String::from_utf8_lossy(&post.body).contains("client_secret"));
}

#[tokio::test]
async fn failed_rotation_is_not_retried_and_does_not_echo_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth/v1/token"))
        .respond_with(ResponseTemplate::new(503).set_body_string("sensitive-refresh-fixture"))
        .expect(1)
        .mount(&server)
        .await;
    let client = local_client(&server).unwrap();
    let error = client
        .refresh(
            &format!("{}/api/oauth/v1/token", server.uri()),
            "sensitive-refresh-fixture",
            &["yc.identity.current.read".into()],
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("sensitive-refresh-fixture"));
    assert!(error.contains("503"));
}

#[tokio::test]
async fn foreign_discovery_endpoint_is_rejected_before_any_credential_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer":server.uri(),
            "authorization_endpoint":format!("{}/api/oauth/v1/authorize",server.uri()),
            "token_endpoint":"https://attacker.example/token",
            "revocation_endpoint":format!("{}/api/oauth/v1/revoke",server.uri()),
            "code_challenge_methods_supported":["S256"],
            "grant_types_supported":["authorization_code","refresh_token"],
            "response_types_supported":["code"],
            "token_endpoint_auth_methods_supported":["none"],
            "authorization_response_iss_parameter_supported":true
        })))
        .mount(&server)
        .await;
    assert!(local_client(&server).unwrap().discover().await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn oauth_discovery_does_not_follow_redirects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/unexpected"))
        .mount(&server)
        .await;
    Mock::given(path("/unexpected"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    assert!(local_client(&server).unwrap().discover().await.is_err());
}

#[tokio::test]
async fn successful_response_cannot_expand_requested_scopes() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/oauth/v1/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"opaque","refresh_token":"rotated","token_type":"Bearer","expires_in":3600,"scope":"yc.contact.record.export"})))
        .mount(&server).await;
    assert!(local_client(&server)
        .unwrap()
        .refresh(
            &format!("{}/api/oauth/v1/token", server.uri()),
            "fixture",
            &["yc.identity.current.read".into()]
        )
        .await
        .is_err());
}

#[test]
fn remote_http_and_ambiguous_issuers_are_rejected() {
    for issuer in [
        "http://www.ycloud.com",
        "https://user@www.ycloud.com",
        "https://www.ycloud.com/path",
        "https://www.ycloud.com/?query=value",
        "https://www.ycloud.com/#fragment",
    ] {
        assert!(OAuthClient::new(issuer).is_err());
    }
}

fn saved_profile(
    server: &MockServer,
    path: &std::path::Path,
) -> ycloud_console_cli::config::Config {
    use ycloud_console_cli::config::{AuthConfig, Config, DashboardConfig, OAuthSession};
    std::env::set_var("YCLOUD_OAUTH_TEST_LOOPBACK", "1");
    let config = Config {
        dashboard: DashboardConfig {
            base_url: server.uri(),
        },
        auth: AuthConfig {
            oauth: Some(OAuthSession {
                version: 2,
                business_base_url: Some(server.uri()),
                token_endpoint: Some(format!("{}/api/oauth/v1/token", server.uri())),
                revocation_endpoint: Some(format!("{}/api/oauth/v1/revoke", server.uri())),
                refresh_uncertain: false,
                issuer: server.uri(),
                client_id: "ycloud-console-cli".into(),
                resource: format!("{}/api/cli", server.uri()),
                access_expires_at: 1,
            }),
            token_type: "Bearer".into(),
            access_token: "old-access-fixture".into(),
            refresh_token: "old-refresh-fixture".into(),
            record_id: String::new(),
            requested_permissions: vec![
                "yc.identity.current.read".into(),
                "yc.tenant.list.read".into(),
            ],
            tenant_id: None,
            user_id: None,
        },
    };
    config.save(path).unwrap();
    config
}

#[tokio::test]
async fn rotation_saves_complete_pair_and_narrowed_scopes_atomically() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    saved_profile(&server, &location);
    Mock::given(method("POST")).and(path("/api/oauth/v1/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=old-refresh-fixture"))
        .and(body_string_contains("resource="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"new-access-fixture","refresh_token":"new-refresh-fixture","token_type":"Bearer","expires_in":3600,"scope":"yc.identity.current.read"})))
        .expect(1).mount(&server).await;
    let client = ycloud_console_cli::http::DashboardClient::new(server.uri()).unwrap();
    ycloud_console_cli::auth::refresh(&client, &location)
        .await
        .unwrap();
    let saved = ycloud_console_cli::config::Config::load(&location).unwrap();
    assert_eq!(saved.auth.access_token, "new-access-fixture");
    assert_eq!(saved.auth.refresh_token, "new-refresh-fixture");
    assert_eq!(
        saved.auth.requested_permissions,
        ["yc.identity.current.read"]
    );
    assert!(saved.auth.oauth.unwrap().access_expires_at > 3600);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&location).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn failed_refresh_blocks_reuse_without_legacy_fallback() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    let original = saved_profile(&server, &location);
    Mock::given(method("POST"))
        .and(path("/api/oauth/v1/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":"invalid_grant"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/cli/auth/refresh"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = ycloud_console_cli::http::DashboardClient::new(server.uri()).unwrap();
    assert!(ycloud_console_cli::auth::refresh(&client, &location)
        .await
        .is_err());
    let saved = ycloud_console_cli::config::Config::load(&location).unwrap();
    assert!(saved.auth.oauth.as_ref().unwrap().refresh_uncertain);
    assert_eq!(saved.auth.refresh_token, original.auth.refresh_token);
    assert!(ycloud_console_cli::auth::refresh(&client, &location)
        .await
        .is_err());
}

#[tokio::test]
async fn logout_cleans_local_profile_and_distinguishes_unconfirmed_remote_revocation() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    saved_profile(&server, &location);
    let client = ycloud_console_cli::http::DashboardClient::new(server.uri()).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/oauth/v1/revoke"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    assert!(ycloud_console_cli::auth::logout(&client, &location)
        .await
        .is_err());
    assert!(!location.exists());
    server.reset().await;
    saved_profile(&server, &location);
    Mock::given(method("POST"))
        .and(path("/api/oauth/v1/revoke"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(body_string_contains("token=old-refresh-fixture"))
        .and(body_string_contains("client_id=ycloud-console-cli"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    ycloud_console_cli::auth::logout(&client, &location)
        .await
        .unwrap();
    assert!(!location.exists());
}

#[test]
fn pre_oauth_config_without_protocol_marker_stays_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    std::fs::write(&location,"[dashboard]\nbase_url = 'https://www.ycloud.com'\n[auth]\ntoken_type = 'Bearer'\naccess_token = 'YCLI.fixture'\nrefresh_token = 'YCLI.refresh-fixture'\nrecord_id = 'legacy-fixture'\nrequested_permissions = ['yc.identity.current.read']\n").unwrap();
    let config = ycloud_console_cli::config::Config::load(&location).unwrap();
    assert!(config.auth.oauth.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_cli_callback_flow_checks_issuer_state_and_persists_standard_session() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    let server = MockServer::start().await;
    discovery(&server).await;
    Mock::given(method("POST")).and(path("/api/oauth/v1/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"opaque-login-fixture","refresh_token":"refresh-login-fixture","token_type":"Bearer","expires_in":3600,"scope":"yc.identity.current.read yc.tenant.list.read"})))
        .expect(1).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/cli/v1/whoami"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ycloud"))
        .args([
            "--dashboard-url",
            &server.uri(),
            "--config",
            location.to_str().unwrap(),
            "login",
            "--oauth",
            "--no-browser",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.starts_with("http://") {
                let _ = send.send(line);
            }
        }
    });
    let auth_url =
        tokio::task::spawn_blocking(move || receive.recv_timeout(Duration::from_secs(10)))
            .await
            .unwrap()
            .unwrap();
    let authorize = url::Url::parse(&auth_url).unwrap();
    let query: std::collections::HashMap<_, _> = authorize.query_pairs().into_owned().collect();
    let mut callback = url::Url::parse(&query["redirect_uri"]).unwrap();
    callback.query_pairs_mut().extend_pairs([
        ("code", "code-login-fixture"),
        ("state", query["state"].as_str()),
        ("iss", "https://attacker.example"),
    ]);
    assert_eq!(
        reqwest::get(callback.as_str())
            .await
            .unwrap()
            .status()
            .as_u16(),
        400
    );
    callback.set_query(None);
    callback.query_pairs_mut().extend_pairs([
        ("code", "code-login-fixture"),
        ("state", query["state"].as_str()),
        ("iss", server.uri().as_str()),
    ]);
    assert_eq!(
        reqwest::get(callback.as_str())
            .await
            .unwrap()
            .status()
            .as_u16(),
        200
    );
    let status = tokio::task::spawn_blocking(move || {
        for _ in 0..100 {
            if let Some(s) = child.try_wait().unwrap() {
                return s;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        child.kill().unwrap();
        child.wait().unwrap()
    })
    .await
    .unwrap();
    reader.join().unwrap();
    assert!(status.success());
    let config = ycloud_console_cli::config::Config::load(&location).unwrap();
    assert_eq!(config.auth.oauth.unwrap().issuer, server.uri());
    let requests = server.received_requests().await.unwrap();
    let exchange = requests
        .iter()
        .find(|r| r.url.path() == "/api/oauth/v1/token")
        .unwrap();
    let form: std::collections::HashMap<_, _> = url::form_urlencoded::parse(&exchange.body)
        .into_owned()
        .collect();
    assert_eq!(form["redirect_uri"], query["redirect_uri"]);
    assert_eq!(
        pkce::challenge_for_verifier(&form["code_verifier"]).code_challenge,
        query["code_challenge"]
    );
}

#[tokio::test]
async fn foreign_dashboard_override_cannot_receive_a_saved_oauth_credential() {
    let original = MockServer::start().await;
    let foreign = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    saved_profile(&original, &location);
    let config_path = location.clone();
    let foreign_url = foreign.uri();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_ycloud"))
            .args([
                "--dashboard-url",
                &foreign_url,
                "--config",
                config_path.to_str().unwrap(),
                "whoami",
            ])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(!output.status.success());
    assert!(foreign.received_requests().await.unwrap().is_empty());
    assert!(original.received_requests().await.unwrap().is_empty());
    assert!(location.exists());
}

#[tokio::test]
async fn discovered_endpoint_paths_are_used_instead_of_fixed_paths() {
    let server = MockServer::start().await;
    std::env::set_var("YCLOUD_OAUTH_TEST_LOOPBACK", "1");
    let issuer = server.uri();
    Mock::given(path("/.well-known/oauth-authorization-server")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "issuer":issuer,"authorization_endpoint":format!("{issuer}/custom/authorize"),
        "token_endpoint":format!("{issuer}/custom/token"),"revocation_endpoint":format!("{issuer}/custom/revoke"),
        "code_challenge_methods_supported":["S256"],"grant_types_supported":["authorization_code","refresh_token"],
        "response_types_supported":["code"],"token_endpoint_auth_methods_supported":["none"],"authorization_response_iss_parameter_supported":true
    }))).mount(&server).await;
    Mock::given(path("/.well-known/oauth-protected-resource/api/cli")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "resource":format!("{issuer}/api/cli"),"authorization_servers":[issuer],"scopes_supported":["yc.identity.current.read"],"bearer_methods_supported":["header"]
    }))).mount(&server).await;
    Mock::given(method("POST")).and(path("/custom/token")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "access_token":"opaque","refresh_token":"rotated","token_type":"Bearer","expires_in":3600,"scope":"yc.identity.current.read"
    }))).expect(1).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/custom/revoke"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let client = local_client(&server).unwrap();
    let metadata = client.discover().await.unwrap();
    let authorize = client
        .authorize_url(
            &metadata,
            &["yc.identity.current.read".into()],
            "state",
            &pkce::generate_pkce_pair(),
            "http://127.0.0.1:12345/oauth/callback",
        )
        .unwrap();
    assert_eq!(authorize.path(), "/custom/authorize");
    client
        .refresh(
            &metadata.token_endpoint,
            "fixture",
            &["yc.identity.current.read".into()],
        )
        .await
        .unwrap();
    client
        .revoke(&metadata.revocation_endpoint, "fixture")
        .await
        .unwrap();
}

#[tokio::test]
async fn old_trial_binding_is_upgraded_only_after_trusted_discovery() {
    let server = MockServer::start().await;
    discovery(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    let mut config = saved_profile(&server, &location);
    let session = config.auth.oauth.as_mut().unwrap();
    session.version = 1;
    session.business_base_url = None;
    session.token_endpoint = None;
    session.revocation_endpoint = None;
    config.save(&location).unwrap();
    let old_refresh = config.auth.refresh_token.clone();
    ycloud_console_cli::oauth::prepare_session(&mut config, &location, &server.uri())
        .await
        .unwrap();
    assert_eq!(config.auth.refresh_token, old_refresh);
    assert_eq!(config.auth.oauth.as_ref().unwrap().version, 2);
    assert!(config.auth.oauth.as_ref().unwrap().token_endpoint.is_some());
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() == "GET"));
}

#[tokio::test]
async fn unsafe_bindings_and_security_field_duplicates_fail_without_sending_secrets() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    let config = saved_profile(&server, &location);
    let client = local_client(&server).unwrap();
    for field in [
        "issuer", "client", "resource", "version", "endpoint", "base",
    ] {
        let mut changed = config.auth.oauth.clone().unwrap();
        match field {
            "issuer" => changed.issuer = "https://other.example".into(),
            "client" => changed.client_id = "other".into(),
            "resource" => changed.resource = format!("{}/other", server.uri()),
            "version" => changed.version = 99,
            "endpoint" => changed.token_endpoint = Some("https://other.example/token".into()),
            _ => changed.business_base_url = Some(format!("{}/unrelated", server.uri())),
        }
        assert!(client.require_session(&changed).is_err());
    }
    Mock::given(path("/.well-known/oauth-authorization-server"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(format!(
                    r#"{{"issuer":"{}","issuer":"https://other.example"}}"#,
                    server.uri()
                )),
        )
        .mount(&server)
        .await;
    assert!(client.discover().await.is_err());
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() == "GET"));
}

#[tokio::test]
async fn saved_business_binding_rejects_unrelated_paths_and_does_not_follow_redirects() {
    let server = MockServer::start().await;
    let other = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    saved_profile(&server, &location);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ycloud"))
        .env("YCLOUD_OAUTH_TEST_LOOPBACK", "1")
        .args([
            "--config",
            location.to_str().unwrap(),
            "--dashboard-url",
            &format!("{}/unrelated", server.uri()),
            "whoami",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(server.received_requests().await.unwrap().is_empty());
    Mock::given(path("/api/cli/v1/whoami"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/steal", other.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = ycloud_console_cli::http::DashboardClient::new(server.uri())
        .unwrap()
        .bind_oauth(&format!("{}/api/cli", server.uri()))
        .unwrap();
    assert!(client.whoami("opaque-fixture").await.is_err());
    assert!(other.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn loopback_http_requires_an_explicit_process_test_switch() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    saved_profile(&server, &location);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ycloud"))
        .env_remove("YCLOUD_OAUTH_TEST_LOOPBACK")
        .args(["--config", location.to_str().unwrap(), "whoami"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn cli_logout_cleans_an_old_trial_profile_when_discovery_is_unavailable() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    let mut config = saved_profile(&server, &location);
    let session = config.auth.oauth.as_mut().unwrap();
    session.version = 1;
    session.business_base_url = None;
    session.token_endpoint = None;
    session.revocation_endpoint = None;
    config.save(&location).unwrap();
    Mock::given(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let location_arg = location.to_str().unwrap().to_string();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_ycloud"))
            .env("YCLOUD_OAUTH_TEST_LOOPBACK", "1")
            .args(["--config", &location_arg, "logout"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(!output.status.success());
    assert!(!location.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("remote revocation was not confirmed"));
}
