use std::{
    path::Path,
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use chrono::DateTime;

use crate::{
    cli::{
        AnalyticsCallingLogsArgs, AnalyticsLogsArgs, AnalyticsRangeArgs, ContactStatsArgs,
        ContactsListArgs, LoginArgs, PermissionProfile,
    },
    config::Config,
    http::{
        AnalyticsCallingLogsRequest, AnalyticsLogsRequest, AnalyticsRangeRequest,
        ContactStatisticsRequest, ContactsSearchRequest, DashboardClient,
    },
};

pub async fn login(client: &DashboardClient, config_path: &Path, args: LoginArgs) -> Result<()> {
    let authentication_client = client.authentication_client();
    let client = &authentication_client;
    crate::oauth::login(client, config_path, args).await
}

pub async fn reauthorize(
    client: &DashboardClient,
    config_path: &Path,
    requested_permissions: Vec<String>,
) -> Result<()> {
    let current = Config::load(config_path)?;
    login(
        client,
        config_path,
        LoginArgs {
            oauth: true,
            oauth_issuer: current.auth.oauth.as_ref().map(|s| s.issuer.clone()),
            oauth_client_id: current.auth.oauth.as_ref().map(|s| s.client_id.clone()),
            oauth_resource: current.auth.oauth.as_ref().map(|s| s.resource.clone()),
            no_browser: false,
            profile: PermissionProfile::Custom,
            permissions: requested_permissions,
        },
    )
    .await
}

pub(crate) fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .status();
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = Command::new("xdg-open").arg(url).status();

    let status = status.context("failed to start browser command")?;
    if !status.success() {
        anyhow::bail!("browser command exited with status {status}");
    }
    Ok(())
}

pub async fn whoami(client: &DashboardClient, config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let identity = client
        .whoami(&config.auth.access_token)
        .await?
        .require_data("whoami")?;
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

pub async fn contact_statistics(
    client: &DashboardClient,
    config_path: &Path,
    args: ContactStatsArgs,
) -> Result<()> {
    let start = DateTime::parse_from_rfc3339(&args.start_time)
        .context("start-time must use RFC 3339 date-time format")?;
    let end = DateTime::parse_from_rfc3339(&args.end_time)
        .context("end-time must use RFC 3339 date-time format")?;
    if start >= end {
        anyhow::bail!("start-time must be earlier than end-time");
    }
    if end.signed_duration_since(start).num_milliseconds() > 90 * 24 * 60 * 60 * 1_000 {
        anyhow::bail!("contact statistics time range must not exceed 90 days");
    }

    let config = Config::load(config_path)?;
    let statistics = client
        .contact_statistics(
            &config.auth.access_token,
            ContactStatisticsRequest {
                start_time: &args.start_time,
                end_time: &args.end_time,
                sources: &args.sources,
                group_by: args.group_by.map(|value| value.as_api_value()),
            },
        )
        .await?
        .require_data("contacts/statistics")?;
    println!("{}", serde_json::to_string_pretty(&statistics)?);
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
    let config = Config::load(config_path)?;
    crate::oauth::refresh(client, config_path, config).await
}

pub async fn logout(client: &DashboardClient, config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;
    crate::oauth::require_oauth_session(&config)?;
    crate::oauth::logout(client, config_path, config).await
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
