use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

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
    let pkce = match args.code_verifier {
        Some(verifier) => pkce::challenge_for_verifier(&verifier),
        None => pkce::generate_pkce_pair(),
    };
    let state = args
        .state
        .unwrap_or_else(|| format!("yc_cli_{}", random_state_suffix()));
    let authorize_url = client.authorize_url(&args.scope, &state, &pkce)?;

    println!("Open this URL in a logged-in Dashboard browser:");
    println!("{authorize_url}");
    println!();
    println!("Copy data.code from the JSON response and paste it below.");

    let code = match args.code {
        Some(code) => code,
        None => prompt("Authorization code: ")?,
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
            scope: args.scope,
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
    println!("permissions: {}", identity.permissions.join(","));
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
    if !token.access_token.starts_with("YCLI.") {
        anyhow::bail!("backend returned a non-YCLI access token");
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
