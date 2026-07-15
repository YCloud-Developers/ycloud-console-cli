use std::{
    io::Read,
    io::{self, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use url::Url;

use crate::{
    cli::{
        AnalyticsCallingLogsArgs, AnalyticsLogsArgs, AnalyticsOverviewArgs, AnalyticsRangeArgs,
        ContactsListArgs, LoginArgs,
    },
    config::{self, AuthConfig, Config, DashboardConfig},
    http::{
        AnalyticsCallingLogsRequest, AnalyticsLogsRequest, AnalyticsOverviewRequest,
        AnalyticsRangeRequest, ContactsSearchRequest, DashboardClient, TokenData,
    },
    pkce,
};

pub async fn login(client: &DashboardClient, config_path: &Path, args: LoginArgs) -> Result<()> {
    let profile = args.profile.as_str().to_string();
    let requested_permissions = args.permissions.clone();
    let pkce = match args.code_verifier {
        Some(verifier) => pkce::challenge_for_verifier(&verifier),
        None => pkce::generate_pkce_pair(),
    };
    let state = args
        .state
        .unwrap_or_else(|| format!("yc_cli_{}", random_state_suffix()));
    let manual = args.manual || args.code.is_some();
    let (authorize_url, listener) = if manual {
        (
            client.authorize_url(&profile, &requested_permissions, &state, &pkce, None)?,
            None,
        )
    } else {
        let listener = TcpListener::bind("127.0.0.1:0")
            .context("failed to bind localhost callback listener")?;
        let redirect_uri = format!("http://{}/callback", listener.local_addr()?);
        (
            client.authorize_url(
                &profile,
                &requested_permissions,
                &state,
                &pkce,
                Some(&redirect_uri),
            )?,
            Some(listener),
        )
    };

    let code = match args.code {
        Some(code) => code,
        None if manual => {
            println!("Open this URL in a logged-in Dashboard browser:");
            println!("{authorize_url}");
            println!();
            println!("Copy data.code from the JSON response and paste it below.");
            prompt("Authorization code: ")?
        }
        None => {
            println!("Opening browser for Dashboard authorization:");
            println!("{authorize_url}");
            println!();
            if let Err(error) = open_browser(authorize_url.as_str()) {
                println!("Could not open browser automatically: {error:#}");
                println!("Open the URL above in a logged-in Dashboard browser.");
            }
            println!("Waiting for browser callback on localhost...");
            wait_for_callback(
                listener.expect("listener must exist"),
                &state,
                Duration::from_secs(300),
            )?
        }
    };

    let token = client
        .exchange_token(code.trim(), &pkce.code_verifier)
        .await?
        .require_data("token")?;

    ensure_ycli_token(&token)?;
    let mut config = Config {
        dashboard: DashboardConfig {
            base_url: client.base_url(),
        },
        auth: AuthConfig {
            token_type: token.token_type.clone(),
            access_token: token.access_token.clone(),
            refresh_token: token.refresh_token.clone(),
            record_id: token.record_id.clone(),
            requested_permissions: token.requested_permissions.clone(),
            tenant_id: None,
            user_id: None,
        },
    };

    if let Ok(identity) = client
        .whoami(&token.access_token)
        .await
        .and_then(|r| r.require_data("whoami"))
    {
        config.auth.tenant_id = Some(identity.tenant_id);
        config.auth.user_id = Some(identity.user_id);
    }

    config.save(config_path)?;
    println!(
        "Login succeeded. Profile saved at {}.",
        config_path.display()
    );
    Ok(())
}

fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String> {
    listener
        .set_nonblocking(true)
        .context("failed to configure localhost callback listener")?;
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => return handle_callback(&mut stream, expected_state),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    anyhow::bail!("timed out waiting for browser callback");
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error).context("failed to accept browser callback"),
        }
    }
}

fn handle_callback(stream: &mut TcpStream, expected_state: &str) -> Result<String> {
    let mut buffer = [0u8; 4096];
    let size = stream
        .read(&mut buffer)
        .context("failed to read browser callback")?;
    let request = String::from_utf8_lossy(&buffer[..size]);
    let request_line = request
        .lines()
        .next()
        .context("browser callback was empty")?;
    let target = request_line
        .split_whitespace()
        .nth(1)
        .context("browser callback request target is missing")?;
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .context("failed to parse browser callback URL")?;

    if url.path() != "/callback" {
        write_callback_response(stream, 404, "yc login callback not found")?;
        anyhow::bail!("unexpected browser callback path: {}", url.path());
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => error_description = Some(value.into_owned()),
            _ => {}
        }
    }

    if state.as_deref() != Some(expected_state) {
        write_callback_response(stream, 400, "yc login state mismatch")?;
        anyhow::bail!("browser callback state does not match login request");
    }
    if let Some(error) = error {
        write_callback_response(stream, 400, "yc login authorization failed")?;
        if let Some(description) = error_description.filter(|value| !value.trim().is_empty()) {
            anyhow::bail!("dashboard authorization failed: {error}: {description}");
        }
        anyhow::bail!("dashboard authorization failed: {error}");
    }
    let code = code.context("browser callback did not include authorization code")?;
    write_callback_response(
        stream,
        200,
        "yc login succeeded. You can close this browser tab.",
    )?;
    Ok(code)
}

fn write_callback_response(stream: &mut TcpStream, status: u16, message: &str) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>yc login</title></head><body><p>{}</p></body></html>",
        message
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .context("failed to write browser callback response")
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = Command::new("cmd").args(["/C", "start", "", url]).status();
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = Command::new("xdg-open").arg(url).status();

    let status = status.context("failed to start browser command")?;
    if !status.success() {
        anyhow::bail!("browser command exited with status {status}");
    }
    Ok(())
}

pub async fn whoami(client: &DashboardClient, config_path: &Path) -> Result<()> {
    let mut config = Config::load(config_path)?;
    let identity = client
        .whoami(&config.auth.access_token)
        .await?
        .require_data("whoami")?;
    config.auth.tenant_id = Some(identity.tenant_id.clone());
    config.auth.user_id = Some(identity.user_id.clone());
    config.save(config_path)?;

    println!("userId: {}", identity.user_id);
    println!("tenantId: {}", identity.tenant_id);
    println!(
        "requestedPermissions: {}",
        identity.requested_permissions.join(",")
    );
    println!(
        "effectivePermissions: {}",
        identity.effective_permissions.join(",")
    );
    Ok(())
}

pub async fn tenants_list(client: &DashboardClient, config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let tenants = client
        .tenants(&config.auth.access_token)
        .await?
        .require_data("tenants/list")?;
    println!("{}", serde_json::to_string_pretty(&tenants.tenants)?);
    Ok(())
}

pub async fn contacts_list(
    client: &DashboardClient,
    config_path: &Path,
    args: ContactsListArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let contacts = client
        .contacts_search(
            &config.auth.access_token,
            ContactsSearchRequest {
                page_no: args.page_no,
                page_size: args.page_size,
                condition: args.condition.as_deref(),
            },
        )
        .await?
        .require_data("contacts/search")?;
    println!("{}", serde_json::to_string_pretty(&contacts)?);
    Ok(())
}

pub async fn contacts_metadata(client: &DashboardClient, config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let metadata = client
        .contacts_metadata(&config.auth.access_token)
        .await?
        .require_data("contacts/metadata")?;
    println!("{}", serde_json::to_string_pretty(&metadata)?);
    Ok(())
}

pub async fn integrations_status(client: &DashboardClient, config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let statuses = client
        .integrations_status(&config.auth.access_token)
        .await?
        .require_data("integrations/status")?;
    println!("{}", serde_json::to_string_pretty(&statuses)?);
    Ok(())
}

pub async fn analytics_outline(
    client: &DashboardClient,
    config_path: &Path,
    args: AnalyticsRangeArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let range = analytics_range(&args)?;
    let outline = client
        .whatsapp_analytics_outline(&config.auth.access_token, range)
        .await?
        .data
        .unwrap_or_else(|| serde_json::json!({}));
    println!("{}", serde_json::to_string_pretty(&outline)?);
    Ok(())
}

pub async fn analytics_overview(
    client: &DashboardClient,
    config_path: &Path,
    args: AnalyticsOverviewArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let range = analytics_range(&args.range)?;
    let request = AnalyticsOverviewRequest {
        start_time: range.start_time,
        end_time: range.end_time,
        timezone: &args.timezone,
        from: args.from.as_deref(),
        region_code: args.region_code.as_deref(),
        message_category: args.message_category.as_deref(),
    };

    let delivery = client
        .whatsapp_delivery_analytics(&config.auth.access_token, &request)
        .await?
        .data
        .unwrap_or(serde_json::Value::Null);
    let message_detail = client
        .whatsapp_message_detail(&config.auth.access_token, &request)
        .await?
        .data
        .unwrap_or(serde_json::Value::Null);
    let failure_reason_share = client
        .whatsapp_failure_reason_share(&config.auth.access_token, &request)
        .await?
        .data
        .unwrap_or(serde_json::Value::Null);

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "range": {
                "startTime": range.start_time,
                "endTime": range.end_time,
                "timezone": args.timezone,
            },
            "deliveryAnalytics": delivery,
            "messageDetail": message_detail,
            "failureReasonShare": failure_reason_share,
        }))?
    );
    Ok(())
}

pub async fn analytics_logs(
    client: &DashboardClient,
    config_path: &Path,
    args: AnalyticsLogsArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let range = analytics_range(&args.range)?;
    let source = args.source.as_deref();
    let request = AnalyticsLogsRequest {
        direction: &args.direction,
        start_time: range.start_time,
        end_time: range.end_time,
        page_no: args.page_no,
        page_size: args.page_size,
        timezone: &args.timezone,
        condition: args.condition.as_deref(),
        from: args.from.as_deref(),
        business_phones: args.business_phones.iter().map(String::as_str).collect(),
        to_region_codes: args.to_region_codes.iter().map(String::as_str).collect(),
        status: args.status.as_deref(),
        smb: source
            .map(|value| value == "WhatsApp Business App" || value.eq_ignore_ascii_case("smb")),
        pricing_category: args.pricing_category.iter().map(String::as_str).collect(),
    };
    let logs = client
        .whatsapp_logs_search(&config.auth.access_token, &request)
        .await?
        .data
        .unwrap_or(serde_json::Value::Null);
    println!("{}", serde_json::to_string_pretty(&logs)?);
    Ok(())
}

pub async fn analytics_calling_logs(
    client: &DashboardClient,
    config_path: &Path,
    args: AnalyticsCallingLogsArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let range = analytics_range(&args.range)?;
    let request = AnalyticsCallingLogsRequest {
        start_time: range.start_time,
        end_time: range.end_time,
        page_no: args.page_no,
        page_size: args.page_size,
        condition: args.condition.as_deref(),
        directions: args.directions.iter().map(String::as_str).collect(),
        region_codes: args.region_codes.iter().map(String::as_str).collect(),
        sources: args.sources.iter().map(String::as_str).collect(),
        status: args.status.iter().map(String::as_str).collect(),
        phone_number_ids: args.phone_number_ids.iter().map(String::as_str).collect(),
    };
    let logs = client
        .calling_logs_search(&config.auth.access_token, &request)
        .await?
        .data
        .unwrap_or(serde_json::Value::Null);
    println!("{}", serde_json::to_string_pretty(&logs)?);
    Ok(())
}

pub async fn refresh(client: &DashboardClient, config_path: &Path) -> Result<()> {
    let mut config = Config::load(config_path)?;
    let token = client
        .refresh(&config.auth.refresh_token)
        .await?
        .require_data("refresh")?;
    ensure_ycli_token(&token)?;
    config.auth.token_type = token.token_type;
    config.auth.access_token = token.access_token;
    config.auth.refresh_token = token.refresh_token;
    config.auth.record_id = token.record_id;
    config.auth.requested_permissions = token.requested_permissions;
    config.save(config_path)?;
    println!("Token refreshed.");
    Ok(())
}

pub async fn logout(client: &DashboardClient, config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let _ = client
        .revoke(&config.auth.access_token, &config.auth.record_id)
        .await
        .context("failed to revoke server-side token")?;
    config::remove(config_path)?;
    println!("Logged out. Local profile removed.");
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush().context("failed to flush stdout")?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("failed to read stdin")?;
    Ok(line.trim().to_string())
}

fn ensure_ycli_token(token: &TokenData) -> Result<()> {
    if token.token_type != "Bearer" {
        anyhow::bail!("backend returned an unsupported token type");
    }
    if !token.access_token.starts_with("YCLI.") {
        anyhow::bail!("backend returned a non-YCLI access token");
    }
    if token.permission_model_version != 1 {
        anyhow::bail!(
            "unsupported CLI permission model version: {}",
            token.permission_model_version
        );
    }
    if token.requested_permissions.is_empty() {
        anyhow::bail!("backend returned an empty CLI permission snapshot");
    }
    Ok(())
}

fn analytics_range(args: &AnalyticsRangeArgs) -> Result<AnalyticsRangeRequest> {
    let end_time = match args.end_time {
        Some(end_time) => end_time,
        None => now_millis()?,
    };
    let start_time = args
        .start_time
        .unwrap_or(end_time - Duration::from_secs(7 * 24 * 60 * 60).as_millis() as i64);
    if start_time >= end_time {
        anyhow::bail!("start-time must be earlier than end-time");
    }
    Ok(AnalyticsRangeRequest {
        start_time,
        end_time,
    })
}

fn now_millis() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    Ok(duration.as_millis() as i64)
}

fn random_state_suffix() -> String {
    let bytes: [u8; 8] = rand::random();
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::handle_callback;
    use anyhow::Result;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
    };

    #[test]
    fn callback_error_includes_decoded_description() {
        let (result, response) = invoke_callback(
            "/callback?error=invalid_scope&error_description=permission%20is%20not%20active%3A%20yc.member.list.read&state=expected",
            "expected",
        );

        let error = result.expect_err("authorization rejection should fail");
        assert_eq!(
            error.to_string(),
            "dashboard authorization failed: invalid_scope: permission is not active: yc.member.list.read"
        );
        assert!(response.starts_with("HTTP/1.1 400 Bad Request"));
    }

    #[test]
    fn callback_error_validates_state_before_reporting_authorization_error() {
        let (result, response) = invoke_callback(
            "/callback?error=access_denied&error_description=permission%20denied&state=unexpected",
            "expected",
        );

        let error = result.expect_err("mismatched state should fail");
        assert_eq!(
            error.to_string(),
            "browser callback state does not match login request"
        );
        assert!(response.contains("yc login state mismatch"));
    }

    fn invoke_callback(target: &str, expected_state: &str) -> (Result<String>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind callback listener");
        let mut browser = TcpStream::connect(listener.local_addr().expect("listener address"))
            .expect("connect callback client");
        let (mut callback, _) = listener.accept().expect("accept callback client");
        write!(
            browser,
            "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .expect("write callback request");

        let result = handle_callback(&mut callback, expected_state);
        drop(callback);

        let mut response = String::new();
        browser
            .read_to_string(&mut response)
            .expect("read callback response");
        (result, response)
    }
}
