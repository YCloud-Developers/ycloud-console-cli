use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn failed_standard_discovery_never_falls_back_or_replaces_the_saved_profile() {
    for status in [404, 503] {
        for compatibility_flag in [false, true] {
            let issuer = MockServer::start().await;
            let business = MockServer::start().await;
            Mock::given(path("/.well-known/oauth-authorization-server"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&issuer)
                .await;
            let directory = tempfile::tempdir().unwrap();
            let profile = directory.path().join("config.toml");
            // Login must not read or overwrite a previous profile until the new grant succeeds.
            std::fs::write(&profile, "existing-profile-fixture").unwrap();
            let issuer_url = issuer.uri();
            let business_url = business.uri();
            let location = profile.clone();
            let output = tokio::task::spawn_blocking(move || {
                let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ycloud"));
                command.env("YCLOUD_OAUTH_TEST_LOOPBACK", "1").args([
                    "--config",
                    location.to_str().unwrap(),
                    "--dashboard-url",
                    &business_url,
                    "login",
                    "--oauth-issuer",
                    &issuer_url,
                    "--no-browser",
                ]);
                if compatibility_flag {
                    command.arg("--oauth");
                }
                command.output().unwrap()
            })
            .await
            .unwrap();
            assert!(!output.status.success());
            assert_eq!(
                std::fs::read_to_string(&profile).unwrap(),
                "existing-profile-fixture"
            );
            assert!(business.received_requests().await.unwrap().is_empty());
            let requests = issuer.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].url.path(),
                "/.well-known/oauth-authorization-server"
            );
        }
    }
}
