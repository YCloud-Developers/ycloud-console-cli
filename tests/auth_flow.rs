use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};
use wiremock::{
    matchers::{body_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};
use ycloud_console_cli::{config::Config, http::DashboardClient};

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
                "recordId": "record-1",
                "requestedPermissions": ["yc.identity.current.read", "yc.tenant.list.read"],
                "permissionModelVersion": 1
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
                "requestedPermissions": ["yc.identity.current.read", "yc.tenant.list.read"],
                "effectivePermissions": ["yc.identity.current.read", "yc.tenant.list.read"]
            }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let client = DashboardClient::new(server.uri()).unwrap();
    ycloud_console_cli::auth::login(
        &client,
        &config_path,
        ycloud_console_cli::cli::LoginArgs {
            profile: ycloud_console_cli::cli::PermissionProfile::Basic,
            permissions: vec![],
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
    assert_eq!(
        config.auth.requested_permissions,
        vec!["yc.identity.current.read", "yc.tenant.list.read"]
    );
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
                "recordId": "record-2",
                "requestedPermissions": ["yc.identity.current.read", "yc.tenant.list.read"],
                "permissionModelVersion": 1
            }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let initial = ycloud_console_cli::config::Config {
        dashboard: ycloud_console_cli::config::DashboardConfig {
            base_url: server.uri(),
        },
        auth: ycloud_console_cli::config::AuthConfig {
            token_type: "Bearer".to_string(),
            access_token: "YCLI.old-access".to_string(),
            refresh_token: "YCLI.old-refresh".to_string(),
            record_id: "record-1".to_string(),
            requested_permissions: vec!["yc.identity.current.read".to_string()],
            tenant_id: None,
            user_id: None,
        },
    };
    initial.save(&config_path).unwrap();

    let client = DashboardClient::new(server.uri()).unwrap();
    ycloud_console_cli::auth::refresh(&client, &config_path)
        .await
        .unwrap();

    let updated = Config::load(&config_path).unwrap();
    assert_eq!(updated.auth.access_token, "YCLI.new-access");
    assert_eq!(updated.auth.refresh_token, "YCLI.new-refresh");
    assert_eq!(updated.auth.record_id, "record-2");
    assert_eq!(
        updated.auth.requested_permissions,
        vec!["yc.identity.current.read", "yc.tenant.list.read"]
    );
}

#[tokio::test]
async fn contacts_list_calls_cli_read_adapter() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/read/contacts/search"))
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
    let initial = ycloud_console_cli::config::Config {
        dashboard: ycloud_console_cli::config::DashboardConfig {
            base_url: server.uri(),
        },
        auth: ycloud_console_cli::config::AuthConfig {
            token_type: "Bearer".to_string(),
            access_token: "YCLI.access".to_string(),
            refresh_token: "YCLI.refresh".to_string(),
            record_id: "1272676752573050880".to_string(),
            requested_permissions: vec!["yc.contact.record.read".to_string()],
            tenant_id: Some("tenant-1".to_string()),
            user_id: Some("user-1".to_string()),
        },
    };
    initial.save(&config_path).unwrap();

    let client = DashboardClient::new(server.uri()).unwrap();
    ycloud_console_cli::auth::contacts_list(
        &client,
        &config_path,
        ycloud_console_cli::cli::ContactsListArgs {
            page_no: 1,
            page_size: 10,
            condition: None,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn contacts_metadata_calls_cli_readonly_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/cli/read/contacts/metadata"))
        .and(header("authorization", "Bearer YCLI.access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "sources": ["whatsapp"],
                "tags": [
                    {
                        "id": "tag-1",
                        "name": "VIP"
                    }
                ],
                "segments": [],
                "segmentFilters": []
            }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    saved_config(server.uri()).save(&config_path).unwrap();

    let client = DashboardClient::new(server.uri()).unwrap();
    ycloud_console_cli::auth::contacts_metadata(&client, &config_path)
        .await
        .unwrap();
}

#[tokio::test]
async fn integrations_status_calls_cli_readonly_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/cli/read/integrations/status"))
        .and(header("authorization", "Bearer YCLI.access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": [
                {
                    "type": "SHOP",
                    "integration": "SHOPIFY",
                    "status": "ENABLED"
                }
            ]
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    saved_config(server.uri()).save(&config_path).unwrap();

    let client = DashboardClient::new(server.uri()).unwrap();
    ycloud_console_cli::auth::integrations_status(&client, &config_path)
        .await
        .unwrap();
}

#[tokio::test]
async fn analytics_overview_calls_cli_read_adapters() {
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
        "/api/cli/read/whatsapp/analytics/delivery",
        "/api/cli/read/whatsapp/analytics/message-detail",
        "/api/cli/read/whatsapp/analytics/failure-reasons",
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
    ycloud_console_cli::auth::analytics_overview(
        &client,
        &config_path,
        ycloud_console_cli::cli::AnalyticsOverviewArgs {
            range: ycloud_console_cli::cli::AnalyticsRangeArgs {
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
async fn analytics_logs_calls_cli_message_search_adapter() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/read/whatsapp/messages/search"))
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
    ycloud_console_cli::auth::analytics_logs(
        &client,
        &config_path,
        ycloud_console_cli::cli::AnalyticsLogsArgs {
            range: ycloud_console_cli::cli::AnalyticsRangeArgs {
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
async fn analytics_calling_logs_calls_cli_calling_search_adapter() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/read/calling/logs/search"))
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
    ycloud_console_cli::auth::analytics_calling_logs(
        &client,
        &config_path,
        ycloud_console_cli::cli::AnalyticsCallingLogsArgs {
            range: ycloud_console_cli::cli::AnalyticsRangeArgs {
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

fn saved_config(base_url: String) -> ycloud_console_cli::config::Config {
    ycloud_console_cli::config::Config {
        dashboard: ycloud_console_cli::config::DashboardConfig { base_url },
        auth: ycloud_console_cli::config::AuthConfig {
            token_type: "Bearer".to_string(),
            access_token: "YCLI.access".to_string(),
            refresh_token: "YCLI.refresh".to_string(),
            record_id: "1272676752573050880".to_string(),
            requested_permissions: vec![
                "yc.identity.current.read".to_string(),
                "yc.whatsapp.analytics.read".to_string(),
            ],
            tenant_id: Some("tenant-1".to_string()),
            user_id: Some("user-1".to_string()),
        },
    }
}

#[tokio::test]
async fn request_times_out_instead_of_waiting_indefinitely() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/cli/auth/tenants/list"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(200))
                .set_body_json(serde_json::json!({"code": 0, "data": {"tenants": []}})),
        )
        .mount(&server)
        .await;

    let client =
        DashboardClient::new_with_timeout(server.uri(), Duration::from_millis(25)).unwrap();
    let error = client.tenants("YCLI.access").await.unwrap_err();

    assert!(
        error.to_string().to_lowercase().contains("timed out"),
        "unexpected error: {error:#}"
    );
}

#[tokio::test]
async fn response_body_timeout_is_not_hidden_as_a_parse_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{",
            )
            .unwrap();
        release_rx.recv().unwrap();
    });

    let client =
        DashboardClient::new_with_timeout(format!("http://{address}"), Duration::from_millis(25))
            .unwrap();
    let error = client.tenants("YCLI.access").await.unwrap_err();

    release_tx.send(()).unwrap();
    assert!(
        error.to_string().to_lowercase().contains("timed out"),
        "unexpected error: {error:#}"
    );
    server.join().unwrap();
}

#[tokio::test]
async fn integrations_status_prefers_v1_without_legacy_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/cli/v1/integrations/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "msg": "OK",
            "data": [{"type": "SHOP", "integration": "SHOPIFY", "status": "ENABLED"}],
            "error": null,
            "requestId": "req-v1",
            "traceId": "trace-v1",
            "warnings": [],
            "futureField": true
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/cli/read/integrations/status"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let client = DashboardClient::new(server.uri()).unwrap();
    let response = client.integrations_status("YCLI.access").await.unwrap();

    assert_eq!(response.request_id.as_deref(), Some("req-v1"));
    assert!(response.data.is_some());
}

#[tokio::test]
async fn permission_denial_does_not_fallback_and_surfaces_correlation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/cli/v1/integrations/status"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "code": 403,
            "msg": "Permission denied",
            "data": null,
            "error": {"code": "permission_denied", "message": "Permission denied", "retryable": false, "details": {}},
            "requestId": "req-denied",
            "traceId": "trace-denied",
            "warnings": []
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/cli/read/integrations/status"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let client = DashboardClient::new(server.uri()).unwrap();
    let error = client
        .integrations_status("YCLI.access")
        .await
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("permission_denied"),
        "unexpected error: {error}"
    );
    assert!(error.contains("req-denied"), "unexpected error: {error}");
    assert!(error.contains("trace-denied"), "unexpected error: {error}");
}

#[tokio::test]
async fn safe_read_retries_typed_429_with_new_request_id_and_same_invocation() {
    let server = MockServer::start().await;
    let observed = Arc::new(Mutex::new(Vec::<(String, String, String)>::new()));
    let calls = Arc::new(Mutex::new(0usize));
    let observed_responder = Arc::clone(&observed);
    let calls_responder = Arc::clone(&calls);
    Mock::given(method("GET"))
        .and(path("/api/cli/v1/integrations/status"))
        .respond_with(move |request: &wiremock::Request| {
            let request_id = request.headers["x-request-id"].to_str().unwrap().to_string();
            let invocation_id = request.headers["x-ycloud-invocation-id"]
                .to_str()
                .unwrap()
                .to_string();
            let invocation_mode = request.headers["x-ycloud-invocation-mode"]
                .to_str()
                .unwrap()
                .to_string();
            observed_responder
                .lock()
                .unwrap()
                .push((request_id, invocation_id, invocation_mode));
            let mut calls = calls_responder.lock().unwrap();
            *calls += 1;
            if *calls == 1 {
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "0")
                    .set_body_json(serde_json::json!({
                        "code": 429,
                        "msg": "Too many CLI requests",
                        "data": null,
                        "error": {"code": "rate_limited", "message": "Too many CLI requests", "retryable": true, "details": {"retryAfterSeconds": 0}},
                        "requestId": "server-request-1",
                        "traceId": "server-trace-1",
                        "warnings": []
                    }))
            } else {
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "code": 0,
                    "msg": "OK",
                    "data": [],
                    "error": null,
                    "requestId": "server-request-2",
                    "traceId": "server-trace-2",
                    "warnings": []
                }))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/cli/read/integrations/status"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let client = DashboardClient::new(server.uri()).unwrap();
    client.integrations_status("YCLI.access").await.unwrap();

    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 2);
    assert_ne!(observed[0].0, observed[1].0);
    assert_eq!(observed[0].1, observed[1].1);
    assert_eq!(observed[0].2, "interactive");
    assert_eq!(observed[1].2, "interactive");
}

#[tokio::test]
async fn auth_lifecycle_does_not_retry_typed_429() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/auth/token"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "code": 429,
            "msg": "Too many CLI requests",
            "data": null,
            "error": {"code": "rate_limited", "message": "Too many CLI requests", "retryable": true, "details": {"retryAfterSeconds": 1}},
            "requestId": "request-auth-limit",
            "traceId": "trace-auth-limit",
            "warnings": []
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = DashboardClient::new(server.uri()).unwrap();
    let error = client
        .exchange_token("authorization-code", "code-verifier")
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("rate_limited"), "unexpected error: {error}");
    assert!(
        error.contains("request-auth-limit"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn analytics_v1_serializes_rfc3339_and_iana_timezone() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/v1/whatsapp/analytics/delivery"))
        .and(body_json(serde_json::json!({
            "startTime": "1970-01-01T00:00:00.000Z",
            "endTime": "1970-01-01T00:00:01.000Z",
            "timezone": "UTC",
            "regionCode": "US",
            "messageCategory": "utility"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0, "msg": "OK", "data": {"dataPoints": []}, "error": null,
            "requestId": "req-analytics", "traceId": "trace-analytics", "warnings": []
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/cli/read/whatsapp/analytics/delivery"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let client = DashboardClient::new(server.uri()).unwrap();
    let request = ycloud_console_cli::http::AnalyticsOverviewRequest {
        start_time: 0,
        end_time: 1000,
        timezone: "UTC",
        from: None,
        region_code: Some("US"),
        message_category: Some("utility"),
    };

    client
        .whatsapp_delivery_analytics("YCLI.access", &request)
        .await
        .unwrap();
}
