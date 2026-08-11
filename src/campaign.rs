use std::{collections::HashSet, path::Path};

use anyhow::{Context, Result};
use chrono::{Days, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    cli::{
        CampaignAnalyticsArgs, CampaignExportArgs, CampaignListArgs, CampaignMessageFilterArgs,
        CampaignMessagesArgs,
    },
    config::Config,
    export,
    http::DashboardClient,
};

pub async fn list(
    client: &DashboardClient,
    config_path: &Path,
    args: CampaignListArgs,
) -> Result<()> {
    validate_optional_campaign_ids(&args.campaign_ids)?;
    let config = Config::load(config_path)?;
    let data = client
        .campaigns_search(
            &config.auth.access_token,
            &CampaignSearchRequest::from(args),
        )
        .await?
        .require_data("Campaign search")?;
    println!("{}", serde_json::to_string_pretty(&data)?);
    Ok(())
}

pub async fn messages(
    client: &DashboardClient,
    config_path: &Path,
    args: CampaignMessagesArgs,
) -> Result<()> {
    validate_filter(&args.filter)?;
    let config = Config::load(config_path)?;
    let response = client
        .campaign_messages_search(
            &config.auth.access_token,
            &CampaignMessageSearchRequest::from(args),
        )
        .await?;
    let pagination = response.pagination.as_ref().map(|value| {
        json!({
            "nextCursor": value.next_cursor,
            "hasMore": value.has_more,
            "total": value.total,
        })
    });
    let warnings = response
        .warnings
        .iter()
        .map(|value| json!({"code": value.code, "message": value.message}))
        .collect::<Vec<_>>();
    let data = response.require_data("Campaign message search")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "data": data,
            "pagination": pagination,
            "warnings": warnings,
        }))?
    );
    Ok(())
}

pub async fn analytics(
    client: &DashboardClient,
    config_path: &Path,
    args: CampaignAnalyticsArgs,
) -> Result<()> {
    validate_campaign_ids(&args.campaign_ids)?;
    validate_window(args.start_time, args.end_time, &args.timezone)?;
    if args.dimensions.is_empty() || args.dimensions.len() > 6 {
        anyhow::bail!("Campaign analytics requires 1 to 6 dimensions");
    }
    let config = Config::load(config_path)?;
    let data = client
        .campaign_analytics(
            &config.auth.access_token,
            &CampaignAnalyticsRequest::from(args),
        )
        .await?
        .require_data("Campaign analytics")?;
    println!("{}", serde_json::to_string_pretty(&data)?);
    Ok(())
}

pub async fn export_messages(
    client: &DashboardClient,
    config_path: &Path,
    args: CampaignExportArgs,
) -> Result<()> {
    validate_filter(&args.filter)?;
    let config = Config::load(config_path)?;
    let idempotency_key = args
        .idempotency_key
        .clone()
        .unwrap_or_else(export::new_idempotency_key);
    let request = CampaignMessageExportRequest::from(&args);
    let task = client
        .create_campaign_message_export(&config.auth.access_token, &idempotency_key, &request)
        .await?
        .require_data("Campaign message export")?;
    export::finish_created_task(client, &config, task, args.wait).await
}

fn validate_filter(filter: &CampaignMessageFilterArgs) -> Result<()> {
    validate_campaign_ids(&filter.campaign_ids)?;
    validate_window(filter.start_time, filter.end_time, &filter.timezone)
}

fn validate_optional_campaign_ids(values: &[String]) -> Result<()> {
    if values.is_empty() {
        return Ok(());
    }
    validate_campaign_ids(values)
}

fn validate_campaign_ids(values: &[String]) -> Result<()> {
    if values.is_empty() || values.len() > 100 {
        anyhow::bail!("Campaign commands require 1 to 100 Campaign IDs");
    }
    let mut unique = HashSet::with_capacity(values.len());
    for value in values {
        if value.trim().is_empty() || value.len() > 64 {
            anyhow::bail!("Campaign IDs must contain 1 to 64 non-whitespace characters");
        }
        if !unique.insert(value) {
            anyhow::bail!("Campaign IDs must be unique");
        }
    }
    Ok(())
}

fn validate_window(start_time: i64, end_time: i64, timezone: &str) -> Result<()> {
    if start_time < 0 || end_time <= start_time {
        anyhow::bail!("Campaign time range must satisfy 0 <= start-time < end-time");
    }
    let zone: Tz = timezone
        .parse()
        .with_context(|| format!("invalid IANA timezone: {timezone}"))?;
    let start = Utc
        .timestamp_millis_opt(start_time)
        .single()
        .context("Campaign start-time is outside the supported timestamp range")?;
    let end = Utc
        .timestamp_millis_opt(end_time)
        .single()
        .context("Campaign end-time is outside the supported timestamp range")?;
    let max_end = start
        .with_timezone(&zone)
        .checked_add_days(Days::new(90))
        .context("the 90-day boundary falls in a timezone transition; choose a nearby start-time")?
        .with_timezone(&Utc);
    if end > max_end {
        anyhow::bail!("Campaign time range may not exceed 90 natural days in {timezone}");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignSearchRequest {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub campaign_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub waba_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub business_phones: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub template_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time_gte: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time_lt: Option<i64>,
    pub page_no: u32,
    pub page_size: u32,
}

impl From<CampaignListArgs> for CampaignSearchRequest {
    fn from(value: CampaignListArgs) -> Self {
        Self {
            campaign_ids: value.campaign_ids,
            name: value.name,
            waba_ids: value.waba_ids,
            business_phones: value.business_phones,
            template_ids: value.template_ids,
            template_name: value.template_name,
            statuses: value.statuses,
            create_time_gte: value.create_time_gte,
            create_time_lt: value.create_time_lt,
            page_no: value.page_no,
            page_size: value.page_size,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignMessageSearchRequest {
    pub campaign_ids: Vec<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub timezone: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub template_names: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub business_phones: Vec<String>,
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl From<CampaignMessagesArgs> for CampaignMessageSearchRequest {
    fn from(value: CampaignMessagesArgs) -> Self {
        Self {
            campaign_ids: value.filter.campaign_ids,
            start_time: value.filter.start_time,
            end_time: value.filter.end_time,
            timezone: value.filter.timezone,
            statuses: value.filter.statuses,
            template_names: value.filter.template_names,
            business_phones: value.filter.business_phones,
            limit: value.limit,
            cursor: value.cursor,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignAnalyticsRequest {
    pub campaign_ids: Vec<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub timezone: String,
    pub dimensions: Vec<String>,
}

impl From<CampaignAnalyticsArgs> for CampaignAnalyticsRequest {
    fn from(value: CampaignAnalyticsArgs) -> Self {
        Self {
            campaign_ids: value.campaign_ids,
            start_time: value.start_time,
            end_time: value.end_time,
            timezone: value.timezone,
            dimensions: value
                .dimensions
                .into_iter()
                .map(|item| item.as_api_value().to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignMessageExportRequest {
    pub campaign_ids: Vec<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub timezone: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub template_names: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub business_phones: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

impl From<&CampaignExportArgs> for CampaignMessageExportRequest {
    fn from(value: &CampaignExportArgs) -> Self {
        Self {
            campaign_ids: value.filter.campaign_ids.clone(),
            start_time: value.filter.start_time,
            end_time: value.filter.end_time,
            timezone: value.filter.timezone.clone(),
            statuses: value.filter.statuses.clone(),
            template_names: value.filter.template_names.clone(),
            business_phones: value.filter.business_phones.clone(),
            file_name: value.file_name.clone(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignPage {
    pub page_no: u64,
    pub page_size: u64,
    pub total_count: u64,
    pub total_page: u64,
    #[serde(default)]
    pub records: Vec<Campaign>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Campaign {
    pub campaign_id: String,
    pub name: Option<String>,
    pub business_phone: Option<String>,
    pub waba_id: Option<String>,
    pub template_id: Option<String>,
    pub template_name: Option<String>,
    pub template_language: Option<String>,
    pub status: Option<i32>,
    pub create_time: Option<i64>,
    pub send_time: Option<i64>,
    pub end_time: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignMessage {
    pub message_id: String,
    pub campaign_id: String,
    pub waba_id: Option<String>,
    pub business_phone: Option<String>,
    pub recipient: Option<String>,
    pub message_type: Option<String>,
    pub template_name: Option<String>,
    pub template_language: Option<String>,
    pub status: Option<String>,
    pub error_code: Option<String>,
    pub create_time: Option<i64>,
    pub send_time: Option<i64>,
    pub deliver_time: Option<i64>,
    pub read_time: Option<i64>,
    pub fail_time: Option<i64>,
    pub click_count: Option<u64>,
    pub reply_count: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignAnalytics {
    pub window_start: i64,
    pub window_end: i64,
    pub timezone: String,
    #[serde(default)]
    pub data_points: Vec<CampaignAnalyticsDataPoint>,
    #[serde(default)]
    pub campaign_lifetime_metrics: Vec<CampaignLifetimeMetric>,
    #[serde(default)]
    pub availability: Vec<CampaignAnalyticsAvailability>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignLifetimeMetric {
    pub campaign_id: String,
    pub clicked: u64,
    pub scope: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignAnalyticsDataPoint {
    pub campaign_id: Option<String>,
    pub template_name: Option<String>,
    pub template_language: Option<String>,
    pub business_phone: Option<String>,
    pub date: Option<String>,
    pub status: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub total: Option<u64>,
    pub sent: Option<u64>,
    pub delivered: Option<u64>,
    pub read: Option<u64>,
    pub failed: Option<u64>,
    pub replied: Option<u64>,
    pub flow_replied: Option<u64>,
    pub converted: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignAnalyticsAvailability {
    pub source: String,
    pub available: bool,
    pub scope: Option<String>,
    pub fresh_through: Option<i64>,
    pub reason_code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_natural_days_across_dst() {
        let zone: Tz = "America/New_York".parse().unwrap();
        let start = zone
            .with_ymd_and_hms(2026, 8, 1, 12, 0, 0)
            .single()
            .unwrap();
        let end = start.checked_add_days(Days::new(90)).unwrap();

        validate_window(
            start.timestamp_millis(),
            end.timestamp_millis(),
            "America/New_York",
        )
        .unwrap();
        assert!(validate_window(
            start.timestamp_millis(),
            (end + chrono::Duration::milliseconds(1)).timestamp_millis(),
            "America/New_York",
        )
        .is_err());
    }

    #[test]
    fn rejects_duplicate_or_excessive_campaign_ids() {
        assert!(validate_campaign_ids(&["campaign-1".into(), "campaign-1".into()]).is_err());
        assert!(validate_campaign_ids(&vec!["campaign".into(); 101]).is_err());
    }

    #[test]
    fn serializes_message_filters_without_empty_arrays() {
        let request = CampaignMessageSearchRequest::from(CampaignMessagesArgs {
            filter: CampaignMessageFilterArgs {
                campaign_ids: vec!["campaign-1".into()],
                start_time: 1,
                end_time: 2,
                timezone: "UTC".into(),
                statuses: Vec::new(),
                template_names: Vec::new(),
                business_phones: Vec::new(),
            },
            limit: 100,
            cursor: None,
        });
        let value = serde_json::to_value(request).unwrap();

        assert_eq!(value["campaignIds"], json!(["campaign-1"]));
        assert!(value.get("statuses").is_none());
        assert!(value.get("cursor").is_none());
    }
}
