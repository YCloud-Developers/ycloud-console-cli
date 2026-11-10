use std::{
    io::{self, BufRead, Read, Write},
    path::PathBuf,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use clap::{Args, ValueEnum};
use rand::{rngs::OsRng, RngCore};
use reqwest::{header, Response, StatusCode};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

#[derive(Debug, Clone, Args)]
pub struct PairUpload {
    #[arg(long)]
    file: PathBuf,
    #[arg(long, value_enum)]
    purpose: Purpose,
    #[arg(long)]
    content_type: String,
    #[arg(long)]
    waba_id: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Purpose {
    Audience,
    CampaignMedia,
}

#[derive(Deserialize)]
struct IssuedTicket {
    ticket: String,
}

pub async fn run(origin: &str, args: &PairUpload) -> Result<()> {
    let origin = crate::oauth::origin(origin)?;
    let trusted_ycloud = origin.scheme() == "https"
        && origin.port().is_none()
        && matches!(
            origin.host_str(),
            Some(
                "open-platform.ycloud.com"
                    | "open-platform-prepare.ycloud.com"
                    | "open-platform-test.ycloud.com"
                    | "open-platform-dev.ycloud.com"
            )
        );
    if !trusted_ycloud && origin.scheme() != "http" {
        bail!("Pair upload requires a trusted YCloud Platform origin");
    }

    if !valid_waba(&args.waba_id) || !valid_content_type(&args.content_type) {
        bail!("Expected a numeric WABA ID and a plain media type");
    }
    let (purpose, limit) = match args.purpose {
        Purpose::Audience => ("AUDIENCE", 15 * 1024 * 1024),
        Purpose::CampaignMedia => ("CAMPAIGN_MEDIA", 100 * 1024 * 1024),
    };
    let file_name = args
        .file
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && name.chars().count() <= 255)
        .context("Expected a UTF-8 file name of at most 255 characters")?;
    let mut file = tokio::fs::File::open(&args.file)
        .await
        .context("Cannot open the selected file")?;
    let metadata = file.metadata().await?;
    let size = metadata.len();
    if !metadata.is_file() || size == 0 || size > limit {
        bail!("Expected a nonempty regular file within the size limit");
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut seen = 0;
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        seen += n as u64;
        if seen > size {
            bail!("File changed while calculating its digest");
        }
        hasher.update(&buffer[..n]);
    }
    if seen != size {
        bail!("File changed while calculating its digest");
    }
    file.seek(std::io::SeekFrom::Start(0)).await?;
    let sha256 = format!("{:x}", hasher.finalize());
    let binding = hash(&format!(
        "ycloud-campaign-upload-v1\n{purpose}\n{}\n{}\n{size}\n{sha256}",
        args.waba_id, args.content_type
    ));
    let mut entropy = [0u8; 32];
    OsRng.fill_bytes(&mut entropy);
    let proof = URL_SAFE_NO_PAD.encode(entropy);
    let pairing_code = hash(&format!("{proof}\n{binding}"));
    println!("YCloud upload destination: {origin}");
    println!("Share only this JSON with WorkBuddy:");
    println!(
        "{}",
        json!({
            "purpose": purpose, "wabaId": args.waba_id, "fileName": file_name,
            "contentType": args.content_type, "sizeBytes": size, "sha256": sha256,
            "uploadPairingCode": pairing_code
        })
    );
    print!(
        "After approving this file in WorkBuddy, paste its assetId (or press Enter to cancel): "
    );
    io::stdout().flush()?;
    let mut asset = String::new();
    io::stdin().lock().take(128).read_line(&mut asset)?;
    let asset = asset.trim();
    if asset.is_empty() {
        bail!("Upload cancelled");
    }
    if !valid_nonce(asset) {
        bail!("Expected the assetId from the confirmed MCP upload");
    }

    let base = format!("/api/cli/v1/whatsapp/paired-uploads/{asset}");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build()?;
    let response = client
        .post(origin.join(&format!("{base}/ticket"))?)
        .header(header::AUTHORIZATION, credential("YCloudUploadProof", &proof)?)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Ticket outcome is unknown; check this asset in WorkBuddy before creating a new pairing"))?;
    drop(proof);
    if response.status() != StatusCode::OK {
        return rejected(response, "Ticket request");
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Could not read the upload ticket"))?
    {
        if body.len() + chunk.len() > 4096 {
            bail!("Invalid upload ticket response");
        }
        body.extend_from_slice(&chunk);
    }
    let ticket: IssuedTicket = serde_json::from_slice(&body)
        .map_err(|_| anyhow::anyhow!("Invalid upload ticket response"))?;
    drop(body);
    let response = client
        .put(origin.join(&format!("{base}/content"))?)
        .header(
            header::AUTHORIZATION,
            credential("YCloudUploadTicket", &ticket.ticket)?,
        )
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, size)
        .body(reqwest::Body::wrap_stream(ReaderStream::new(file)))
        .send()
        .await
        .map_err(|_| {
            anyhow::anyhow!("Upload outcome is unknown; query the existing asset in WorkBuddy")
        })?;
    if response.status() != StatusCode::NO_CONTENT {
        return rejected(response, "Upload");
    }
    println!("{}", json!({"assetId": asset, "assetStatus": "PARSING"}));
    Ok(())
}

fn credential(scheme: &str, nonce: &str) -> Result<header::HeaderValue> {
    if !valid_nonce(nonce) {
        bail!("Invalid upload credential");
    }
    let mut value = header::HeaderValue::from_str(&format!("{scheme} {nonce}"))
        .map_err(|_| anyhow::anyhow!("Invalid upload credential"))?;
    value.set_sensitive(true);
    Ok(value)
}

fn hash(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

fn valid_nonce(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn valid_waba(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|b| b.is_ascii_digit())
}

fn valid_content_type(value: &str) -> bool {
    value.len() <= 128
        && value.split_once('/').is_some_and(|(a, b)| {
            [a, b].into_iter().all(|part| {
                !part.is_empty()
                    && part.bytes().all(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-' | b'_')
                    })
            })
        })
}

fn rejected(response: Response, action: &str) -> Result<()> {
    // The endpoint never needs to surface the response body, which could contain a credential.
    bail!(
        "{action} failed (HTTP {}). Check the same asset in WorkBuddy; the CLI will not retry automatically.",
        response.status().as_u16()
    )
}
