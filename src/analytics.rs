use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    cli::{AnalyticsFilterArgs, AnalyticsQueryArgs, AnalyticsTemplatesListArgs},
    config::Config,
    http::{
        millis_to_rfc3339, AnalyticsMetric, AnalyticsRequest, AnalyticsSummary,
        AnalyticsTemplatePage, AnalyticsTemplateSearchRequest, AnalyticsTrend, DashboardApiError,
        DashboardClient,
    },
};

pub async fn overview(
    client: &DashboardClient,
    config_path: &Path,
    args: AnalyticsQueryArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let request = request(&args.filter)?;
    let templates_request = AnalyticsTemplateSearchRequest {
        analytics: request.clone(),
        template_name: None,
        page_no: 1,
        page_size: 10,
        sort_field: "total".to_string(),
        sort_direction: "DESC".to_string(),
    };
    let token = &config.auth.access_token;
    let (summary, trend, templates) = tokio::join!(
        async {
            client
                .whatsapp_delivery_analytics(token, &request)
                .await?
                .require_data("analytics summary")
        },
        async {
            client
                .whatsapp_message_detail(token, &request)
                .await?
                .require_data("analytics trend")
        },
        async {
            client
                .whatsapp_template_statistics(token, &templates_request)
                .await?
                .require_data("template statistics")
        },
    );
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "range": request,
                "summary": section(&summary),
                "trend": section(&trend),
                "templates": section(&templates),
            }))?
        );
    } else {
        println!("Analytics overview (operational analytics; not for billing reconciliation)");
        print_section("Message type summary", &summary, print_summary)?;
        print_section("Volume trend", &trend, print_trend)?;
        print_section(
            "Template statistics (first 10)",
            &templates,
            print_templates,
        )?;
    }
    summary?;
    trend?;
    templates?;
    Ok(())
}

pub async fn trend(
    client: &DashboardClient,
    config_path: &Path,
    args: AnalyticsQueryArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let request = request(&args.filter)?;
    let data = client
        .whatsapp_message_detail(&config.auth.access_token, &request)
        .await?
        .require_data("analytics trend")?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&data)?);
    } else {
        print_trend(&data)?;
    }
    Ok(())
}

pub async fn templates_list(
    client: &DashboardClient,
    config_path: &Path,
    args: AnalyticsTemplatesListArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let request = AnalyticsTemplateSearchRequest {
        analytics: request(&args.filter)?,
        template_name: args.template_name,
        page_no: args.page_no,
        page_size: 10,
        sort_field: args.sort_field.as_api_value().to_string(),
        sort_direction: args.sort_direction.as_api_value().to_string(),
    };
    let data = client
        .whatsapp_template_statistics(&config.auth.access_token, &request)
        .await?
        .require_data("template statistics")?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&data)?);
    } else {
        print_templates(&data)?;
        println!(
            "page {} ({} records per page, {} total)",
            data.page_no, data.page_size, data.total_elements
        );
    }
    Ok(())
}

pub(crate) fn request(args: &AnalyticsFilterArgs) -> Result<AnalyticsRequest> {
    if args.preset.is_some() && (args.range.start_time.is_some() || args.range.end_time.is_some()) {
        anyhow::bail!("preset cannot be combined with start-time or end-time");
    }
    let end_time = args.range.end_time.unwrap_or(now_millis()?);
    let days = args.preset.map(|value| value.days()).unwrap_or(7);
    let start_time = args
        .range
        .start_time
        .unwrap_or(end_time - Duration::from_secs((days * 24 * 60 * 60) as u64).as_millis() as i64);
    if start_time >= end_time {
        anyhow::bail!("start-time must be earlier than end-time");
    }
    if end_time - start_time > Duration::from_secs(90 * 24 * 60 * 60).as_millis() as i64 {
        anyhow::bail!("analytics time range must not exceed 90 days");
    }
    Ok(AnalyticsRequest {
        start_time: millis_to_rfc3339(start_time)?,
        end_time: millis_to_rfc3339(end_time)?,
        timezone: args.timezone.clone(),
        froms: distinct(&args.froms),
        region_codes: distinct(&args.region_codes),
        message_types: args
            .message_types
            .iter()
            .map(|value| value.as_api_value().to_string())
            .collect(),
        waba_ids: distinct(&args.waba_ids),
    })
}

fn now_millis() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_millis() as i64)
}

fn distinct(values: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for value in values {
        if !result.contains(value) {
            result.push(value.clone());
        }
    }
    result
}

fn section<T: Serialize>(result: &Result<T>) -> Value {
    match result {
        Ok(data) => json!({"status": "ok", "data": data}),
        Err(error) => json!({"status": "error", "error": section_error(error)}),
    }
}

fn section_error(error: &anyhow::Error) -> Value {
    if let Some(api) = error.downcast_ref::<DashboardApiError>() {
        return json!({
            "code": api.code(),
            "message": api.message(),
            "retryable": api.retryable(),
            "requestId": api.request_id(),
            "traceId": api.trace_id(),
        });
    }
    json!({
        "code": if error.to_string().contains("timed out") { "section_timeout" } else { "request_failed" },
        "message": error.to_string(),
        "retryable": true,
        "requestId": Value::Null,
        "traceId": Value::Null,
    })
}

fn print_section<T>(title: &str, result: &Result<T>, render: fn(&T) -> Result<()>) -> Result<()> {
    println!("\n{title}");
    match result {
        Ok(data) => render(data),
        Err(error) => {
            let detail = section_error(error);
            println!(
                "ERROR {} (retryable={}, requestId={}, traceId={}): {}",
                detail["code"].as_str().unwrap_or("request_failed"),
                detail["retryable"].as_bool().unwrap_or(false),
                detail["requestId"].as_str().unwrap_or("-"),
                detail["traceId"].as_str().unwrap_or("-"),
                detail["message"].as_str().unwrap_or("request failed")
            );
            Ok(())
        }
    }
}

fn print_summary(data: &AnalyticsSummary) -> Result<()> {
    let rows = data.message_types.iter().map(metric_row).collect();
    print_table(
        &["Message type", "Total", "Delivered", "Delivery rate"],
        rows,
    );
    Ok(())
}

fn metric_row(metric: &AnalyticsMetric) -> Vec<String> {
    vec![
        metric.message_type.clone(),
        number(metric.total),
        number(metric.delivered),
        rate(metric.delivery_rate),
    ]
}

fn print_trend(data: &AnalyticsTrend) -> Result<()> {
    println!("timezone: {}", data.timezone);
    println!(
        "granularity: {}",
        data.granularity.as_deref().unwrap_or("-")
    );
    let mut rows = Vec::new();
    for bucket in &data.buckets {
        let period = format!(
            "{}..{}",
            millis_to_rfc3339(bucket.start_time)?,
            millis_to_rfc3339(bucket.end_time)?
        );
        rows.push(vec![
            period.clone(),
            "overall".to_string(),
            number(bucket.total),
            number(bucket.delivered),
            number(bucket.failed),
            number(bucket.processing),
            rate(bucket.delivery_rate),
        ]);
        for metric in &bucket.message_types {
            rows.push(vec![
                period.clone(),
                metric.message_type.clone(),
                number(metric.total),
                number(metric.delivered),
                "-".to_string(),
                "-".to_string(),
                rate(metric.delivery_rate),
            ]);
        }
    }
    print_table(
        &[
            "Bucket",
            "Message type",
            "Total",
            "Delivered",
            "Failed",
            "Processing",
            "Delivery rate",
        ],
        rows,
    );
    Ok(())
}

fn print_templates(data: &AnalyticsTemplatePage) -> Result<()> {
    let rows = data
        .records
        .iter()
        .map(|record| {
            vec![
                record.waba_id.clone().unwrap_or_else(|| "-".to_string()),
                record.template_name.clone(),
                record
                    .language_code
                    .clone()
                    .unwrap_or_else(|| "-".to_string()),
                record.category.clone().unwrap_or_else(|| "-".to_string()),
                number(record.total),
                number(record.delivered),
                number(record.failed),
                rate(record.delivery_rate),
                record
                    .top_failure_reason
                    .as_ref()
                    .and_then(|value| value.error_code.clone())
                    .unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    print_table(
        &[
            "WABA",
            "Template",
            "Language",
            "Category",
            "Total",
            "Delivered",
            "Failed",
            "Delivery rate",
            "Top failure",
        ],
        rows,
    );
    Ok(())
}

fn number(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn rate(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}%"))
        .unwrap_or_else(|| "-".to_string())
}

fn print_table(headers: &[&str], rows: Vec<Vec<String>>) {
    let mut widths: Vec<usize> = headers.iter().map(|value| value.len()).collect();
    for row in &rows {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(value.len());
        }
    }
    println!(
        "{}",
        headers
            .iter()
            .enumerate()
            .map(|(index, value)| format!("{value:<width$}", width = widths[index]))
            .collect::<Vec<_>>()
            .join("  ")
    );
    println!(
        "{}",
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("  ")
    );
    for row in rows {
        println!(
            "{}",
            row.iter()
                .enumerate()
                .map(|(index, value)| format!("{value:<width$}", width = widths[index]))
                .collect::<Vec<_>>()
                .join("  ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{AnalyticsMessageType, AnalyticsRangeArgs};

    fn args() -> AnalyticsFilterArgs {
        AnalyticsFilterArgs {
            range: AnalyticsRangeArgs {
                start_time: Some(0),
                end_time: Some(1_000),
            },
            preset: None,
            timezone: "UTC".to_string(),
            froms: vec!["1".to_string(), "1".to_string()],
            region_codes: vec!["US".to_string()],
            message_types: vec![AnalyticsMessageType::Utility],
            waba_ids: vec!["waba-1".to_string()],
        }
    }

    #[test]
    fn request_uses_repeatable_new_filters() {
        let request = request(&args()).unwrap();
        assert_eq!(request.froms, vec!["1"]);
        assert_eq!(request.message_types, vec!["utility"]);
        assert_eq!(request.waba_ids, vec!["waba-1"]);
    }

    #[test]
    fn rejects_more_than_ninety_days() {
        let mut args = args();
        args.range.end_time = Some(90 * 24 * 60 * 60 * 1_000 + 1);
        assert!(request(&args).unwrap_err().to_string().contains("90 days"));
    }
}
