use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
use wiremock::{
    matchers::{body_bytes, body_partial_json, header, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};
use ycloud_console_cli::config::{AuthConfig, Config, DashboardConfig, OAuthSession};

const PREVIEW: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SECRET: &str = "fixture-confirmation-secret";
const ROOT: &str = "/api/cli/v1/whatsapp";

fn profile(server: &MockServer, location: &Path) {
    let origin = server.uri();
    Config {
        dashboard: DashboardConfig {
            base_url: origin.clone(),
        },
        auth: AuthConfig {
            oauth: Some(OAuthSession {
                version: 2,
                business_base_url: Some(origin.clone()),
                token_endpoint: Some(format!("{origin}/api/oauth/v1/token")),
                revocation_endpoint: Some(format!("{origin}/api/oauth/v1/revoke")),
                refresh_uncertain: false,
                issuer: origin.clone(),
                client_id: "ycloud-console-cli".into(),
                resource: format!("{origin}/api/cli"),
                access_expires_at: 4_102_444_800,
            }),
            token_type: "Bearer".into(),
            access_token: "fixture-access".into(),
            refresh_token: "fixture-refresh".into(),
            record_id: "fixture-record".into(),
            requested_permissions: vec![],
            tenant_id: None,
            user_id: None,
        },
    }
    .save(location)
    .unwrap();
}

async fn run(config: &Path, arguments: &[&str], answer: Option<&str>) -> Output {
    let config = config.to_owned();
    let arguments: Vec<String> = arguments.iter().map(|value| value.to_string()).collect();
    let answer = answer.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ycloud"))
            .env("YCLOUD_OAUTH_TEST_LOOPBACK", "1")
            .arg("--config")
            .arg(config)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(answer) = answer {
            writeln!(child.stdin.take().unwrap(), "{answer}").unwrap();
        }
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap()
}

fn ok(value: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":value}))
}

async fn preview(server: &MockServer, route: &str, expected: Value, count: u64) {
    Mock::given(method("POST")).and(path(route)).and(header("authorization","Bearer fixture-access"))
        .and(body_partial_json(json!({"phase":"PREVIEW"}))).and(body_partial_json(expected))
        .respond_with(ok(json!({
            "phase":"PREVIEW","previewId":PREVIEW,"confirmationToken":SECRET,
            "campaignId":"campaign-fixture","resourceVersion":"version-confirmed","expiresAt":4102444800000_i64,
            "plannedChange":[{"action":"PREPARE","resourceType":"CAMPAIGN_RECORD","summary":"Local fixture"}],"risk":"MEDIUM"
        }))).expect(count).mount(server).await;
}

fn assert_output(output: Output) -> Value {
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{stderr}");
    assert!(!stdout.contains(SECRET) && !stderr.contains(SECRET));
    serde_json::from_str(&stdout).unwrap()
}

#[tokio::test]
async fn preview_requires_confirmation_and_lost_trial_reply_reads_original_receipt() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let input = dir.path().join("trial.json");
    profile(&server, &config);
    let original = std::fs::read(&config).unwrap();
    let data = json!({"resourceVersion":"campaign-revision","testTarget":{"kind":"PHONE","value":"+12025550123"},"sampleId":format!("{PREVIEW}:1")});
    std::fs::write(&input, data.to_string()).unwrap();
    let route = format!("{ROOT}/campaigns/campaign-fixture/test");
    preview(&server, &route, data, 3).await;

    Mock::given(method("POST")).and(path(&route))
        .and(body_partial_json(json!({"phase":"EXECUTE","execution":{"confirmationToken":SECRET,"resourceVersion":"version-confirmed"}})))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({"code":503,"error":{"code":"downstream_unavailable","message":"Fixture interruption","retryable":true}})))
        .expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path(format!("{ROOT}/campaigns/campaign-fixture")))
        .and(query_param("operationId",PREVIEW))
        .respond_with(ok(json!({"campaignId":"campaign-fixture","selectedOperation":{"operationId":PREVIEW,"state":"SUCCEEDED","messageId":"mock-message"}})))
        .expect(1).mount(&server).await;

    let mut args = vec![
        "campaign",
        "test",
        "--campaign-id",
        "campaign-fixture",
        "--input",
        input.to_str().unwrap(),
    ];
    assert_eq!(
        assert_output(run(&config, &args, None).await)["phase"],
        "PREVIEW"
    );
    args.push("--apply");
    let refused = run(&config, &args, Some("wrong-preview")).await;
    assert!(!refused.status.success());
    let receipt = assert_output(run(&config, &args, Some(PREVIEW)).await);
    assert_eq!(
        receipt["receipt"]["selectedOperation"]["messageId"],
        "mock-message"
    );
    assert_eq!(std::fs::read(config).unwrap(), original);
}

#[tokio::test]
async fn native_upload_streams_exact_bytes_only_to_the_bound_route() {
    for same_origin in [true, false] {
        let server = MockServer::start().await;
        let other = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let attachment = dir.path().join("audience.xlsx");
        let content = b"fixture-private-file";
        std::fs::write(&attachment, content).unwrap();
        profile(&server, &config);

        let route = format!("{ROOT}/assets");
        preview(&server, &route, json!({"purpose":"AUDIENCE","sizeBytes":content.len(),"sha256":format!("{:x}",Sha256::digest(content))}), 1).await;
        let content_route = format!("{route}/{PREVIEW}/content");
        let target = format!(
            "{}{}",
            if same_origin {
                server.uri()
            } else {
                other.uri()
            },
            content_route
        );
        Mock::given(method("POST")).and(path(route))
            .and(body_partial_json(json!({"phase":"EXECUTE"})))
            .respond_with(ok(json!({"phase":"EXECUTE","assetId":PREVIEW,"upload":{"method":"PUT","url":target,"expiresAt":4102444800000_i64}})))
            .expect(1).mount(&server).await;
        Mock::given(method("PUT"))
            .and(path(&content_route))
            .and(header("authorization", "Bearer fixture-access"))
            .and(header("content-type", "application/octet-stream"))
            .and(body_bytes(content.as_slice()))
            .respond_with(ok(
                json!({"phase":"EXECUTE","assetId":PREVIEW,"assetStatus":"PARSING"}),
            ))
            .expect(u64::from(same_origin))
            .mount(&server)
            .await;
        let output = run(
            &config,
            &[
                "whatsapp",
                "asset",
                "upload",
                "--purpose",
                "audience",
                "--content-type",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "--file",
                attachment.to_str().unwrap(),
                "--apply",
            ],
            Some(PREVIEW),
        )
        .await;
        if same_origin {
            assert_eq!(assert_output(output)["assetStatus"], "PARSING");
        } else {
            assert!(!output.status.success());
            assert!(other.received_requests().await.unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn template_prepare_and_submit_keep_distinct_previews_and_query_review_state() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let input = dir.path().join("template.json");
    profile(&server, &config);
    for (action, fields) in [
        (
            "prepare",
            json!({
                "wabaId":"waba-fixture","action":"CREATE","source":{"kind":"CONTENT"},
                "name":"fixture_order","category":"UTILITY",
                "languages":[{"language":"en_US","components":[{"type":"BODY","text":"Your order is ready."}]}]
            }),
        ),
        (
            "submit",
            json!({"preparationOperationId":"prep-fixture","preparedVersion":"prepared-v1"}),
        ),
    ] {
        std::fs::write(&input, fields.to_string()).unwrap();
        let route = format!("{ROOT}/templates/{action}");
        preview(&server, &route, fields, 1).await;
        Mock::given(method("POST")).and(path(route)).and(body_partial_json(json!({"phase":"EXECUTE"})))
            .respond_with(ok(json!({"phase":"EXECUTE","operationId":PREVIEW,"resourceVersion":"version-confirmed","state":"QUEUED"})))
            .expect(1).mount(&server).await;
        let mut args = vec![
            "whatsapp",
            "template",
            action,
            "--input",
            input.to_str().unwrap(),
            "--apply",
        ];
        if action == "submit" {
            args.extend(["--waba-id", "waba-fixture"]);
        }
        assert_eq!(
            assert_output(run(&config, &args, Some(PREVIEW)).await)["state"],
            "QUEUED"
        );
    }
    Mock::given(method("GET"))
        .and(path(format!("{ROOT}/template-detail")))
        .and(query_param("ref[kind]", "OPERATION"))
        .and(query_param("ref[operationId]", PREVIEW))
        .respond_with(ok(
            json!({"operationId":PREVIEW,"operationState":"UNKNOWN"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let read = run(
        &config,
        &[
            "whatsapp",
            "template",
            "get",
            "--waba-id",
            "waba-fixture",
            "--operation-id",
            PREVIEW,
        ],
        None,
    )
    .await;
    assert_eq!(assert_output(read)["operationState"], "UNKNOWN");
}

#[tokio::test]
async fn estimate_prepare_query_submit_and_cancel_follow_the_confirmed_routes() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let input = dir.path().join("campaign.json");
    profile(&server, &config);
    let source = json!({"kind":"SEGMENTS","segmentIds":["segment-fixture"]});
    std::fs::write(&input, json!({"source":source}).to_string()).unwrap();
    Mock::given(method("POST"))
        .and(path(format!("{ROOT}/campaigns/audience-estimate")))
        .and(body_partial_json(json!({"source":source})))
        .respond_with(ok(json!({"matchedContactCount":42})))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        assert_output(
            run(
                &config,
                &["campaign", "estimate", "--input", input.to_str().unwrap()],
                None
            )
            .await
        )["matchedContactCount"],
        42
    );

    for (action, route, fields, state) in [
        (
            "prepare",
            "campaigns/prepare",
            json!({
                "name":"Fixture campaign","fromPhone":"+12025550124","wabaId":"waba-fixture",
                "templateId":"template-fixture","language":"en_US","source":source,"bindings":[],"sendType":"NOW"
            }),
            "QUEUED",
        ),
        (
            "submit",
            "campaigns/campaign-fixture/submit",
            json!({"resourceVersion":"ready-v1"}),
            "SUCCEEDED",
        ),
        (
            "cancel",
            "campaigns/campaign-fixture/cancel",
            json!({"resourceVersion":"accepted-v2"}),
            "SUCCEEDED",
        ),
    ] {
        std::fs::write(&input, fields.to_string()).unwrap();
        let route = format!("{ROOT}/{route}");
        preview(&server, &route, fields, 1).await;
        Mock::given(method("POST")).and(path(route)).and(body_partial_json(json!({"phase":"EXECUTE"})))
            .respond_with(ok(json!({"phase":"EXECUTE","campaignId":"campaign-fixture","operationId":PREVIEW,"state":state})))
            .expect(1).mount(&server).await;
        let mut args = vec![
            "campaign",
            action,
            "--input",
            input.to_str().unwrap(),
            "--apply",
        ];
        if action != "prepare" {
            args.extend(["--campaign-id", "campaign-fixture"]);
        }
        assert_eq!(
            assert_output(run(&config, &args, Some(PREVIEW)).await)["state"],
            state
        );
        if action == "prepare" {
            Mock::given(method("GET")).and(path(format!("{ROOT}/campaigns/campaign-fixture")))
                .and(query_param("operationId",PREVIEW))
                .respond_with(ok(json!({"preparationState":"READY","resourceVersion":"ready-v1","selectedOperation":{"state":"SUCCEEDED"}})))
                .expect(1).mount(&server).await;
            let read = run(
                &config,
                &[
                    "campaign",
                    "get",
                    "--campaign-id",
                    "campaign-fixture",
                    "--operation-id",
                    PREVIEW,
                ],
                None,
            )
            .await;
            assert_eq!(assert_output(read)["preparationState"], "READY");
        }
    }
}
