use wiremock::{
    matchers::{body_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};
use ycloud_console_cli::{
    campaign::{
        CampaignAnalyticsRequest, CampaignMessageExportRequest, CampaignMessageSearchRequest,
    },
    http::DashboardClient,
};

#[tokio::test]
async fn multi_campaign_message_search_uses_stable_cursor_contract() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/v1/campaigns/messages/search"))
        .and(header("authorization", "Bearer YCLI.access"))
        .and(body_json(serde_json::json!({
            "campaignIds": ["campaign-1", "campaign-2"],
            "startTime": 1,
            "endTime": 2,
            "timezone": "UTC",
            "statuses": ["delivered"],
            "limit": 100,
            "cursor": "cursor-1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": [{
                "messageId": "message-1",
                "campaignId": "campaign-1",
                "recipient": "861****8000",
                "status": "delivered"
            }],
            "pagination": {"nextCursor": "cursor-2", "hasMore": true},
            "warnings": []
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = DashboardClient::new(server.uri()).unwrap();
    let response = client
        .campaign_messages_search(
            "YCLI.access",
            &CampaignMessageSearchRequest {
                campaign_ids: vec!["campaign-1".into(), "campaign-2".into()],
                start_time: 1,
                end_time: 2,
                timezone: "UTC".into(),
                statuses: vec!["delivered".into()],
                template_names: Vec::new(),
                business_phones: Vec::new(),
                limit: 100,
                cursor: Some("cursor-1".into()),
            },
        )
        .await
        .unwrap();

    assert_eq!(response.data.unwrap()[0].message_id, "message-1");
    assert_eq!(
        response.pagination.unwrap().next_cursor.as_deref(),
        Some("cursor-2")
    );
}

#[tokio::test]
async fn campaign_analytics_preserves_unavailable_source() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/v1/campaigns/analytics"))
        .and(body_json(serde_json::json!({
            "campaignIds": ["campaign-1"],
            "startTime": 1,
            "endTime": 2,
            "timezone": "UTC",
            "dimensions": ["campaign"]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "windowStart": 1,
                "windowEnd": 2,
                "timezone": "UTC",
                "dataPoints": [{"campaignId": "campaign-1", "total": 2}],
                "availability": [
                    {"source": "security", "available": true, "freshThrough": 2},
                    {"source": "clicks", "available": false, "reasonCode": "source_unavailable"}
                ]
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = DashboardClient::new(server.uri()).unwrap();
    let data = client
        .campaign_analytics(
            "YCLI.access",
            &CampaignAnalyticsRequest {
                campaign_ids: vec!["campaign-1".into()],
                start_time: 1,
                end_time: 2,
                timezone: "UTC".into(),
                dimensions: vec!["campaign".into()],
            },
        )
        .await
        .unwrap()
        .require_data("Campaign analytics")
        .unwrap();

    assert!(!data.availability[1].available);
    assert_eq!(
        data.availability[1].reason_code.as_deref(),
        Some("source_unavailable")
    );
}

#[tokio::test]
async fn campaign_export_and_cancel_use_dedicated_stable_paths() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/v1/campaign-message-exports"))
        .and(header("idempotency-key", "campaign-export-1"))
        .and(body_json(serde_json::json!({
            "campaignIds": ["campaign-1"],
            "startTime": 1,
            "endTime": 2,
            "timezone": "UTC"
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(task("PENDING")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/cli/v1/exports/cancel"))
        .and(body_json(serde_json::json!({"taskId": "task-1"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(task("CANCELED")))
        .expect(1)
        .mount(&server)
        .await;

    let client = DashboardClient::new(server.uri()).unwrap();
    let created = client
        .create_campaign_message_export(
            "YCLI.access",
            "campaign-export-1",
            &CampaignMessageExportRequest {
                campaign_ids: vec!["campaign-1".into()],
                start_time: 1,
                end_time: 2,
                timezone: "UTC".into(),
                statuses: Vec::new(),
                template_names: Vec::new(),
                business_phones: Vec::new(),
                file_name: None,
            },
        )
        .await
        .unwrap()
        .require_data("Campaign export")
        .unwrap();
    let canceled = client
        .cancel_export("YCLI.access", &created.task_id)
        .await
        .unwrap()
        .require_data("export cancel")
        .unwrap();

    assert_eq!(created.task_type, "CAMPAIGN_MESSAGE");
    assert_eq!(canceled.status, "CANCELED");
    assert!(canceled.terminal());
}

fn task(status: &str) -> serde_json::Value {
    serde_json::json!({
        "code": 0,
        "data": {
            "taskId": "task-1",
            "taskType": "CAMPAIGN_MESSAGE",
            "status": status,
            "progress": if status == "CANCELED" { 100 } else { 0 },
            "artifacts": [],
            "warnings": []
        }
    })
}
