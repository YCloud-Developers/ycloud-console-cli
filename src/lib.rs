pub mod auth;
pub mod cli;
pub mod config;
pub mod http;
pub mod pkce;

use anyhow::Result;
use clap::Parser;
use cli::{
    AnalyticsCommand, Cli, Command, ContactsCommand, IntegrationsCommand, TenantsCommand,
    DEFAULT_DASHBOARD_URL,
};
use config::Config;

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config_path()?;
    let dashboard_url_override = cli.dashboard_url.clone();

    match cli.command {
        Command::Login(args) => {
            let dashboard_url =
                dashboard_url_override.unwrap_or_else(|| DEFAULT_DASHBOARD_URL.to_string());
            let client = http::DashboardClient::new(dashboard_url)?;
            auth::login(&client, &config_path, args).await
        }
        Command::Whoami => {
            let client = client_for_saved_profile(dashboard_url_override, &config_path)?;
            auth::whoami(&client, &config_path).await
        }
        Command::Analytics { command } => {
            let client = client_for_saved_profile(dashboard_url_override, &config_path)?;
            match command {
                AnalyticsCommand::Outline(args) => {
                    auth::analytics_outline(&client, &config_path, args).await
                }
                AnalyticsCommand::Overview(args) => {
                    auth::analytics_overview(&client, &config_path, args).await
                }
                AnalyticsCommand::Logs(args) => {
                    auth::analytics_logs(&client, &config_path, args).await
                }
                AnalyticsCommand::CallingLogs(args) => {
                    auth::analytics_calling_logs(&client, &config_path, args).await
                }
            }
        }
        Command::Contacts { command } => match command {
            ContactsCommand::List(args) => {
                let client = client_for_saved_profile(dashboard_url_override, &config_path)?;
                auth::contacts_list(&client, &config_path, args).await
            }
            ContactsCommand::Metadata => {
                let client = client_for_saved_profile(dashboard_url_override, &config_path)?;
                auth::contacts_metadata(&client, &config_path).await
            }
        },
        Command::Integrations { command } => match command {
            IntegrationsCommand::Status => {
                let client = client_for_saved_profile(dashboard_url_override, &config_path)?;
                auth::integrations_status(&client, &config_path).await
            }
        },
        Command::Tenants { command } => match command {
            TenantsCommand::List => {
                let client = client_for_saved_profile(dashboard_url_override, &config_path)?;
                auth::tenants_list(&client, &config_path).await
            }
        },
        Command::Refresh => {
            let client = client_for_saved_profile(dashboard_url_override, &config_path)?;
            auth::refresh(&client, &config_path).await
        }
        Command::Logout => {
            let client = client_for_saved_profile(dashboard_url_override, &config_path)?;
            auth::logout(&client, &config_path).await
        }
    }
}

fn client_for_saved_profile(
    override_url: Option<String>,
    config_path: &std::path::Path,
) -> Result<http::DashboardClient> {
    let dashboard_url = match override_url {
        Some(url) => url,
        None => Config::load(config_path)?.dashboard.base_url,
    };
    http::DashboardClient::new(dashboard_url)
}
