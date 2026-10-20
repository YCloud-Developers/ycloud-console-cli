pub mod analytics;
pub mod auth;
pub mod campaign;
pub mod cli;
pub mod config;
pub mod export;
pub mod http;
pub mod oauth;
mod oauth_callback;
pub mod permissions;
pub mod pkce;
pub mod waba_assignment;

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{
    AnalyticsCommand, AnalyticsTemplatesCommand, CampaignCommand, Cli, Command, ContactsCommand,
    ExportsCommand, InboxCommand, InboxConversationsCommand, IntegrationsCommand, TenantsCommand,
    WabaAssignmentCommand, WhatsappCommand, DEFAULT_DASHBOARD_URL,
};
use config::Config;
use http::{DashboardApiError, InvocationMode};

pub async fn run() -> Result<()> {
    let mut cli = Cli::parse();
    preserve_idempotency_key(&mut cli.command);
    let config_path = cli.config_path()?;
    let invocation_mode = cli.invocation_mode()?;

    match execute_once(&cli, &config_path, invocation_mode).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let Some(api_error) = error.downcast_ref::<DashboardApiError>() else {
                return Err(error);
            };
            let config = Config::load(&config_path)
                .context("failed to load the current permissions for reauthorization")?;
            let Some(mut plan) =
                permissions::plan(&cli, &config.auth.requested_permissions, api_error)
            else {
                return Err(error);
            };

            if config.auth.oauth.is_some() {
                plan.login_command =
                    permissions::oauth_login_command(&cli, &config, &plan.requested_permissions);
            }

            eprintln!(
                "Permission denied (requestId={}, traceId={}).",
                api_error.request_id().unwrap_or_default(),
                api_error.trace_id().unwrap_or_default()
            );
            if plan.missing_permissions.is_empty() {
                eprintln!(
                    "The required permission is already requested but is not effective for this account. Ask a tenant administrator to update the account role or data range."
                );
                eprintln!(
                    "Requested permissions: {}",
                    plan.requested_permissions.join(",")
                );
                return Err(error);
            }

            eprintln!(
                "Missing permissions: {}",
                plan.missing_permissions.join(",")
            );
            if invocation_mode == InvocationMode::Automation {
                eprintln!("Run this command interactively, then retry:");
                eprintln!("{}", plan.login_command);
                return Err(error);
            }

            if !confirm_reauthorization()? {
                eprintln!("Reauthorization cancelled. Run this command when ready:");
                eprintln!("{}", plan.login_command);
                return Err(error);
            }

            let client =
                client_for_saved_profile(cli.dashboard_url.clone(), &config_path, invocation_mode)
                    .await?;
            auth::reauthorize(&client, &config_path, plan.requested_permissions).await?;
            match execute_once(&cli, &config_path, invocation_mode).await {
                Err(retry_error) => {
                    if let Some(api_error) = retry_error.downcast_ref::<DashboardApiError>() {
                        if permissions::is_reauthorizable(api_error) {
                            eprintln!(
                                "The command is still denied after reauthorization (requestId={}, traceId={}). Ask a tenant administrator to verify the account role and data range.",
                                api_error.request_id().unwrap_or_default(),
                                api_error.trace_id().unwrap_or_default()
                            );
                        }
                    }
                    Err(retry_error)
                }
                result => result,
            }
        }
    }
}

async fn execute_once(
    cli: &Cli,
    config_path: &Path,
    invocation_mode: InvocationMode,
) -> Result<()> {
    match &cli.command {
        Command::Login(args) => {
            let dashboard_url = cli
                .dashboard_url
                .clone()
                .unwrap_or_else(|| DEFAULT_DASHBOARD_URL.to_string());
            let client = http::DashboardClient::new_with_mode(dashboard_url, invocation_mode)?;
            auth::login(&client, config_path, args.clone()).await
        }
        Command::Whoami => {
            let client = saved_client(cli, config_path, invocation_mode).await?;
            auth::whoami(&client, config_path).await
        }
        Command::Analytics { command } => {
            let client = match command {
                AnalyticsCommand::Overview(_)
                | AnalyticsCommand::Trend(_)
                | AnalyticsCommand::Templates { .. } => {
                    saved_client_with_timeout(
                        cli,
                        config_path,
                        invocation_mode,
                        std::time::Duration::from_secs(30),
                    )
                    .await?
                }
                _ => saved_client(cli, config_path, invocation_mode).await?,
            };
            match command {
                AnalyticsCommand::Outline(args) => {
                    auth::analytics_outline(&client, config_path, args.clone()).await
                }
                AnalyticsCommand::Overview(args) => {
                    analytics::overview(&client, config_path, args.clone()).await
                }
                AnalyticsCommand::Trend(args) => {
                    analytics::trend(&client, config_path, args.clone()).await
                }
                AnalyticsCommand::Templates { command } => match command {
                    AnalyticsTemplatesCommand::List(args) => {
                        analytics::templates_list(&client, config_path, args.clone()).await
                    }
                    AnalyticsTemplatesCommand::Export(args) => {
                        export::export_template_statistics(&client, config_path, args.clone()).await
                    }
                },
                AnalyticsCommand::Logs(args) => {
                    auth::analytics_logs(&client, config_path, args.clone()).await
                }
                AnalyticsCommand::CallingLogs(args) => {
                    auth::analytics_calling_logs(&client, config_path, args.clone()).await
                }
            }
        }
        Command::Contacts { command } => {
            let client = saved_client(cli, config_path, invocation_mode).await?;
            match command {
                ContactsCommand::List(args) => {
                    auth::contacts_list(&client, config_path, args.clone()).await
                }
                ContactsCommand::Stats(args) => {
                    auth::contact_statistics(&client, config_path, args.clone()).await
                }
                ContactsCommand::Metadata => auth::contacts_metadata(&client, config_path).await,
                ContactsCommand::Export(args) => {
                    export::export_contacts(&client, config_path, args.clone()).await
                }
            }
        }
        Command::Campaign { command } => {
            let client = saved_client(cli, config_path, invocation_mode).await?;
            match command {
                CampaignCommand::List(args) => {
                    campaign::list(&client, config_path, args.clone()).await
                }
                CampaignCommand::Messages(args) => {
                    campaign::messages(&client, config_path, args.clone()).await
                }
                CampaignCommand::Analytics(args) => {
                    campaign::analytics(&client, config_path, args.clone()).await
                }
                CampaignCommand::Export(args) => {
                    campaign::export_messages(&client, config_path, args.clone()).await
                }
            }
        }
        Command::Whatsapp { command } => {
            let client = saved_client(cli, config_path, invocation_mode).await?;
            match command {
                WhatsappCommand::WabaAssignment { command } => match command {
                    WabaAssignmentCommand::List(args) => {
                        waba_assignment::list(&client, config_path, args.clone()).await
                    }
                },
            }
        }
        Command::Inbox { command } => {
            let client = saved_client(cli, config_path, invocation_mode).await?;
            match command {
                InboxCommand::Conversations { command } => match command {
                    InboxConversationsCommand::Search(args) => {
                        export::search_conversations(&client, config_path, args.clone()).await
                    }
                    InboxConversationsCommand::Export(args) => {
                        export::export_conversations(&client, config_path, args.clone()).await
                    }
                },
            }
        }
        Command::Exports { command } => {
            let client = saved_client(cli, config_path, invocation_mode).await?;
            match command {
                ExportsCommand::Query(args) => {
                    export::query_export(&client, config_path, args.clone()).await
                }
                ExportsCommand::Retry(args) => {
                    export::retry_export(&client, config_path, args.clone()).await
                }
                ExportsCommand::Download(args) => {
                    export::download_export(&client, config_path, args.clone()).await
                }
            }
        }
        Command::Integrations {
            command: IntegrationsCommand::Status,
        } => {
            let client = saved_client(cli, config_path, invocation_mode).await?;
            auth::integrations_status(&client, config_path).await
        }
        Command::Tenants {
            command: TenantsCommand::List,
        } => {
            let client = saved_client(cli, config_path, invocation_mode).await?;
            auth::tenants_list(&client, config_path).await
        }
        Command::Refresh => {
            let client = saved_client(cli, config_path, invocation_mode).await?;
            auth::refresh(&client, config_path).await
        }
        Command::Logout => {
            let client = client_for_logout(cli, config_path, invocation_mode)?;
            auth::logout(&client, config_path).await
        }
    }
}

fn client_for_logout(
    cli: &Cli,
    config_path: &Path,
    mode: InvocationMode,
) -> Result<http::DashboardClient> {
    let config = Config::load(config_path)?;
    let base = cli
        .dashboard_url
        .as_deref()
        .unwrap_or(&config.dashboard.base_url);
    if let Some(session) = &config.auth.oauth {
        if base != config.dashboard.base_url {
            anyhow::bail!("OAuth business address override changed; log in again");
        }
        oauth::OAuthClient::configured(
            &session.issuer,
            &session.client_id,
            &session.resource,
            base,
        )?
        .require_session(session)?;
    }
    // Discovery/remote failures are handled inside logout so local cleanup still happens.
    http::DashboardClient::new_with_mode(base.to_string(), mode)
}

async fn saved_client(
    cli: &Cli,
    config_path: &Path,
    invocation_mode: InvocationMode,
) -> Result<http::DashboardClient> {
    client_for_saved_profile(cli.dashboard_url.clone(), config_path, invocation_mode).await
}

async fn saved_client_with_timeout(
    cli: &Cli,
    config_path: &Path,
    invocation_mode: InvocationMode,
    timeout: std::time::Duration,
) -> Result<http::DashboardClient> {
    saved_profile_client(
        cli.dashboard_url.clone(),
        config_path,
        invocation_mode,
        Some(timeout),
    )
    .await
}

async fn client_for_saved_profile(
    override_url: Option<String>,
    config_path: &Path,
    invocation_mode: InvocationMode,
) -> Result<http::DashboardClient> {
    saved_profile_client(override_url, config_path, invocation_mode, None).await
}

async fn saved_profile_client(
    override_url: Option<String>,
    config_path: &Path,
    invocation_mode: InvocationMode,
    timeout: Option<std::time::Duration>,
) -> Result<http::DashboardClient> {
    let _guard = config::lock_profile(config_path).await?;
    let mut config = Config::load(config_path)?;
    let url = override_url.unwrap_or_else(|| config.dashboard.base_url.clone());
    if config.auth.oauth.is_some() {
        oauth::prepare_session_locked(&mut config, config_path, &url).await?;
    }
    let client = match timeout {
        Some(timeout) => {
            http::DashboardClient::new_with_mode_and_timeout(url, invocation_mode, timeout)?
        }
        None => http::DashboardClient::new_with_mode(url, invocation_mode)?,
    };
    let client = match &config.auth.oauth {
        Some(session) => client.bind_oauth(&session.resource)?,
        None => client,
    };
    Ok(client.bind_profile(&config.auth.access_token, &config.auth.refresh_token))
}

fn preserve_idempotency_key(command: &mut Command) {
    let slot = match command {
        Command::Contacts {
            command: ContactsCommand::Export(args),
        } => &mut args.idempotency_key,
        Command::Inbox {
            command:
                InboxCommand::Conversations {
                    command: InboxConversationsCommand::Export(args),
                },
        } => &mut args.idempotency_key,
        Command::Exports {
            command: ExportsCommand::Retry(args),
        } => &mut args.idempotency_key,
        Command::Analytics {
            command:
                AnalyticsCommand::Templates {
                    command: AnalyticsTemplatesCommand::Export(args),
                },
        } => &mut args.idempotency_key,
        Command::Campaign {
            command: CampaignCommand::Export(args),
        } => &mut args.idempotency_key,
        _ => return,
    };
    if slot.is_none() {
        *slot = Some(export::new_idempotency_key());
    }
}

fn confirm_reauthorization() -> Result<bool> {
    eprint!("Reauthorize with the combined permission set and retry once? [y/N] ");
    io::stderr()
        .flush()
        .context("failed to flush permission prompt")?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("failed to read permission prompt")?;
    Ok(is_confirmation(&answer))
}

fn is_confirmation(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replaced_profile_cannot_send_new_credentials_through_old_client() {
        use crate::config::{AuthConfig, DashboardConfig, OAuthSession};
        let server = wiremock::MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.toml");
        let mut config = Config {
            dashboard: DashboardConfig {
                base_url: server.uri(),
            },
            auth: AuthConfig {
                oauth: None,
                token_type: "Bearer".into(),
                access_token: "fixture-old-access".into(),
                refresh_token: "fixture-old-refresh".into(),
                record_id: "fixture".into(),
                requested_permissions: vec![],
                tenant_id: None,
                user_id: None,
            },
        };
        config.save(&path).unwrap();
        let client = client_for_saved_profile(None, &path, InvocationMode::Automation)
            .await
            .unwrap();
        config.dashboard.base_url = "https://business.bound.example".into();
        config.auth.access_token = "yc_oauth_at_fixture-new".into();
        config.auth.refresh_token = "yc_oauth_rt_fixture-new".into();
        config.auth.oauth = Some(OAuthSession {
            version: 2,
            business_base_url: Some(config.dashboard.base_url.clone()),
            issuer: "https://auth.bound.example".into(),
            client_id: "fixture".into(),
            resource: "https://business.bound.example/api/cli".into(),
            token_endpoint: Some("https://auth.bound.example/api/oauth/v1/token".into()),
            revocation_endpoint: Some("https://auth.bound.example/api/oauth/v1/revoke".into()),
            access_expires_at: 4_102_444_800,
            refresh_uncertain: false,
        });
        config.save(&path).unwrap();
        assert!(auth::whoami(&client, &path).await.is_err());
        assert!(client
            .contacts_metadata(&config.auth.access_token)
            .await
            .is_err());
        assert!(client
            .revoke(&config.auth.access_token, "fixture")
            .await
            .is_err());
        assert!(client.refresh(&config.auth.refresh_token).await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(!format!("{client:?}").contains("fixture-old"));
    }

    #[test]
    fn reauthorization_confirmation_defaults_to_no() {
        assert!(!is_confirmation(""));
        assert!(!is_confirmation("n"));
        assert!(is_confirmation("Y"));
        assert!(is_confirmation("yes"));
    }

    #[test]
    fn generated_export_idempotency_key_survives_command_clone() {
        let mut cli =
            Cli::try_parse_from(["ycloud", "inbox", "conversations", "export", "--no-wait"])
                .unwrap();

        preserve_idempotency_key(&mut cli.command);
        let cloned = cli.command.clone();
        let Command::Inbox {
            command:
                InboxCommand::Conversations {
                    command: InboxConversationsCommand::Export(first),
                },
        } = cli.command
        else {
            panic!("expected conversation export");
        };
        let Command::Inbox {
            command:
                InboxCommand::Conversations {
                    command: InboxConversationsCommand::Export(second),
                },
        } = cloned
        else {
            panic!("expected cloned conversation export");
        };

        assert!(first.idempotency_key.is_some());
        assert_eq!(first.idempotency_key, second.idempotency_key);
    }

    #[test]
    fn template_export_receives_generated_idempotency_key() {
        let mut cli =
            Cli::try_parse_from(["ycloud", "analytics", "templates", "export", "--no-wait"])
                .unwrap();

        preserve_idempotency_key(&mut cli.command);
        let Command::Analytics {
            command:
                AnalyticsCommand::Templates {
                    command: AnalyticsTemplatesCommand::Export(args),
                },
        } = cli.command
        else {
            panic!("expected template statistics export");
        };
        assert!(args.idempotency_key.is_some());
    }

    #[test]
    fn campaign_export_receives_generated_idempotency_key() {
        let mut cli = Cli::try_parse_from([
            "ycloud",
            "campaign",
            "export",
            "--campaign-id",
            "campaign-1",
            "--start-time",
            "1",
            "--end-time",
            "2",
            "--no-wait",
        ])
        .unwrap();

        preserve_idempotency_key(&mut cli.command);
        let Command::Campaign {
            command: CampaignCommand::Export(args),
        } = cli.command
        else {
            panic!("expected Campaign export");
        };
        assert!(args.idempotency_key.is_some());
    }
}
