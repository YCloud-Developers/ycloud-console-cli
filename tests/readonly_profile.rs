use std::{
    path::Path,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

use serde_json::json;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};
use ycloud_console_cli::config::{AuthConfig, Config, DashboardConfig, OAuthSession};

#[cfg(unix)]
use std::path::PathBuf;

fn save_profile(server: &MockServer, location: &Path, version: Option<u32>) {
    let base = server.uri();
    Config {
        dashboard: DashboardConfig {
            base_url: base.clone(),
        },
        auth: AuthConfig {
            oauth: version.map(|version| OAuthSession {
                version,
                business_base_url: (version == 2).then(|| base.clone()),
                token_endpoint: (version == 2).then(|| format!("{base}/api/oauth/v1/token")),
                revocation_endpoint: (version == 2).then(|| format!("{base}/api/oauth/v1/revoke")),
                refresh_uncertain: false,
                issuer: base.clone(),
                client_id: "ycloud-console-cli".into(),
                resource: format!("{base}/api/cli"),
                access_expires_at: 4_102_444_800,
            }),
            token_type: "Bearer".into(),
            access_token: "fixture-access".into(),
            refresh_token: "fixture-refresh".into(),
            record_id: "fixture-record".into(),
            requested_permissions: vec!["yc.identity.current.read".into()],
            tenant_id: None,
            user_id: None,
        },
    }
    .save(location)
    .unwrap();
}

async fn mock_queries(server: &MockServer) {
    for (endpoint, data) in [
        (
            "/api/cli/v1/whoami",
            json!({"userId":"fixture-user", "tenantId":"fixture-tenant"}),
        ),
        ("/api/cli/v1/tenants", json!({"tenants":[]})),
        ("/api/cli/v1/contacts/metadata", json!({"sources":[]})),
    ] {
        Mock::given(method("GET"))
            .and(path(endpoint))
            .and(header("authorization", "Bearer fixture-access"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code":0, "data":data})))
            .mount(server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/api/cli/v1/contacts/search"))
        .and(header("authorization", "Bearer fixture-access"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"records":[]}})),
        )
        .mount(server)
        .await;
}

async fn run_cli(location: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ycloud"));
    command
        .env("YCLOUD_OAUTH_TEST_LOOPBACK", "1")
        .arg("--config")
        .arg(location)
        .args(args);
    run_command(command).await
}

async fn run_command(mut command: Command) -> Output {
    tokio::task::spawn_blocking(move || {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().unwrap().is_some() {
                return child.wait_with_output().unwrap();
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("CLI command did not finish within the test deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })
    .await
    .unwrap()
}

async fn assert_queries_leave_profile_unchanged(server: &MockServer, location: &Path) {
    let original = std::fs::read(location).unwrap();
    let modified = std::fs::metadata(location).unwrap().modified().unwrap();
    for args in [
        vec!["whoami"],
        vec!["tenants", "list"],
        vec!["contacts", "metadata"],
        vec!["contacts", "list"],
    ] {
        let output = run_cli(location, &args).await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(std::fs::read(location).unwrap(), original);
    assert_eq!(
        std::fs::metadata(location).unwrap().modified().unwrap(),
        modified
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests
        .iter()
        .all(|r| r.url.path().starts_with("/api/cli/v1/")));
}

#[tokio::test]
async fn queries_work_when_the_lock_path_cannot_be_opened() {
    for version in [None, Some(1), Some(2)] {
        let server = MockServer::start().await;
        mock_queries(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let location = dir.path().join("config.toml");
        save_profile(&server, &location, version);
        // A directory cannot be opened as a writable lock file on any platform.
        std::fs::create_dir(dir.path().join(".config.toml.lock")).unwrap();
        assert_queries_leave_profile_unchanged(&server, &location).await;
    }
}

#[cfg(unix)]
struct ReadOnlyDirectory(PathBuf);

#[cfg(unix)]
impl ReadOnlyDirectory {
    fn new(directory: &Path, profile: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(profile, std::fs::Permissions::from_mode(0o400)).unwrap();
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o500)).unwrap();
        Self(directory.to_path_buf())
    }
}

#[cfg(unix)]
impl Drop for ReadOnlyDirectory {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn queries_work_with_a_read_only_profile_and_directory_without_creating_a_lock() {
    for version in [None, Some(1), Some(2)] {
        let server = MockServer::start().await;
        mock_queries(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let location = dir.path().join("config.toml");
        save_profile(&server, &location, version);
        let _permissions = ReadOnlyDirectory::new(dir.path(), &location);
        assert_queries_leave_profile_unchanged(&server, &location).await;
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[tokio::test]
async fn queries_do_not_overwrite_an_unconfirmed_refresh_marker() {
    let server = MockServer::start().await;
    mock_queries(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("config.toml");
    save_profile(&server, &location, Some(2));
    let mut config = Config::load(&location).unwrap();
    config.auth.oauth.as_mut().unwrap().refresh_uncertain = true;
    config.save(&location).unwrap();
    assert_queries_leave_profile_unchanged(&server, &location).await;
    assert!(
        Config::load(&location)
            .unwrap()
            .auth
            .oauth
            .unwrap()
            .refresh_uncertain
    );
}

#[tokio::test]
async fn refresh_does_not_create_locks_and_ignores_obsolete_lock_paths() {
    for (version, obsolete_lock) in [
        (None, false),
        (None, true),
        (Some(2), false),
        (Some(2), true),
    ] {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let location = dir.path().join("config.toml");
        save_profile(&server, &location, version);
        if obsolete_lock {
            std::fs::create_dir(dir.path().join(".config.toml.lock")).unwrap();
        }
        let (endpoint, response) = if version.is_some() {
            (
                "/api/oauth/v1/token",
                json!({
                    "access_token":"new-access-fixture", "refresh_token":"new-refresh-fixture",
                    "token_type":"Bearer", "expires_in":3600, "scope":"yc.identity.current.read"
                }),
            )
        } else {
            (
                "/api/cli/auth/refresh",
                json!({"code":0,"data":{
                    "accessToken":"YCLI.new-access", "refreshToken":"YCLI.new-refresh",
                    "tokenType":"Bearer", "recordId":"fixture-record",
                    "requestedPermissions":["yc.identity.current.read"],"permissionModelVersion":1
                }}),
            )
        };
        Mock::given(method("POST"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .expect(1)
            .mount(&server)
            .await;
        let output = run_cli(&location, &["refresh"]).await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        let saved = Config::load(&location).unwrap();
        assert_ne!(saved.auth.refresh_token, "fixture-refresh");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            if obsolete_lock { 2 } else { 1 }
        );
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn sandbox_denies_standard_refresh_before_http_even_without_file_locking() {
    let server = MockServer::start().await;
    mock_queries(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let directory = dir.path().canonicalize().unwrap();
    let location = directory.join("config.toml");
    save_profile(&server, &location, Some(2));
    let original = std::fs::read(&location).unwrap();
    let sandbox = format!("(version 1)(allow default)(deny file-write* (subpath {directory:?}))");
    for action in ["whoami", "refresh"] {
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command
            .args(["-p", &sandbox, env!("CARGO_BIN_EXE_ycloud")])
            .env("YCLOUD_OAUTH_TEST_LOOPBACK", "1")
            .arg("--config")
            .arg(&location)
            .arg(action);
        let output = run_command(command).await;
        if action == "whoami" {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        } else {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr)
                .contains("failed to create temporary config"));
        }
    }
    assert_eq!(std::fs::read(&location).unwrap(), original);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/api/cli/v1/whoami");
}
