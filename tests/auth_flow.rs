use wiremock::{
    matchers::{body_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};
use yc_cli::{config::Config, http::DashboardClient};

#[tokio::test]
async fn token_exchange_uses_backend_contract_and_saves_config_shape() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/auth/token"))
        .and(body_json(serde_json::json!({
            "grantType": "authorization_code",
            "code": "code-1",
            "codeVerifier": "verifier-1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "tokenType": "Bearer",
                "accessToken": "YCLI.access",
                "refreshToken": "YCLI.refresh",
                "recordId": "record-1"
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/cli/auth/whoami"))
        .and(header("authorization", "Bearer YCLI.access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "userId": "user-1",
                "tenantId": "tenant-1",
                "permissions": ["developers"]
            }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let client = DashboardClient::new(server.uri()).unwrap();
    yc_cli::auth::login(
        &client,
        &config_path,
        yc_cli::cli::LoginArgs {
            scope: "developers".to_string(),
            code: Some("code-1".to_string()),
            code_verifier: Some("verifier-1".to_string()),
            state: Some("state-1".to_string()),
        },
    )
    .await
    .unwrap();

    let config = Config::load(&config_path).unwrap();
    assert_eq!(config.dashboard.base_url, server.uri());
    assert_eq!(config.auth.access_token, "YCLI.access");
    assert_eq!(config.auth.refresh_token, "YCLI.refresh");
    assert_eq!(config.auth.record_id, "record-1");
    assert_eq!(config.auth.scope, "developers");
    assert_eq!(config.auth.tenant_id.as_deref(), Some("tenant-1"));
    assert_eq!(config.auth.user_id.as_deref(), Some("user-1"));
}

#[tokio::test]
async fn refresh_rotates_stored_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/auth/refresh"))
        .and(body_json(serde_json::json!({
            "grantType": "refresh_token",
            "refreshToken": "YCLI.old-refresh"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "tokenType": "Bearer",
                "accessToken": "YCLI.new-access",
                "refreshToken": "YCLI.new-refresh",
                "recordId": "record-2"
            }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let initial = yc_cli::config::Config {
        dashboard: yc_cli::config::DashboardConfig {
            base_url: server.uri(),
        },
        auth: yc_cli::config::AuthConfig {
            token_type: "Bearer".to_string(),
            access_token: "YCLI.old-access".to_string(),
            refresh_token: "YCLI.old-refresh".to_string(),
            record_id: "record-1".to_string(),
            scope: "developers".to_string(),
            tenant_id: None,
            user_id: None,
        },
    };
    initial.save(&config_path).unwrap();

    let client = DashboardClient::new(server.uri()).unwrap();
    yc_cli::auth::refresh(&client, &config_path).await.unwrap();

    let updated = Config::load(&config_path).unwrap();
    assert_eq!(updated.auth.access_token, "YCLI.new-access");
    assert_eq!(updated.auth.refresh_token, "YCLI.new-refresh");
    assert_eq!(updated.auth.record_id, "record-2");
}
