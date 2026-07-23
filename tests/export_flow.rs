use std::time::Duration;

use sha2::{Digest, Sha256};
use wiremock::{
    matchers::{body_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};
use ycloud_console_cli::http::{DashboardClient, ExportTask, InvocationMode};

fn task(status: &str) -> serde_json::Value {
    serde_json::json!({
        "taskId": "task-1",
        "taskType": "COMBINED",
        "status": status,
        "progress": 0,
        "recordCount": 2,
        "artifacts": [],
        "warnings": []
    })
}

#[tokio::test]
async fn create_export_sends_idempotency_key_and_is_never_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/v1/inbox/conversation-exports"))
        .and(header("authorization", "Bearer YCLI.access"))
        .and(header("idempotency-key", "export-key-1"))
        .and(body_json(serde_json::json!({
            "filter": {"inboxIds": ["inbox-1"]},
            "includeContacts": true
        })))
        .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
            "code": 503,
            "msg": "unavailable",
            "error": {"code": "downstream_unavailable", "message": "retry later", "retryable": true},
            "requestId": "req-1",
            "traceId": "trace-1"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = DashboardClient::new_with_mode(server.uri(), InvocationMode::Automation).unwrap();
    let error = client
        .create_conversation_export(
            "YCLI.access",
            "export-key-1",
            &serde_json::json!({
                "filter": {"inboxIds": ["inbox-1"]},
                "includeContacts": true
            }),
        )
        .await
        .expect_err("async create must not retry a 503");
    assert!(error.to_string().contains("downstream_unavailable"));
}

#[tokio::test]
async fn query_understands_partial_success_and_ready_artifacts() {
    let server = MockServer::start().await;
    let mut data = task("PARTIAL_SUCCESS");
    data["progress"] = serde_json::json!(100);
    data["truncated"] = serde_json::json!(true);
    data["truncationReason"] = serde_json::json!("export_limit_reached");
    data["artifacts"] = serde_json::json!([
        {"type": "CONVERSATIONS", "format": "CSV", "fileName": "conversations.csv", "status": "READY", "recordCount": 2},
        {"type": "CONTACTS", "format": "CSV", "fileName": "contacts.csv", "status": "FAILED", "recordCount": 0},
        {"type": "MANIFEST", "format": "JSON", "fileName": "manifest.json", "status": "READY"}
    ]);
    data["warnings"] = serde_json::json!(["contact artifact failed"]);
    Mock::given(method("POST"))
        .and(path("/api/cli/v1/exports/query"))
        .and(body_json(serde_json::json!({"taskId": "task-1"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": data
        })))
        .mount(&server)
        .await;

    let client = DashboardClient::new(server.uri()).unwrap();
    let value: ExportTask = client
        .query_export("YCLI.access", "task-1")
        .await
        .unwrap()
        .require_data("query")
        .unwrap();
    assert!(value.terminal());
    assert_eq!(value.status, "PARTIAL_SUCCESS");
    assert_eq!(value.truncated, Some(true));
    assert_eq!(
        value.truncation_reason.as_deref(),
        Some("export_limit_reached")
    );
    assert_eq!(
        value
            .artifacts
            .iter()
            .filter(|item| item.status == "READY")
            .count(),
        2
    );
}

#[tokio::test]
async fn signed_artifact_download_streams_and_returns_checksum() {
    let server = MockServer::start().await;
    let content = b"contactId,nickName\n1,Alice\n";
    Mock::given(method("GET"))
        .and(path("/signed/contacts.csv"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content))
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("contacts.csv.part");
    let client = DashboardClient::new_with_timeout(server.uri(), Duration::from_secs(2)).unwrap();
    let receipt = client
        .download_to_file(&format!("{}/signed/contacts.csv", server.uri()), &path)
        .await
        .unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), content);
    assert_eq!(receipt.size, content.len() as u64);
    assert_eq!(
        receipt.checksum_sha256,
        format!("{:x}", Sha256::digest(content))
    );
}

#[tokio::test]
async fn message_part_url_request_uses_stable_artifact_identity() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/cli/v1/exports/artifact-url"))
        .and(body_json(serde_json::json!({
            "taskId": "task-1",
            "artifactType": "MESSAGES",
            "artifactId": "messages-00002",
            "partNumber": 2
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "taskId": "task-1",
                "artifactId": "messages-00002",
                "artifactType": "MESSAGES",
                "partNumber": 2,
                "fileName": "messages-00002.jsonl",
                "url": "https://example.invalid/messages-00002.jsonl",
                "expiresAt": 1000
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = DashboardClient::new(server.uri()).unwrap();
    let value = client
        .export_artifact_url(
            "YCLI.access",
            "task-1",
            "MESSAGES",
            Some("messages-00002"),
            Some(2),
        )
        .await
        .unwrap()
        .require_data("artifact URL")
        .unwrap();

    assert_eq!(value.artifact_id.as_deref(), Some("messages-00002"));
    assert_eq!(value.part_number, Some(2));
}
