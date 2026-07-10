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
            manual: false,
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

#[tokio::test]
async fn contacts_list_calls_attila_web_business_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/contacts/search"))
        .and(header("authorization", "Bearer YCLI.access"))
        .and(body_json(serde_json::json!({
            "pageNo": 1,
            "pageSize": 10
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "pageNo": 1,
                "pageSize": 10,
                "totalCount": 1,
                "records": [
                    {
                        "contactId": 1,
                        "nickName": "test contact"
                    }
                ]
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
            access_token: "YCLI.access".to_string(),
            refresh_token: "YCLI.refresh".to_string(),
            record_id: "1272676752573050880".to_string(),
            scope: "developers".to_string(),
            tenant_id: Some("tenant-1".to_string()),
            user_id: Some("user-1".to_string()),
        },
    };
    initial.save(&config_path).unwrap();

    let client = DashboardClient::new(server.uri()).unwrap();
    yc_cli::auth::contacts_list(
        &client,
        &config_path,
        yc_cli::cli::ContactsListArgs {
            page_no: 1,
            page_size: 10,
            condition: None,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn analytics_overview_calls_dashboard_page_endpoints() {
    let server = MockServer::start().await;
    let expected_body = serde_json::json!({
        "startTime": 1782921600000_i64,
        "endTime": 1783526400000_i64,
        "timezone": "GMT+8",
        "from": "8613800138000",
        "regionCode": "CN",
        "messageCategory": "marketing,utility"
    });
    for endpoint in [
        "/api/whatsapp/analytics/deliveryAnalytics",
        "/api/whatsapp/analytics/messageDetail",
        "/api/whatsapp/analytics/failureReasonShare",
    ] {
        Mock::given(method("POST"))
            .and(path(endpoint))
            .and(header("authorization", "Bearer YCLI.access"))
            .and(body_json(expected_body.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "data": {
                    "endpoint": endpoint
                }
            })))
            .mount(&server)
            .await;
    }

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    saved_config(server.uri()).save(&config_path).unwrap();

    let client = DashboardClient::new(server.uri()).unwrap();
    yc_cli::auth::analytics_overview(
        &client,
        &config_path,
        yc_cli::cli::AnalyticsOverviewArgs {
            range: yc_cli::cli::AnalyticsRangeArgs {
                start_time: Some(1782921600000),
                end_time: Some(1783526400000),
            },
            timezone: "GMT+8".to_string(),
            from: Some("8613800138000".to_string()),
            region_code: Some("CN".to_string()),
            message_category: Some("marketing,utility".to_string()),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn analytics_logs_calls_dashboard_message_search_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/whatsapp/message/search"))
        .and(header("authorization", "Bearer YCLI.access"))
        .and(body_json(serde_json::json!({
            "direction": "OutBound",
            "startTime": 1782921600000_i64,
            "endTime": 1783526400000_i64,
            "pageNo": 1,
            "pageSize": 20,
            "timezone": "GMT+8",
            "condition": "test",
            "businessPhones": ["8613800138000"],
            "toRegionCodes": ["CN"],
            "status": "sent,delivered",
            "smb": false,
            "pricingCategory": ["marketing"]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "records": [],
                "pagin": {
                    "totalCount": 0
                }
            }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    saved_config(server.uri()).save(&config_path).unwrap();

    let client = DashboardClient::new(server.uri()).unwrap();
    yc_cli::auth::analytics_logs(
        &client,
        &config_path,
        yc_cli::cli::AnalyticsLogsArgs {
            range: yc_cli::cli::AnalyticsRangeArgs {
                start_time: Some(1782921600000),
                end_time: Some(1783526400000),
            },
            page_no: 1,
            page_size: 20,
            condition: Some("test".to_string()),
            direction: "OutBound".to_string(),
            from: None,
            business_phones: vec!["8613800138000".to_string()],
            to_region_codes: vec!["CN".to_string()],
            status: Some("sent,delivered".to_string()),
            source: Some("WhatsApp Business API".to_string()),
            pricing_category: vec!["marketing".to_string()],
            timezone: "GMT+8".to_string(),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn analytics_calling_logs_calls_dashboard_calling_search_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/calling/logs/search"))
        .and(header("authorization", "Bearer YCLI.access"))
        .and(body_json(serde_json::json!({
            "startTime": 1782921600000_i64,
            "endTime": 1783526400000_i64,
            "pageNo": 1,
            "pageSize": 20,
            "condition": "test",
            "directions": ["BUSINESS_INITIATED"],
            "regionCodes": ["CN"],
            "sources": ["CALLING"],
            "status": ["COMPLETED"],
            "phoneNumberIds": ["phone-id-1"]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "records": [],
                "pagin": {
                    "totalCount": 0
                }
            }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    saved_config(server.uri()).save(&config_path).unwrap();

    let client = DashboardClient::new(server.uri()).unwrap();
    yc_cli::auth::analytics_calling_logs(
        &client,
        &config_path,
        yc_cli::cli::AnalyticsCallingLogsArgs {
            range: yc_cli::cli::AnalyticsRangeArgs {
                start_time: Some(1782921600000),
                end_time: Some(1783526400000),
            },
            page_no: 1,
            page_size: 20,
            condition: Some("test".to_string()),
            directions: vec!["BUSINESS_INITIATED".to_string()],
            region_codes: vec!["CN".to_string()],
            sources: vec!["CALLING".to_string()],
            status: vec!["COMPLETED".to_string()],
            phone_number_ids: vec!["phone-id-1".to_string()],
        },
    )
    .await
    .unwrap();
}

fn saved_config(base_url: String) -> yc_cli::config::Config {
    yc_cli::config::Config {
        dashboard: yc_cli::config::DashboardConfig { base_url },
        auth: yc_cli::config::AuthConfig {
            token_type: "Bearer".to_string(),
            access_token: "YCLI.access".to_string(),
            refresh_token: "YCLI.refresh".to_string(),
            record_id: "1272676752573050880".to_string(),
            scope: "developers whatsapp:manager:analytics".to_string(),
            tenant_id: Some("tenant-1".to_string()),
            user_id: Some("user-1".to_string()),
        },
    }
}
