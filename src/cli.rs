use std::{io::IsTerminal, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::http::InvocationMode;

pub const DEFAULT_DASHBOARD_URL: &str = "https://www.ycloud.com";

#[derive(Debug, Parser, Clone)]
#[command(name = "ycloud", version, about = "YCloud Console CLI")]
pub struct Cli {
    #[arg(long, global = true, env = "YCLOUD_DASHBOARD_URL")]
    pub dashboard_url: Option<String>,

    #[arg(long, global = true, env = "YCLOUD_CONFIG")]
    pub config: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        help = "Use automation retry budgets; overrides YCLOUD_INVOCATION_MODE and TTY detection"
    )]
    pub automation: bool,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn config_path(&self) -> Result<PathBuf> {
        if let Some(path) = &self.config {
            return Ok(path.clone());
        }
        let home = dirs::home_dir().context("failed to resolve home directory")?;
        Ok(home.join(".ycloud").join("config.toml"))
    }

    pub fn invocation_mode(&self) -> Result<InvocationMode> {
        if self.automation {
            return Ok(InvocationMode::Automation);
        }
        if let Ok(value) = std::env::var("YCLOUD_INVOCATION_MODE") {
            return match value.trim().to_ascii_lowercase().as_str() {
                "interactive" => Ok(InvocationMode::Interactive),
                "automation" => Ok(InvocationMode::Automation),
                _ => anyhow::bail!("YCLOUD_INVOCATION_MODE must be interactive or automation"),
            };
        }
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            return Ok(InvocationMode::Automation);
        }
        Ok(InvocationMode::Interactive)
    }
}

#[derive(Debug, Subcommand, Clone)]
pub enum Command {
    #[command(about = "Authorize ycloud with a logged-in YCloud Dashboard browser")]
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
    #[command(about = "Search and export Inbox conversations")]
    Inbox {
        #[command(subcommand)]
        command: InboxCommand,
    },
    #[command(about = "Query, retry, or download asynchronous exports")]
    Exports {
        #[command(subcommand)]
        command: ExportsCommand,
    },
    #[command(about = "Query Dashboard integration status APIs")]
    Integrations {
        #[command(subcommand)]
        command: IntegrationsCommand,
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

#[derive(Debug, Subcommand, Clone)]
pub enum TenantsCommand {
    List,
}

#[derive(Debug, Subcommand, Clone)]
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

#[derive(Debug, Args, Clone)]
pub struct LoginArgs {
    #[arg(long, value_enum, default_value_t = PermissionProfile::Basic)]
    pub profile: PermissionProfile,

    #[arg(
        long = "permission",
        value_name = "PERMISSION",
        help = "Additional atomic CLI permission; may be repeated"
    )]
    pub permissions: Vec<String>,

    #[arg(long, help = "Authorization code returned by /api/cli/auth/authorize")]
    pub code: Option<String>,

    #[arg(long, help = "PKCE code verifier used for scripted login tests")]
    pub code_verifier: Option<String>,

    #[arg(long, help = "OAuth state used for scripted login tests")]
    pub state: Option<String>,

    #[arg(
        long,
        help = "Use manual copy/paste authorization code flow instead of localhost callback"
    )]
    pub manual: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PermissionProfile {
    Basic,
    ContactsRead,
    AnalyticsRead,
    IntegrationsRead,
    Readonly,
    Custom,
}

impl PermissionProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::ContactsRead => "contacts-read",
            Self::AnalyticsRead => "analytics-read",
            Self::IntegrationsRead => "integrations-read",
            Self::Readonly => "readonly",
            Self::Custom => "custom",
        }
    }
}

impl std::fmt::Display for PermissionProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Subcommand, Clone)]
pub enum ContactsCommand {
    List(ContactsListArgs),
    #[command(about = "List contact sources, tags, segments, and segment filters")]
    Metadata,
    #[command(about = "Export contacts asynchronously")]
    Export(ContactExportArgs),
}

#[derive(Debug, Subcommand, Clone)]
pub enum InboxCommand {
    Conversations {
        #[command(subcommand)]
        command: InboxConversationsCommand,
    },
}

#[derive(Debug, Subcommand, Clone)]
pub enum InboxConversationsCommand {
    #[command(about = "Search permission-scoped Inbox conversations")]
    Search(ConversationSearchArgs),
    #[command(about = "Export conversations and, by default, their contacts")]
    Export(ConversationExportArgs),
}

#[derive(Debug, Subcommand, Clone)]
pub enum ExportsCommand {
    #[command(about = "Query an export task")]
    Query(ExportTaskArgs),
    #[command(about = "Retry a failed or partially successful export as a new task")]
    Retry(ExportRetryArgs),
    #[command(about = "Download selected ready artifacts")]
    Download(ExportDownloadArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExportFormat {
    Auto,
    Csv,
    Xlsx,
}

impl ExportFormat {
    pub fn as_api_value(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Csv => "CSV",
            Self::Xlsx => "XLSX",
        }
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_api_value().to_ascii_lowercase().as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExportArtifactType {
    Conversations,
    Contacts,
    Messages,
    Manifest,
    Archive,
}

impl ExportArtifactType {
    pub fn as_api_value(self) -> &'static str {
        match self {
            Self::Conversations => "CONVERSATIONS",
            Self::Contacts => "CONTACTS",
            Self::Messages => "MESSAGES",
            Self::Manifest => "MANIFEST",
            Self::Archive => "ARCHIVE",
        }
    }
}

#[derive(Debug, Args, Clone, Default)]
pub struct ConversationFilterArgs {
    #[arg(long = "inbox-id", value_delimiter = ',')]
    pub inbox_ids: Vec<String>,
    #[arg(long = "assignee-id", value_delimiter = ',')]
    pub assignee_ids: Vec<String>,
    #[arg(long = "conversation-tag-id", value_delimiter = ',')]
    pub conversation_tag_ids: Vec<String>,
    #[arg(long = "contact-tag-id", value_delimiter = ',')]
    pub contact_tag_ids: Vec<String>,
    #[arg(long)]
    pub status: Option<i32>,
    #[arg(long)]
    pub condition: Option<String>,
    #[arg(long)]
    pub start_time: Option<i64>,
    #[arg(long)]
    pub end_time: Option<i64>,
    #[arg(long)]
    pub close_start_time: Option<i64>,
    #[arg(long)]
    pub close_end_time: Option<i64>,
    #[arg(long)]
    pub csat: Option<i32>,
    #[arg(long)]
    pub customer: Option<bool>,
}

#[derive(Debug, Args, Clone)]
pub struct ConversationSearchArgs {
    #[command(flatten)]
    pub filter: ConversationFilterArgs,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=1000))]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args, Clone)]
pub struct ConversationExportArgs {
    #[command(flatten)]
    pub filter: ConversationFilterArgs,
    #[arg(long, value_delimiter = ',')]
    pub columns: Vec<String>,
    #[arg(long, value_enum, default_value_t = ExportFormat::Auto)]
    pub format: ExportFormat,
    #[arg(long, help = "Do not generate the related contacts artifact")]
    pub no_contacts: bool,
    #[arg(
        long,
        help = "Also generate archive.zip; independent artifacts remain available"
    )]
    pub archive: bool,
    #[arg(long, default_value = "GMT")]
    pub timezone: String,
    #[arg(long)]
    pub file_name: Option<String>,
    #[arg(
        long,
        help = "Reuse this key to recover safely from an uncertain create response"
    )]
    pub idempotency_key: Option<String>,
    #[command(flatten)]
    pub wait: ExportWaitArgs,
}

#[derive(Debug, Args, Clone)]
pub struct ContactExportArgs {
    #[arg(long)]
    pub condition: Option<String>,
    #[arg(long)]
    pub segment_id: Option<String>,
    #[arg(long)]
    pub blocked: Option<bool>,
    #[arg(long, value_delimiter = ',')]
    pub columns: Vec<String>,
    #[arg(long, value_enum, default_value_t = ExportFormat::Auto)]
    pub format: ExportFormat,
    #[arg(long)]
    pub archive: bool,
    #[arg(long, default_value = "GMT")]
    pub timezone: String,
    #[arg(long)]
    pub file_name: Option<String>,
    #[arg(
        long,
        help = "Reuse this key to recover safely from an uncertain create response"
    )]
    pub idempotency_key: Option<String>,
    #[command(flatten)]
    pub wait: ExportWaitArgs,
}

#[derive(Debug, Args, Clone)]
pub struct ExportWaitArgs {
    #[arg(long, help = "Return immediately after task creation")]
    pub no_wait: bool,
    #[arg(long, default_value = ".")]
    pub output_dir: PathBuf,
    #[arg(long = "artifact", value_enum, value_delimiter = ',')]
    pub artifacts: Vec<ExportArtifactType>,
    #[arg(long, default_value_t = 3600)]
    pub timeout_seconds: u64,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args, Clone)]
pub struct ExportTaskArgs {
    pub task_id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args, Clone)]
pub struct ExportRetryArgs {
    pub task_id: String,
    #[arg(
        long,
        help = "Reuse this key to recover safely from an uncertain retry response"
    )]
    pub idempotency_key: Option<String>,
    #[command(flatten)]
    pub wait: ExportWaitArgs,
}

#[derive(Debug, Args, Clone)]
pub struct ExportDownloadArgs {
    pub task_id: String,
    #[arg(long = "artifact", value_enum, value_delimiter = ',')]
    pub artifacts: Vec<ExportArtifactType>,
    #[arg(long, default_value = ".")]
    pub output_dir: PathBuf,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand, Clone)]
pub enum IntegrationsCommand {
    #[command(about = "List enabled status for Dashboard integrations")]
    Status,
}

#[derive(Debug, Args, Clone)]
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
        default_value = "Asia/Shanghai",
        help = "IANA analytics timezone, for example Asia/Shanghai or UTC"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_defaults_to_online() {
        assert_eq!(DEFAULT_DASHBOARD_URL, "https://www.ycloud.com");
    }

    #[test]
    fn login_defaults_to_basic_profile_without_explicit_permissions() {
        let cli = Cli::try_parse_from(["ycloud", "login", "--manual"]).unwrap();

        let Command::Login(args) = cli.command else {
            panic!("expected login command");
        };
        assert_eq!(args.profile, PermissionProfile::Basic);
        assert!(args.permissions.is_empty());
    }

    #[test]
    fn login_accepts_profile_and_repeated_permission_flags() {
        let cli = Cli::try_parse_from([
            "ycloud",
            "login",
            "--profile",
            "analytics-read",
            "--permission",
            "yc.integration.status.read",
            "--permission",
            "yc.contact.record.read",
            "--manual",
        ])
        .unwrap();

        let Command::Login(args) = cli.command else {
            panic!("expected login command");
        };
        assert_eq!(args.profile, PermissionProfile::AnalyticsRead);
        assert_eq!(
            args.permissions,
            ["yc.integration.status.read", "yc.contact.record.read"]
        );
    }

    #[test]
    fn explicit_automation_flag_selects_automation_mode() {
        let cli = Cli::try_parse_from(["ycloud", "--automation", "whoami"]).unwrap();

        assert_eq!(cli.invocation_mode().unwrap(), InvocationMode::Automation);
    }

    #[test]
    fn parses_combined_conversation_export_defaults() {
        let cli = Cli::try_parse_from([
            "ycloud",
            "inbox",
            "conversations",
            "export",
            "--inbox-id",
            "inbox-1,inbox-2",
            "--no-wait",
            "--idempotency-key",
            "conversation-export-1",
        ])
        .unwrap();
        let Command::Inbox {
            command:
                InboxCommand::Conversations {
                    command: InboxConversationsCommand::Export(args),
                },
        } = cli.command
        else {
            panic!("expected conversation export");
        };
        assert_eq!(args.filter.inbox_ids, ["inbox-1", "inbox-2"]);
        assert!(!args.no_contacts);
        assert!(args.wait.no_wait);
        assert_eq!(args.format, ExportFormat::Auto);
        assert_eq!(
            args.idempotency_key.as_deref(),
            Some("conversation-export-1")
        );
    }

    #[test]
    fn parses_independent_contact_export() {
        let cli = Cli::try_parse_from([
            "ycloud",
            "contacts",
            "export",
            "--condition",
            "vip",
            "--format",
            "csv",
            "--artifact",
            "contacts,manifest",
            "--idempotency-key",
            "contact-export-1",
        ])
        .unwrap();
        let Command::Contacts {
            command: ContactsCommand::Export(args),
        } = cli.command
        else {
            panic!("expected contact export");
        };
        assert_eq!(args.condition.as_deref(), Some("vip"));
        assert_eq!(args.format, ExportFormat::Csv);
        assert_eq!(args.wait.artifacts.len(), 2);
        assert_eq!(args.idempotency_key.as_deref(), Some("contact-export-1"));
    }
}
