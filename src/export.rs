use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use crate::{
    cli::{
        ContactExportArgs, ConversationExportArgs, ConversationFilterArgs, ConversationSearchArgs,
        ExportArtifactType, ExportDownloadArgs, ExportRetryArgs, ExportTaskArgs, ExportWaitArgs,
    },
    config::Config,
    http::{DashboardClient, ExportTask},
};

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
    print_downloads(&task, &downloads, args.json)
}

async fn finish_created_task(
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
        task.artifacts
            .iter()
            .filter(|artifact| artifact.status.eq_ignore_ascii_case("READY"))
            .filter(|artifact| {
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
            path: destination,
            size: receipt.size,
            checksum_sha256: receipt.checksum_sha256,
        });
    }
    Ok(downloads)
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
    insert_option(&mut filter, "status", args.status);
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
    let path = Path::new(value);
    if path.file_name().and_then(|name| name.to_str()) != Some(value) || value.is_empty() {
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
        assert_eq!(safe_file_name("contacts.csv").unwrap(), "contacts.csv");
    }

    #[test]
    fn filter_omits_empty_values() {
        let value = conversation_filter(&ConversationFilterArgs {
            condition: Some("vip".to_string()),
            ..ConversationFilterArgs::default()
        });
        assert_eq!(value, json!({"condition": "vip"}));
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
}
