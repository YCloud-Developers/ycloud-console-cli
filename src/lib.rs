pub mod auth;
pub mod cli;
pub mod config;
pub mod export;
pub mod http;
pub mod permissions;
pub mod pkce;
pub mod waba_assignment;

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{
    AnalyticsCommand, Cli, Command, ContactsCommand, ExportsCommand, InboxCommand,
    InboxConversationsCommand, IntegrationsCommand, TenantsCommand, WabaAssignmentCommand,
    WhatsappCommand, DEFAULT_DASHBOARD_URL,
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
            let Some(plan) = permissions::plan(&cli, &config.auth.requested_permissions, api_error)
            else {
                return Err(error);
            };

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
                client_for_saved_profile(cli.dashboard_url.clone(), &config_path, invocation_mode)?;
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
            let client = saved_client(cli, config_path, invocation_mode)?;
            auth::whoami(&client, config_path).await
        }
        Command::Analytics { command } => {
            let client = saved_client(cli, config_path, invocation_mode)?;
            match command {
                AnalyticsCommand::Outline(args) => {
                    auth::analytics_outline(&client, config_path, args.clone()).await
                }
                AnalyticsCommand::Overview(args) => {
                    auth::analytics_overview(&client, config_path, args.clone()).await
                }
                AnalyticsCommand::Logs(args) => {
                    auth::analytics_logs(&client, config_path, args.clone()).await
                }
                AnalyticsCommand::CallingLogs(args) => {
                    auth::analytics_calling_logs(&client, config_path, args.clone()).await
                }
            }
        }
        Command::Contacts { command } => {
            let client = saved_client(cli, config_path, invocation_mode)?;
            match command {
                ContactsCommand::List(args) => {
                    auth::contacts_list(&client, config_path, args.clone()).await
                }
                ContactsCommand::Metadata => auth::contacts_metadata(&client, config_path).await,
                ContactsCommand::Export(args) => {
                    export::export_contacts(&client, config_path, args.clone()).await
                }
            }
        }
        Command::Whatsapp { command } => {
            let client = saved_client(cli, config_path, invocation_mode)?;
            match command {
                WhatsappCommand::WabaAssignment { command } => match command {
                    WabaAssignmentCommand::List(args) => {
                        waba_assignment::list(&client, config_path, args.clone()).await
                    }
                },
            }
        }
        Command::Inbox { command } => {
            let client = saved_client(cli, config_path, invocation_mode)?;
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
            let client = saved_client(cli, config_path, invocation_mode)?;
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
            let client = saved_client(cli, config_path, invocation_mode)?;
            auth::integrations_status(&client, config_path).await
        }
        Command::Tenants {
            command: TenantsCommand::List,
        } => {
            let client = saved_client(cli, config_path, invocation_mode)?;
            auth::tenants_list(&client, config_path).await
        }
        Command::Refresh => {
            let client = saved_client(cli, config_path, invocation_mode)?;
            auth::refresh(&client, config_path).await
        }
        Command::Logout => {
            let client = saved_client(cli, config_path, invocation_mode)?;
            auth::logout(&client, config_path).await
        }
    }
}

fn saved_client(
    cli: &Cli,
    config_path: &Path,
    invocation_mode: InvocationMode,
) -> Result<http::DashboardClient> {
    client_for_saved_profile(cli.dashboard_url.clone(), config_path, invocation_mode)
}

fn client_for_saved_profile(
    override_url: Option<String>,
    config_path: &Path,
    invocation_mode: InvocationMode,
) -> Result<http::DashboardClient> {
    let dashboard_url = match override_url {
        Some(url) => url,
        None => Config::load(config_path)?.dashboard.base_url,
    };
    http::DashboardClient::new_with_mode(dashboard_url, invocation_mode)
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
}
