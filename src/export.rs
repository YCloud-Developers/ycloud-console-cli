use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use crate::{
    cli::{
        AnalyticsTemplatesExportArgs, ContactExportArgs, ConversationExportArgs,
        ConversationFilterArgs, ConversationSearchArgs, ExportArtifactType, ExportDownloadArgs,
        ExportRetryArgs, ExportTaskArgs, ExportWaitArgs,
    },
    config::Config,
    http::{AnalyticsTemplateExportRequest, DashboardClient, ExportTask},
};

pub async fn export_template_statistics(
    client: &DashboardClient,
    config_path: &Path,
    args: AnalyticsTemplatesExportArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let idempotency_key = args
        .idempotency_key
        .clone()
        .unwrap_or_else(new_idempotency_key);
    let request = AnalyticsTemplateExportRequest {
        analytics: crate::analytics::request(&args.filter)?,
        template_name: args.template_name,
        sort_field: args.sort_field.as_api_value().to_string(),
        sort_direction: args.sort_direction.as_api_value().to_string(),
        file_name: args.file_name,
    };
    let task = client
        .create_template_statistics_export(&config.auth.access_token, &idempotency_key, &request)
        .await?
        .require_data("template statistics export")?;
    finish_created_task(client, &config, task, args.wait).await
}

pub async fn search_conversations(
    client: &DashboardClient,
    config_path: &Path,
    args: ConversationSearchArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let response = client
        .conversations_search(
            &config.auth.access_token,
            &json!({
                "filter": conversation_filter(&args.filter),
                "cursor": args.cursor,
                "limit": args.limit,
            }),
        )
        .await?
        .require_data("conversation search")?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

pub async fn export_conversations(
    client: &DashboardClient,
    config_path: &Path,
    args: ConversationExportArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let idempotency_key = args
        .idempotency_key
        .clone()
        .unwrap_or_else(new_idempotency_key);
    let request = conversation_export_request(&args);
    let task = client
        .create_conversation_export(&config.auth.access_token, &idempotency_key, &request)
        .await?
        .require_data("conversation export")?;
    finish_created_task(client, &config, task, args.wait).await
}

fn conversation_export_request(args: &ConversationExportArgs) -> Value {
    json!({
        "filter": conversation_filter(&args.filter),
        "columns": args.columns,
        "format": args.format.as_api_value(),
        "includeContacts": !args.no_contacts,
        "includeMessages": true,
        "archive": args.archive,
        "timezone": args.timezone,
        "fileName": args.file_name,
    })
}

pub async fn export_contacts(
    client: &DashboardClient,
    config_path: &Path,
    args: ContactExportArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let idempotency_key = args
        .idempotency_key
        .clone()
        .unwrap_or_else(new_idempotency_key);
    let request = json!({
        "condition": args.condition,
        "segmentId": args.segment_id,
        "blocked": args.blocked,
        "columns": args.columns,
        "format": args.format.as_api_value(),
        "archive": args.archive,
        "timezone": args.timezone,
        "fileName": args.file_name,
    });
    let task = client
        .create_contact_export(&config.auth.access_token, &idempotency_key, &request)
        .await?
        .require_data("contact export")?;
    finish_created_task(client, &config, task, args.wait).await
}

pub async fn query_export(
    client: &DashboardClient,
    config_path: &Path,
    args: ExportTaskArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let task = client
        .query_export(&config.auth.access_token, &args.task_id)
        .await?
        .require_data("export query")?;
    print_task(&task)?;
    Ok(())
}

pub async fn retry_export(
    client: &DashboardClient,
    config_path: &Path,
    args: ExportRetryArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let idempotency_key = args
        .idempotency_key
        .clone()
        .unwrap_or_else(new_idempotency_key);
    let task = client
        .retry_export(&config.auth.access_token, &args.task_id, &idempotency_key)
        .await?
        .require_data("export retry")?;
    finish_created_task(client, &config, task, args.wait).await
}

pub async fn download_export(
    client: &DashboardClient,
    config_path: &Path,
    args: ExportDownloadArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let task = client
        .query_export(&config.auth.access_token, &args.task_id)
        .await?
        .require_data("export query")?;
    ensure_downloadable(&task)?;
    let downloads =
        download_artifacts(client, &config, &task, &args.artifacts, &args.output_dir).await?;
    validate_campaign_manifest(&task, &downloads)?;
    print_downloads(&task, &downloads, args.json)
}

pub(crate) async fn finish_created_task(
    client: &DashboardClient,
    config: &Config,
    task: ExportTask,
    wait: ExportWaitArgs,
) -> Result<()> {
    if wait.no_wait {
        print_task(&task)?;
        return Ok(());
    }
    let task = wait_for_task(
        client,
        &config.auth.access_token,
        &task.task_id,
        Duration::from_secs(wait.timeout_seconds),
    )
    .await?;
    ensure_downloadable(&task)?;
    let downloads =
        download_artifacts(client, config, &task, &wait.artifacts, &wait.output_dir).await?;
    validate_campaign_manifest(&task, &downloads)?;
    print_downloads(&task, &downloads, wait.json)
}

async fn wait_for_task(
    client: &DashboardClient,
    access_token: &str,
    task_id: &str,
    timeout: Duration,
) -> Result<ExportTask> {
    let deadline = Instant::now() + timeout;
    loop {
        let task = client
            .query_export(access_token, task_id)
            .await?
            .require_data("export query")?;
        if task.terminal() {
            return Ok(task);
        }
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for export task {task_id}");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn ensure_downloadable(task: &ExportTask) -> Result<()> {
    match task.status.to_ascii_uppercase().as_str() {
        "FINISHED" | "PARTIAL_SUCCESS" => Ok(()),
        "FAILED" => anyhow::bail!(
            "export task {} failed: {}",
            task.task_id,
            task.error_message
                .as_deref()
                .unwrap_or("unknown export failure")
        ),
        status => anyhow::bail!("export task {} is not ready: {status}", task.task_id),
    }
}

async fn download_artifacts(
    client: &DashboardClient,
    config: &Config,
    task: &ExportTask,
    selected: &[ExportArtifactType],
    output_dir: &Path,
) -> Result<Vec<DownloadedArtifact>> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let selected: Vec<&crate::http::ExportArtifact> = if selected.is_empty() {
        task.artifacts
            .iter()
            .filter(|artifact| artifact.status.eq_ignore_ascii_case("READY"))
            .filter(|artifact| !artifact.r#type.eq_ignore_ascii_case("ARCHIVE"))
            .collect()
    } else {
        let include_campaign_manifest = task.task_type.eq_ignore_ascii_case("CAMPAIGN_MESSAGE")
            && selected.contains(&ExportArtifactType::CampaignMessages);
        task.artifacts
            .iter()
            .filter(|artifact| artifact.status.eq_ignore_ascii_case("READY"))
            .filter(|artifact| {
                if include_campaign_manifest && artifact.r#type.eq_ignore_ascii_case("MANIFEST") {
                    return true;
                }
                selected.iter().any(|requested| {
                    requested
                        .as_api_value()
                        .eq_ignore_ascii_case(&artifact.r#type)
                })
            })
            .collect()
    };
    if selected.is_empty() {
        anyhow::bail!("export task {} has no ready artifacts", task.task_id);
    }

    let mut downloads = Vec::new();
    for artifact in selected {
        let metadata = client
            .export_artifact_url(
                &config.auth.access_token,
                &task.task_id,
                &artifact.r#type,
                artifact.artifact_id.as_deref(),
                artifact.part_number,
            )
            .await?
            .require_data("artifact URL")?;
        let safe_name = safe_file_name(&metadata.file_name)?;
        let destination = output_dir.join(safe_name);
        if destination.exists() {
            anyhow::bail!(
                "refusing to overwrite existing artifact {}",
                destination.display()
            );
        }
        let partial = partial_path(&destination);
        if partial.exists() {
            anyhow::bail!("partial artifact already exists: {}", partial.display());
        }
        let receipt = client.download_to_file(&metadata.url, &partial).await?;
        if let Some(expected) = metadata.size {
            if receipt.size != expected {
                let _ = fs::remove_file(&partial);
                anyhow::bail!(
                    "artifact size mismatch for {}: expected {}, got {}",
                    metadata.file_name,
                    expected,
                    receipt.size
                );
            }
        }
        if let Some(expected) = metadata.checksum_sha256.as_deref() {
            if !receipt.checksum_sha256.eq_ignore_ascii_case(expected) {
                let _ = fs::remove_file(&partial);
                anyhow::bail!("artifact checksum mismatch for {}", metadata.file_name);
            }
        }
        fs::rename(&partial, &destination)
            .with_context(|| format!("failed to finalize artifact {}", destination.display()))?;
        downloads.push(DownloadedArtifact {
            artifact_id: metadata.artifact_id,
            artifact_type: metadata.artifact_type,
            part_number: metadata.part_number,
            file_name: metadata.file_name,
            path: destination,
            size: receipt.size,
            checksum_sha256: receipt.checksum_sha256,
        });
    }
    Ok(downloads)
}

fn validate_campaign_manifest(task: &ExportTask, downloads: &[DownloadedArtifact]) -> Result<()> {
    if !task.task_type.eq_ignore_ascii_case("CAMPAIGN_MESSAGE")
        || !downloads
            .iter()
            .any(|item| item.artifact_type.eq_ignore_ascii_case("CAMPAIGN_MESSAGES"))
    {
        return Ok(());
    }
    let manifest = downloads
        .iter()
        .find(|item| item.artifact_type.eq_ignore_ascii_case("MANIFEST"))
        .context("Campaign message parts require the manifest artifact")?;
    let value: Value = serde_json::from_slice(
        &fs::read(&manifest.path)
            .with_context(|| format!("failed to read {}", manifest.path.display()))?,
    )
    .context("Campaign export manifest is not valid JSON")?;
    if value.get("taskId").and_then(Value::as_str) != Some(task.task_id.as_str())
        || value.get("taskType").and_then(Value::as_str) != Some("CAMPAIGN_MESSAGE")
        || value.get("status").and_then(Value::as_str) != Some("FINISHED")
    {
        anyhow::bail!("Campaign export manifest identity or terminal status does not match task");
    }
    let artifacts = value
        .get("artifacts")
        .and_then(Value::as_array)
        .context("Campaign export manifest is missing artifacts")?;
    let expected = artifacts
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("CAMPAIGN_MESSAGES"))
        .collect::<Vec<_>>();
    let actual = downloads
        .iter()
        .filter(|item| item.artifact_type.eq_ignore_ascii_case("CAMPAIGN_MESSAGES"))
        .map(|item| (item.part_number, item))
        .collect::<HashMap<_, _>>();
    if expected.len() != actual.len() {
        anyhow::bail!("Campaign export manifest part count does not match downloaded parts");
    }
    let mut part_numbers = BTreeSet::new();
    for item in &expected {
        let part_number = u32::try_from(
            item.get("partNumber")
                .and_then(Value::as_u64)
                .context("Campaign export manifest part is missing partNumber")?,
        )
        .context("Campaign export manifest partNumber is too large")?;
        if !part_numbers.insert(part_number) {
            anyhow::bail!("Campaign export manifest contains duplicate part numbers");
        }
        let downloaded = actual
            .get(&Some(part_number))
            .with_context(|| format!("Campaign export part {part_number} was not downloaded"))?;
        if item.get("artifactId").and_then(Value::as_str) != downloaded.artifact_id.as_deref()
            || item.get("fileName").and_then(Value::as_str) != Some(downloaded.file_name.as_str())
            || item.get("size").and_then(Value::as_u64) != Some(downloaded.size)
            || item.get("checksumSha256").and_then(Value::as_str)
                != Some(downloaded.checksum_sha256.as_str())
        {
            anyhow::bail!("Campaign export part {part_number} does not match the manifest");
        }
    }
    if part_numbers
        .iter()
        .copied()
        .ne(1..=(part_numbers.len() as u32))
    {
        anyhow::bail!("Campaign export manifest part numbers are not contiguous");
    }
    let manifest_parts = value
        .pointer("/counts/messageParts")
        .and_then(Value::as_u64)
        .context("Campaign export manifest is missing counts.messageParts")?;
    if manifest_parts != expected.len() as u64 {
        anyhow::bail!("Campaign export manifest messageParts count is inconsistent");
    }
    Ok(())
}

fn conversation_filter(args: &ConversationFilterArgs) -> Value {
    let mut filter = Map::new();
    insert_vec(&mut filter, "inboxIds", &args.inbox_ids);
    insert_vec(&mut filter, "assigneeIds", &args.assignee_ids);
    insert_vec(
        &mut filter,
        "conversationTagIds",
        &args.conversation_tag_ids,
    );
    insert_vec(&mut filter, "contactTagIds", &args.contact_tag_ids);
    filter.insert("statusScope".to_string(), json!(args.status.as_api_value()));
    insert_option(&mut filter, "condition", args.condition.clone());
    insert_option(&mut filter, "startTime", args.start_time);
    insert_option(&mut filter, "endTime", args.end_time);
    insert_option(&mut filter, "closeStartTime", args.close_start_time);
    insert_option(&mut filter, "closeEndTime", args.close_end_time);
    insert_option(&mut filter, "csat", args.csat);
    insert_option(&mut filter, "customer", args.customer);
    Value::Object(filter)
}

fn insert_vec(target: &mut Map<String, Value>, name: &str, value: &[String]) {
    if !value.is_empty() {
        target.insert(name.to_string(), json!(value));
    }
}

fn insert_option<T: serde::Serialize>(
    target: &mut Map<String, Value>,
    name: &str,
    value: Option<T>,
) {
    if let Some(value) = value {
        target.insert(name.to_string(), json!(value));
    }
}

fn safe_file_name(value: &str) -> Result<&str> {
    let invalid_character = value.chars().any(|character| {
        matches!(
            character,
            '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
        ) || character == '\0'
    });
    let stem = value.split('.').next().unwrap_or_default();
    let upper_stem = stem.to_ascii_uppercase();
    let windows_reserved = matches!(upper_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper_stem.len() == 4
            && matches!(&upper_stem[..3], "COM" | "LPT")
            && matches!(upper_stem.as_bytes()[3], b'1'..=b'9'));
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.ends_with(['.', ' '])
        || invalid_character
        || windows_reserved
    {
        anyhow::bail!("server returned an unsafe artifact file name");
    }
    Ok(value)
}

fn partial_path(destination: &Path) -> PathBuf {
    let mut value = destination.as_os_str().to_os_string();
    value.push(".part");
    PathBuf::from(value)
}

pub(crate) fn new_idempotency_key() -> String {
    format!("ycloud-cli-{:032x}", rand::random::<u128>())
}

fn print_task(task: &ExportTask) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(task)?);
    Ok(())
}

fn print_downloads(
    task: &ExportTask,
    downloads: &[DownloadedArtifact],
    json_output: bool,
) -> Result<()> {
    if task.status.eq_ignore_ascii_case("PARTIAL_SUCCESS") {
        eprintln!("warning: export completed with partial success; inspect task warnings");
    }
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "task": task,
                "downloads": downloads.iter().map(DownloadedArtifact::json).collect::<Vec<_>>()
            }))?
        );
    } else {
        println!("taskId: {}", task.task_id);
        println!("status: {}", task.status);
        if let Some(completeness) = &task.message_completeness {
            println!("messageCompleteness: {}", completeness);
        }
        for download in downloads {
            println!(
                "{}: {} ({} bytes, sha256={})",
                download.artifact_type,
                download.path.display(),
                download.size,
                download.checksum_sha256
            );
        }
    }
    Ok(())
}

struct DownloadedArtifact {
    artifact_id: Option<String>,
    artifact_type: String,
    part_number: Option<u32>,
    file_name: String,
    path: PathBuf,
    size: u64,
    checksum_sha256: String,
}

impl DownloadedArtifact {
    fn json(&self) -> Value {
        json!({
            "artifactId": self.artifact_id,
            "artifactType": self.artifact_type,
            "partNumber": self.part_number,
            "fileName": self.file_name,
            "path": self.path,
            "size": self.size,
            "checksumSha256": self.checksum_sha256,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal_from_server_file_name() {
        assert!(safe_file_name("../contacts.csv").is_err());
        assert!(safe_file_name("nested/contacts.csv").is_err());
        assert!(safe_file_name("nested\\contacts.csv").is_err());
        assert!(safe_file_name("contacts:1.csv").is_err());
        assert!(safe_file_name("CON").is_err());
        assert!(safe_file_name("com1.txt").is_err());
        assert!(safe_file_name("contacts.csv.").is_err());
        assert_eq!(safe_file_name("contacts.csv").unwrap(), "contacts.csv");
    }

    #[test]
    fn filter_omits_empty_values() {
        let value = conversation_filter(&ConversationFilterArgs {
            condition: Some("vip".to_string()),
            ..ConversationFilterArgs::default()
        });
        assert_eq!(value, json!({"condition": "vip", "statusScope": "ALL"}));
    }

    #[test]
    fn conversation_filter_sends_explicit_status_scope() {
        let value = conversation_filter(&ConversationFilterArgs {
            status: crate::cli::ConversationStatus::Open,
            ..ConversationFilterArgs::default()
        });

        assert_eq!(value, json!({"statusScope": "OPEN"}));
        assert!(value.get("status").is_none());
    }

    #[test]
    fn conversation_export_always_requests_messages() {
        let request = conversation_export_request(&ConversationExportArgs {
            filter: ConversationFilterArgs::default(),
            columns: Vec::new(),
            format: crate::cli::ExportFormat::Auto,
            no_contacts: false,
            archive: false,
            timezone: "GMT".to_string(),
            file_name: None,
            idempotency_key: None,
            wait: ExportWaitArgs {
                no_wait: true,
                output_dir: PathBuf::from("."),
                artifacts: Vec::new(),
                timeout_seconds: 3600,
                json: false,
            },
        });

        assert_eq!(request["includeMessages"], json!(true));
        assert_eq!(request["includeContacts"], json!(true));
    }

    #[test]
    fn campaign_manifest_requires_contiguous_matching_parts() {
        let directory = tempfile::tempdir().unwrap();
        let manifest_path = directory.path().join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec(&json!({
                "taskId": "task-1",
                "taskType": "CAMPAIGN_MESSAGE",
                "status": "FINISHED",
                "counts": {"messageParts": 1},
                "artifacts": [{
                    "artifactId": "campaign-messages-1",
                    "type": "CAMPAIGN_MESSAGES",
                    "partNumber": 1,
                    "fileName": "campaign-messages-00001.jsonl",
                    "size": 10,
                    "checksumSha256": "abc"
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let task = ExportTask {
            task_id: "task-1".into(),
            task_type: "CAMPAIGN_MESSAGE".into(),
            status: "FINISHED".into(),
            progress: 100,
            record_count: Some(1),
            previous_task_id: None,
            truncated: None,
            truncation_reason: None,
            message_completeness: None,
            artifacts: Vec::new(),
            warnings: Vec::new(),
            error_code: None,
            error_message: None,
        };
        let downloads = vec![
            DownloadedArtifact {
                artifact_id: Some("campaign-messages-1".into()),
                artifact_type: "CAMPAIGN_MESSAGES".into(),
                part_number: Some(1),
                file_name: "campaign-messages-00001.jsonl".into(),
                path: directory.path().join("campaign-messages-00001.jsonl"),
                size: 10,
                checksum_sha256: "abc".into(),
            },
            DownloadedArtifact {
                artifact_id: Some("manifest".into()),
                artifact_type: "MANIFEST".into(),
                part_number: None,
                file_name: "manifest.json".into(),
                path: manifest_path,
                size: 1,
                checksum_sha256: "manifest-checksum".into(),
            },
        ];

        validate_campaign_manifest(&task, &downloads).unwrap();
    }
}
