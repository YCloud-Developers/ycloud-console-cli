use serde_json::json;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};
use ycloud_console_cli::{
    config::OAuthSession,
    http::{DashboardClient, InvocationMode},
    oauth::OAuthClient,
};
async fn discovery(server: &MockServer, resource: &str) {
    let issuer = server.uri();
    Mock::given(method("GET")).and(path("/.well-known/oauth-authorization-server"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({"issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),"token_endpoint":format!("{issuer}/token"),"revocation_endpoint":format!("{issuer}/revoke"),"code_challenge_methods_supported":["S256"],"grant_types_supported":["authorization_code","refresh_token"],"response_types_supported":["code"],"token_endpoint_auth_methods_supported":["none"],"authorization_response_iss_parameter_supported":true}))).mount(server).await;
    Mock::given(method("GET")).and(path("/.well-known/oauth-protected-resource/api/cli"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({"resource":resource,"authorization_servers":[issuer],"scopes_supported":["yc.identity.current.read"],"bearer_methods_supported":["header"]}))).mount(server).await;
}
#[tokio::test]
async fn explicit_platform_login_uses_trusted_original_logical_audience() {
    std::env::set_var("YCLOUD_OAUTH_TEST_LOOPBACK", "1");
    let server = MockServer::start().await;
    let resource = "https://www.ycloud.com/api/cli";
    discovery(&server, resource).await;
    let mut client = OAuthClient::new(&server.uri()).unwrap();
    assert!(client.discover().await.is_err());
    client.discover_for_login(false).await.unwrap();
    assert_eq!(client.resource(), resource);
    let mut session = OAuthSession {
        version: 3,
        business_base_url: Some(server.uri()),
        issuer: server.uri(),
        client_id: "ycloud-console-cli".into(),
        resource: resource.into(),
        token_endpoint: Some(format!("{}/token", server.uri())),
        revocation_endpoint: Some(format!("{}/revoke", server.uri())),
        access_expires_at: 4102444800,
        refresh_uncertain: false,
    };
    client.require_session(&session).unwrap();
    session.version = 2;
    assert!(client.require_session(&session).is_err());
    session.version = 3;
    session.business_base_url = Some("https://elsewhere.example".into());
    assert!(client.require_session(&session).is_err());
}
#[tokio::test]
async fn explicit_resource_mismatch_is_not_adopted() {
    std::env::set_var("YCLOUD_OAUTH_TEST_LOOPBACK", "1");
    let server = MockServer::start().await;
    discovery(&server, "https://www.ycloud.com/api/cli").await;
    assert!(OAuthClient::new(&server.uri())
        .unwrap()
        .discover_for_login(true)
        .await
        .is_err());
}
#[tokio::test]
async fn malformed_or_mcp_resource_is_rejected_even_from_issuer() {
    std::env::set_var("YCLOUD_OAUTH_TEST_LOOPBACK", "1");
    let server = MockServer::start().await;
    discovery(&server, "https://www.ycloud.com/mcp").await;
    assert!(OAuthClient::new(&server.uri())
        .unwrap()
        .discover_for_login(false)
        .await
        .is_err());
}
#[tokio::test]
async fn platform_business_transport_does_not_send_to_logical_resource_host() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/cli/v1/whoami"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"data":{"userId":"u","tenantId":"t"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(DashboardClient::new(server.uri())
        .unwrap()
        .bind_oauth("https://www.ycloud.com/api/cli")
        .is_err());
    let client = DashboardClient::new(server.uri())
        .unwrap()
        .bind_oauth_with_issuer("https://www.ycloud.com/api/cli", &server.uri())
        .unwrap();
    client.whoami("fixture").await.unwrap();
}
#[tokio::test]
async fn gone_business_route_is_terminal_even_if_intermediary_marks_retryable() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/cli/v1/whoami"))
      .respond_with(ResponseTemplate::new(410).set_body_json(json!({"code":410,"error":{"code":"cli_upgrade_required","message":"Upgrade CLI and run ycloud login","retryable":true},"data":null}))).expect(1).mount(&server).await;
    let error = DashboardClient::new_with_mode(server.uri(), InvocationMode::Automation)
        .unwrap()
        .whoami("fixture")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cli_upgrade_required"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn overview_preserves_partial_error_without_atomic_read_fallback() {
    use ycloud_console_cli::http::AnalyticsRequest;
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/cli/v1/whatsapp/analytics/overview"))
      .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{
        "range":{"startTime":"2026-10-01T00:00:00Z","endTime":"2026-10-02T00:00:00Z","timezone":"UTC"},
        "summary":{"status":"ok","data":{"messageTypes":[]}},
        "trend":{"status":"error","error":{"code":"downstream_unavailable","message":"retry later","retryable":true,"requestId":"r1","traceId":"t1"}},
        "templates":{"status":"ok","data":{"pageNo":1,"pageSize":10,"totalElements":0,"records":[]}}
      } }))).expect(1).mount(&server).await;
    let request = AnalyticsRequest {
        start_time: "2026-10-01T00:00:00Z".into(),
        end_time: "2026-10-02T00:00:00Z".into(),
        timezone: "UTC".into(),
        froms: vec![],
        region_codes: vec![],
        message_types: vec![],
        waba_ids: vec![],
    };
    let result = DashboardClient::new(server.uri())
        .unwrap()
        .analytics_overview("fixture", &request)
        .await
        .unwrap()
        .require_data("overview")
        .unwrap();
    assert!(result.summary.into_result().is_ok());
    assert!(result
        .trend
        .into_result()
        .unwrap_err()
        .to_string()
        .contains("downstream_unavailable"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
