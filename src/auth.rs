use std::{
    io::{self, Write},
    path::Path,
};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

use crate::{
    cli::{ContactsListArgs, LoginArgs},
    config::{self, AuthConfig, Config, DashboardConfig},
    http::{ContactsSearchRequest, DashboardClient, TokenData},
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

fn random_state_suffix() -> String {
    let bytes: [u8; 8] = rand::random();
    URL_SAFE_NO_PAD.encode(bytes)
}
