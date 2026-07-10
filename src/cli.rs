use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

pub const DEFAULT_DASHBOARD_URL: &str = "http://127.0.0.1:8036";

#[derive(Debug, Parser)]
#[command(name = "yc", version, about = "YCloud Console CLI")]
pub struct Cli {
    #[arg(long, global = true, env = "YC_DASHBOARD_URL")]
    pub dashboard_url: Option<String>,

    #[arg(long, global = true, env = "YC_CONFIG")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn config_path(&self) -> Result<PathBuf> {
        if let Some(path) = &self.config {
            return Ok(path.clone());
        }
        let home = dirs::home_dir().context("failed to resolve home directory")?;
        Ok(home.join(".yc").join("config.toml"))
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Authorize yc with a logged-in YCloud Dashboard browser")]
    Login(LoginArgs),
    #[command(about = "Show the current Dashboard CLI identity and granted permissions")]
    Whoami,
    #[command(about = "Query Dashboard analytics data used by /app/dashboard/analytics")]
    Analytics {
        #[command(subcommand)]
        command: AnalyticsCommand,
    },
    #[command(about = "Query Dashboard contacts APIs")]
    Contacts {
        #[command(subcommand)]
        command: ContactsCommand,
    },
    #[command(about = "Query tenants available to the current Dashboard CLI token")]
    Tenants {
        #[command(subcommand)]
        command: TenantsCommand,
    },
    #[command(about = "Refresh the stored Dashboard CLI token pair")]
    Refresh,
    #[command(about = "Revoke the current Dashboard CLI token and remove local profile")]
    Logout,
}

#[derive(Debug, Subcommand)]
pub enum TenantsCommand {
    List,
}

#[derive(Debug, Subcommand)]
pub enum AnalyticsCommand {
    #[command(about = "List analytics filter options for the selected time range")]
    Outline(AnalyticsRangeArgs),
    #[command(about = "Fetch delivery, message detail, and failure reason analytics")]
    Overview(AnalyticsOverviewArgs),
    #[command(about = "Search WhatsApp message logs from Dashboard analytics")]
    Logs(AnalyticsLogsArgs),
    #[command(about = "Search Calling logs from Dashboard analytics")]
    CallingLogs(AnalyticsCallingLogsArgs),
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    #[arg(
        long,
        default_value = "developers",
        help = "Space-separated Dashboard permission scopes to request"
    )]
    pub scope: String,

    #[arg(long, help = "Authorization code returned by /api/cli/auth/authorize")]
    pub code: Option<String>,

    #[arg(long, help = "PKCE code verifier used for scripted login tests")]
    pub code_verifier: Option<String>,

    #[arg(long, help = "OAuth state used for scripted login tests")]
    pub state: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum ContactsCommand {
    List(ContactsListArgs),
}

#[derive(Debug, Args)]
pub struct ContactsListArgs {
    #[arg(long, default_value_t = 1, help = "Page number")]
    pub page_no: u32,

    #[arg(long, default_value_t = 10, help = "Page size")]
    pub page_size: u32,

    #[arg(
        long,
        help = "Search keyword for contact name, phone, or related fields"
    )]
    pub condition: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub struct AnalyticsRangeArgs {
    #[arg(long, help = "Start time in milliseconds since Unix epoch")]
    pub start_time: Option<i64>,

    #[arg(long, help = "End time in milliseconds since Unix epoch")]
    pub end_time: Option<i64>,
}

#[derive(Debug, Args, Clone)]
pub struct AnalyticsOverviewArgs {
    #[command(flatten)]
    pub range: AnalyticsRangeArgs,

    #[arg(
        long,
        default_value = "GMT+8",
        help = "Analytics timezone, matching Dashboard format such as GMT+8"
    )]
    pub timezone: String,

    #[arg(
        long,
        help = "WhatsApp business phone number, for example 8613800138000"
    )]
    pub from: Option<String>,

    #[arg(long, help = "ISO country or region code, for example CN")]
    pub region_code: Option<String>,

    #[arg(
        long,
        help = "Comma-separated message categories, for example marketing,utility"
    )]
    pub message_category: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub struct AnalyticsLogsArgs {
    #[command(flatten)]
    pub range: AnalyticsRangeArgs,

    #[arg(long, default_value_t = 1, help = "Page number")]
    pub page_no: u32,

    #[arg(long, default_value_t = 20, help = "Page size")]
    pub page_size: u32,

    #[arg(long, help = "Search by contact or template name")]
    pub condition: Option<String>,

    #[arg(
        long,
        default_value = "OutBound",
        help = "Message direction: OutBound or InBound"
    )]
    pub direction: String,

    #[arg(
        long,
        help = "WhatsApp business phone number, for example 8613800138000"
    )]
    pub from: Option<String>,

    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated WhatsApp business phone numbers"
    )]
    pub business_phones: Vec<String>,

    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated recipient region codes"
    )]
    pub to_region_codes: Vec<String>,

    #[arg(
        long,
        help = "Comma-separated outbound message statuses, for example sent,delivered"
    )]
    pub status: Option<String>,

    #[arg(
        long,
        help = "Message platform: WhatsApp Business API, WhatsApp Business App, or smb"
    )]
    pub source: Option<String>,

    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated pricing categories"
    )]
    pub pricing_category: Vec<String>,

    #[arg(
        long,
        default_value = "GMT+8",
        help = "Analytics timezone, matching Dashboard format such as GMT+8"
    )]
    pub timezone: String,
}

#[derive(Debug, Args, Clone)]
pub struct AnalyticsCallingLogsArgs {
    #[command(flatten)]
    pub range: AnalyticsRangeArgs,

    #[arg(long, default_value_t = 1, help = "Page number")]
    pub page_no: u32,

    #[arg(long, default_value_t = 20, help = "Page size")]
    pub page_size: u32,

    #[arg(long, help = "Search by contact")]
    pub condition: Option<String>,

    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated calling directions, for example BUSINESS_INITIATED"
    )]
    pub directions: Vec<String>,

    #[arg(long, value_delimiter = ',', help = "Comma-separated region codes")]
    pub region_codes: Vec<String>,

    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated calling sources, for example API,CALLING"
    )]
    pub sources: Vec<String>,

    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated calling statuses, for example COMPLETED,FAILED"
    )]
    pub status: Vec<String>,

    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated WhatsApp phone number ids"
    )]
    pub phone_number_ids: Vec<String>,
}
