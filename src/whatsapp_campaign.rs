use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::{cli::CampaignCommand, config::Config, http::DashboardClient};

const ROOT: &str = "/api/cli/v1/whatsapp";

#[derive(Debug, Clone, Args)]
pub struct InputArgs {
    /// JSON file with the business fields from the CLI API contract.
    #[arg(long)]
    pub input: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct WriteArgs {
    #[command(flatten)]
    pub input: InputArgs,
    /// Show the preview and prompt for explicit confirmation.
    #[arg(long)]
    pub apply: bool,
}

#[derive(Debug, Clone, Args)]
pub struct CampaignWriteArgs {
    #[arg(long)]
    pub campaign_id: String,
    #[command(flatten)]
    pub write: WriteArgs,
}

#[derive(Debug, Clone, Args)]
pub struct CampaignStatusArgs {
    #[arg(long)]
    pub campaign_id: String,
    #[arg(long)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum TemplateCommand {
    List(TemplateList),
    Get(TemplateGet),
    Options(TemplateOptions),
    Library(TemplateLibrary),
    Prepare(WriteArgs),
    Submit(TemplateSubmit),
}

#[derive(Debug, Clone, Args)]
pub struct TemplateList {
    #[arg(long)]
    waba_id: String,
    #[arg(long)]
    purpose: Option<String>,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    language: Option<String>,
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Args)]
pub struct TemplateGet {
    #[arg(long)]
    waba_id: String,
    #[arg(
        long,
        conflicts_with = "operation_id",
        required_unless_present = "operation_id"
    )]
    template_id: Option<String>,
    #[arg(long, required_unless_present = "template_id")]
    operation_id: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct TemplateOptions {
    #[arg(long)]
    waba_id: Option<String>,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    language: Option<String>,
    #[arg(long)]
    library_ref: Option<String>,
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Args)]
pub struct TemplateLibrary {
    #[arg(long)]
    language: String,
    #[arg(long)]
    search: Option<String>,
    #[arg(long)]
    topic: Option<String>,
    #[arg(long)]
    use_case: Option<String>,
    #[arg(long)]
    industry: Option<String>,
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Args)]
pub struct TemplateSubmit {
    /// WABA used to query the original operation if its response is lost.
    #[arg(long)]
    waba_id: String,
    #[command(flatten)]
    write: WriteArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AssetCommand {
    Get {
        #[arg(long)]
        asset_id: String,
    },
    Upload(AssetUpload),
    /// Upload one file using an MCP-approved pairing, without a CLI login.
    PairUpload(crate::paired_upload::PairUpload),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AssetPurpose {
    Audience,
    CampaignMedia,
    TemplateSample,
}

impl AssetPurpose {
    fn wire(self) -> &'static str {
        match self {
            Self::Audience => "AUDIENCE",
            Self::CampaignMedia => "CAMPAIGN_MEDIA",
            Self::TemplateSample => "TEMPLATE_SAMPLE",
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct AssetUpload {
    #[arg(long)]
    file: PathBuf,
    #[arg(long, value_enum)]
    purpose: AssetPurpose,
    #[arg(long)]
    content_type: String,
    #[arg(long)]
    waba_id: Option<String>,
    #[arg(long)]
    apply: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preview {
    phase: String,
    preview_id: String,
    confirmation_token: String,
    resource_version: String,
    expires_at: i64,
    campaign_id: Option<String>,
    planned_change: Vec<PlannedChange>,
    risk: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannedChange {
    action: String,
    resource_type: String,
    summary: String,
    resource_id: Option<String>,
    recipient_count: Option<u64>,
    scheduled_for: Option<i64>,
    template_fingerprint: Option<String>,
    source_fingerprint: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Execution<'a> {
    confirmation_token: &'a str,
    resource_version: &'a str,
    idempotency_key: &'a str,
}

#[derive(Serialize)]
struct Execute<'a> {
    phase: &'static str,
    execution: Execution<'a>,
}

enum ReceiptLookup<'a> {
    Campaign(Option<&'a str>),
    Template(&'a str),
    Asset,
}

pub async fn campaign(
    client: &DashboardClient,
    config: &Path,
    command: &CampaignCommand,
) -> Result<()> {
    let config = Config::load(config)?;
    let token = &config.auth.access_token;
    let campaigns = format!("{ROOT}/campaigns");
    match command {
        CampaignCommand::Get(args) => print(
            read(
                client,
                token,
                &campaign_status(&args.campaign_id, args.operation_id.as_deref())?,
            )
            .await?,
        ),
        CampaignCommand::Estimate(args) => {
            let value = input(args).await?;
            print(
                client
                    .campaign_post(&format!("{campaigns}/audience-estimate"), token, &value)
                    .await?
                    .require_data("Audience estimate")?,
            )
        }
        CampaignCommand::Prepare(args) => {
            let body = input(&args.input).await?;
            let lookup = ReceiptLookup::Campaign(body.get("campaignId").and_then(Value::as_str));
            print(
                write(
                    client,
                    token,
                    &format!("{campaigns}/prepare"),
                    body.clone(),
                    args.apply,
                    lookup,
                )
                .await?,
            )
        }
        CampaignCommand::Test(args) => campaign_write(client, token, args, "test").await,
        CampaignCommand::Submit(args) => campaign_write(client, token, args, "submit").await,
        CampaignCommand::Cancel(args) => campaign_write(client, token, args, "cancel").await,
        _ => bail!("Unsupported Campaign command"),
    }
}

async fn campaign_write(
    client: &DashboardClient,
    token: &str,
    args: &CampaignWriteArgs,
    action: &str,
) -> Result<()> {
    let path = format!("{ROOT}/campaigns/{}/{action}", id(&args.campaign_id)?);
    let body = input(&args.write.input).await?;
    print(
        write(
            client,
            token,
            &path,
            body,
            args.write.apply,
            ReceiptLookup::Campaign(Some(&args.campaign_id)),
        )
        .await?,
    )
}

pub async fn template(
    client: &DashboardClient,
    config: &Path,
    command: &TemplateCommand,
) -> Result<()> {
    let config = Config::load(config)?;
    let token = &config.auth.access_token;
    match command {
        TemplateCommand::Get(args) => {
            let (kind, value) = match (&args.template_id, &args.operation_id) {
                (Some(value), None) => ("TEMPLATE", value),
                (None, Some(value)) => ("OPERATION", value),
                _ => bail!("Select one template or operation"),
            };
            print(read(client, token, &template_detail(&args.waba_id, kind, value)?).await?)
        }
        TemplateCommand::List(args) => print(
            read(
                client,
                token,
                &query(
                    "templates",
                    &[
                        ("wabaId", Some(args.waba_id.clone())),
                        ("purpose", args.purpose.clone()),
                        ("name", args.name.clone()),
                        ("language", args.language.clone()),
                        ("status", args.status.clone()),
                        ("cursor", args.cursor.clone()),
                        ("limit", args.limit.map(|value| value.to_string())),
                    ],
                ),
            )
            .await?,
        ),
        TemplateCommand::Options(args) => print(
            read(
                client,
                token,
                &query(
                    "template-creation-options",
                    &[
                        ("wabaId", args.waba_id.clone()),
                        ("category", args.category.clone()),
                        ("language", args.language.clone()),
                        ("libraryRef", args.library_ref.clone()),
                        ("cursor", args.cursor.clone()),
                        ("limit", args.limit.map(|value| value.to_string())),
                    ],
                ),
            )
            .await?,
        ),
        TemplateCommand::Library(args) => print(
            read(
                client,
                token,
                &query(
                    "template-library",
                    &[
                        ("language", Some(args.language.clone())),
                        ("search", args.search.clone()),
                        ("topic", args.topic.clone()),
                        ("useCase", args.use_case.clone()),
                        ("industry", args.industry.clone()),
                        ("cursor", args.cursor.clone()),
                        ("limit", args.limit.map(|value| value.to_string())),
                    ],
                ),
            )
            .await?,
        ),
        TemplateCommand::Prepare(args) => {
            let body = input(&args.input).await?;
            let waba = body
                .get("wabaId")
                .and_then(Value::as_str)
                .context("wabaId is required")?
                .to_owned();
            print(
                write(
                    client,
                    token,
                    &format!("{ROOT}/templates/prepare"),
                    body,
                    args.apply,
                    ReceiptLookup::Template(&waba),
                )
                .await?,
            )
        }
        TemplateCommand::Submit(args) => print(
            write(
                client,
                token,
                &format!("{ROOT}/templates/submit"),
                input(&args.write.input).await?,
                args.write.apply,
                ReceiptLookup::Template(&args.waba_id),
            )
            .await?,
        ),
    }
}

pub async fn asset(client: &DashboardClient, config: &Path, command: &AssetCommand) -> Result<()> {
    let config = Config::load(config)?;
    let token = &config.auth.access_token;
    match command {
        AssetCommand::Get { asset_id } => {
            print(read(client, token, &format!("{ROOT}/assets/{}", id(asset_id)?)).await?)
        }
        AssetCommand::Upload(args) => upload(client, token, args).await,
        AssetCommand::PairUpload(_) => bail!("Pair upload must be invoked without a CLI session"),
    }
}

async fn upload(client: &DashboardClient, token: &str, args: &AssetUpload) -> Result<()> {
    let mut file = tokio::fs::File::open(&args.file)
        .await
        .context("Cannot open attachment")?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 100 * 1024 * 1024 {
        bail!("Expected a non-empty regular attachment of at most 100 MiB");
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 65536];
    let mut size = 0;
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > metadata.len() {
            bail!("Attachment changed while calculating its digest");
        }
        digest.update(&buffer[..count]);
    }
    if size != metadata.len() {
        bail!("Attachment changed while calculating its digest");
    }
    file.seek(std::io::SeekFrom::Start(0)).await?;
    let file_name = args
        .file
        .file_name()
        .and_then(|value| value.to_str())
        .context("Expected a UTF-8 file name")?;
    let mut body = json!({
        "purpose": args.purpose.wire(), "fileName": file_name, "contentType": args.content_type,
        "sizeBytes": size, "sha256": format!("{:x}", digest.finalize())
    })
    .as_object()
    .unwrap()
    .clone();
    if let Some(waba) = &args.waba_id {
        body.insert("wabaId".into(), json!(waba));
    }

    let result = write(
        client,
        token,
        &format!("{ROOT}/assets"),
        body,
        args.apply,
        ReceiptLookup::Asset,
    )
    .await?;
    if let Some(target) = result.get("upload") {
        let asset_id = result
            .get("assetId")
            .and_then(Value::as_str)
            .context("Missing asset receipt")?;
        let status_path = format!("{ROOT}/assets/{}", id(asset_id)?);
        let url = target
            .get("url")
            .and_then(Value::as_str)
            .context("Missing upload destination")?;
        if target.get("method").and_then(Value::as_str) != Some("PUT") {
            bail!("Unsupported attachment transfer method");
        }
        let response = client
            .campaign_upload(&format!("{status_path}/content"), url, token, file, size)
            .await;
        return match response {
            Ok(envelope) => print(envelope.require_data("Attachment transfer")?),
            Err(error) => recovered(client, token, &status_path, error).await,
        };
    }
    print(result)
}

async fn input(args: &InputArgs) -> Result<Map<String, Value>> {
    let file = tokio::fs::File::open(&args.input)
        .await
        .context("Cannot open the JSON input file")?;
    let mut bytes = Vec::new();
    file.take(131_073).read_to_end(&mut bytes).await?;
    if bytes.len() > 131_072 {
        bail!("JSON input exceeds 128 KiB");
    }
    let value: Map<String, Value> = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Expected a valid JSON business object"))?;
    if [
        "phase",
        "execution",
        "confirmationToken",
        "idempotencyKey",
        "previewId",
    ]
    .iter()
    .any(|key| value.contains_key(*key))
    {
        bail!("Confirmation fields are supplied by the CLI");
    }
    Ok(value)
}

async fn write(
    client: &DashboardClient,
    token: &str,
    path: &str,
    mut input: Map<String, Value>,
    apply: bool,
    lookup: ReceiptLookup<'_>,
) -> Result<Value> {
    input.insert("phase".into(), json!("PREVIEW"));
    let preview: Preview = client
        .campaign_post(path, token, &input)
        .await?
        .require_data("Write preview")?;
    if preview.phase != "PREVIEW"
        || preview.planned_change.is_empty()
        || id(&preview.preview_id).is_err()
    {
        bail!("Invalid write preview");
    }
    let lookup_path = match lookup {
        ReceiptLookup::Campaign(known) => known
            .or(preview.campaign_id.as_deref())
            .map(|value| campaign_status(value, Some(&preview.preview_id)))
            .transpose()?,
        ReceiptLookup::Template(waba) => {
            Some(template_detail(waba, "OPERATION", &preview.preview_id)?)
        }
        ReceiptLookup::Asset => Some(format!("{ROOT}/assets/{}", id(&preview.preview_id)?)),
    };
    let visible = json!({
        "phase": preview.phase, "previewId": preview.preview_id, "campaignId": preview.campaign_id,
        "risk": preview.risk, "expiresAt": preview.expires_at, "resourceVersion": preview.resource_version,
        "plannedChange": preview.planned_change,
    });
    if !apply {
        return Ok(visible);
    }
    eprintln!("{}", serde_json::to_string_pretty(&visible)?);
    eprint!("Type the preview ID to confirm: ");
    io::stderr().flush()?;
    let mut entered = String::new();
    io::stdin().read_line(&mut entered)?;
    if entered.trim() != preview.preview_id {
        bail!("Confirmation cancelled");
    }
    if chrono::Utc::now().timestamp_millis() >= preview.expires_at {
        bail!("Preview expired. Request a new preview.");
    }
    let key = crate::http::random_identifier("campaign");
    let execute = Execute {
        phase: "EXECUTE",
        execution: Execution {
            confirmation_token: &preview.confirmation_token,
            resource_version: &preview.resource_version,
            idempotency_key: &key,
        },
    };
    match client
        .campaign_post::<Value, _>(path, token, &execute)
        .await
        .and_then(|response| response.require_data("Write receipt"))
    {
        Ok(result) => Ok(result),
        Err(error) => {
            if let Some(status) = lookup_path {
                if let Ok(value) = read(client, token, &status).await {
                    if accepted_receipt(&value) {
                        return Ok(
                            json!({"recovered": true, "previewId": preview.preview_id, "receipt": value}),
                        );
                    }
                }
            }
            Err(error.context(format!(
                "Execute was not repeated. Query operation {}",
                preview.preview_id
            )))
        }
    }
}

fn accepted_receipt(receipt: &Value) -> bool {
    let state = receipt
        .pointer("/selectedOperation/state")
        .or_else(|| receipt.get("operationState"))
        .or_else(|| receipt.get("state"))
        .and_then(Value::as_str);
    matches!(
        state,
        Some(
            "QUEUED"
                | "RUNNING"
                | "CALLING"
                | "SUCCEEDED"
                | "UNKNOWN"
                | "RECEIVING"
                | "PARSING"
                | "READY"
        )
    ) || receipt
        .pointer("/selectedOperation/acceptedAt")
        .and_then(Value::as_i64)
        .is_some()
}

async fn recovered(
    client: &DashboardClient,
    token: &str,
    path: &str,
    error: anyhow::Error,
) -> Result<()> {
    match read(client, token, path).await {
        Ok(value) => print(json!({"recovered": true, "receipt": value})),
        Err(_) => {
            Err(error.context("Upload outcome is unknown. Query the existing asset receipt."))
        }
    }
}

async fn read(client: &DashboardClient, token: &str, path: &str) -> Result<Value> {
    client
        .campaign_read(path, token)
        .await?
        .require_data("WhatsApp query")
}

fn campaign_status(campaign: &str, operation: Option<&str>) -> Result<String> {
    Ok(query(
        &format!("campaigns/{}", id(campaign)?),
        &[("operationId", operation.map(str::to_owned))],
    ))
}

fn template_detail(waba: &str, kind: &str, reference: &str) -> Result<String> {
    let key = if kind == "TEMPLATE" {
        "ref[templateId]"
    } else {
        "ref[operationId]"
    };
    Ok(query(
        "template-detail",
        &[
            ("wabaId", Some(id(waba)?.to_owned())),
            ("ref[kind]", Some(kind.to_owned())),
            (key, Some(id(reference)?.to_owned())),
        ],
    ))
}

fn query(route: &str, pairs: &[(&str, Option<String>)]) -> String {
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(
            pairs
                .iter()
                .filter_map(|(key, value)| value.as_ref().map(|value| (*key, value.as_str()))),
        )
        .finish();
    if query.is_empty() {
        format!("{ROOT}/{route}")
    } else {
        format!("{ROOT}/{route}?{query}")
    }
}

fn id(value: &str) -> Result<&str> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        bail!("Invalid WhatsApp resource identifier");
    }
    Ok(value)
}

fn print(mut value: Value) -> Result<()> {
    fn redact(value: &mut Value) {
        match value {
            Value::Object(fields) => {
                for key in [
                    "confirmationToken",
                    "idempotencyKey",
                    "executionTicket",
                    "upload",
                ] {
                    fields.remove(key);
                }
                for item in fields.values_mut() {
                    redact(item);
                }
            }
            Value::Array(items) => items.iter_mut().for_each(redact),
            _ => {}
        }
    }
    redact(&mut value);
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
