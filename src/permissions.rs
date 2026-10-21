use std::collections::BTreeSet;

use reqwest::StatusCode;

use crate::{
    cli::{
        AnalyticsCommand, CampaignCommand, Cli, Command, ContactsCommand, ExportsCommand,
        InboxCommand, InboxConversationsCommand, IntegrationsCommand, TenantsCommand,
        WabaAssignmentCommand, WhatsappCommand,
    },
    http::DashboardApiError,
};

pub const PERMISSION_DENIED: &str = "permission_denied";
pub const WABA_ASSIGNMENT_PERMISSIONS: [&str; 3] = [
    "yc.whatsapp.phone.read",
    "yc.inbox.phone-assignment.read",
    "yc.inbox.assignment-rule.read",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReauthorizationPlan {
    pub requested_permissions: Vec<String>,
    pub missing_permissions: Vec<String>,
    pub login_command: String,
}

pub fn is_reauthorizable(error: &DashboardApiError) -> bool {
    error.status() == StatusCode::FORBIDDEN && error.code() == PERMISSION_DENIED
}

pub fn plan(
    cli: &Cli,
    current_requested: &[String],
    error: &DashboardApiError,
) -> Option<ReauthorizationPlan> {
    if !is_reauthorizable(error) || !supports_reauthorization(&cli.command) {
        return None;
    }

    let current: BTreeSet<String> = current_requested.iter().cloned().collect();
    let mut requested = current.clone();
    requested.extend(
        static_permissions(&cli.command)
            .into_iter()
            .map(str::to_string),
    );
    if let Some(required) = server_required_permission(error) {
        requested.insert(required.to_string());
    }
    let requested_permissions: Vec<String> = requested.into_iter().collect();
    let missing_permissions = requested_permissions
        .iter()
        .filter(|permission| !current.contains(*permission))
        .cloned()
        .collect();
    let login_command = login_command(cli, &requested_permissions);

    Some(ReauthorizationPlan {
        requested_permissions,
        missing_permissions,
        login_command,
    })
}

fn supports_reauthorization(command: &Command) -> bool {
    !matches!(
        command,
        Command::Login(_) | Command::Refresh | Command::Logout
    )
}

fn server_required_permission(error: &DashboardApiError) -> Option<&str> {
    error
        .details()?
        .get("requiredPermission")?
        .as_str()
        .filter(|value| is_permission_code(value))
}

fn is_permission_code(value: &str) -> bool {
    value.starts_with("yc.")
        && value.len() <= 128
        && value.bytes().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, b'.' | b'-' | b'_')
        })
}

fn static_permissions(command: &Command) -> Vec<&'static str> {
    match command {
        Command::Login(_) | Command::Refresh | Command::Logout => vec![],
        Command::Whoami => vec!["yc.identity.current.read"],
        Command::Tenants {
            command: TenantsCommand::List,
        } => vec!["yc.tenant.list.read"],
        Command::Integrations {
            command: IntegrationsCommand::Status,
        } => vec!["yc.integration.status.read"],
        Command::Contacts {
            command: ContactsCommand::List(_),
        } => vec!["yc.contact.record.read"],
        Command::Contacts {
            command: ContactsCommand::Stats(_),
        } => vec!["yc.contact.record.read"],
        Command::Contacts {
            command: ContactsCommand::Metadata,
        } => vec!["yc.contact.metadata.read"],
        Command::Contacts {
            command: ContactsCommand::Export(_),
        } => vec!["yc.contact.record.export"],
        Command::Campaign {
            command: CampaignCommand::List(_),
        } => vec!["yc.campaign.record.read"],
        Command::Campaign {
            command: CampaignCommand::Messages(_),
        } => vec!["yc.campaign.record.read", "yc.whatsapp.message.read"],
        Command::Campaign {
            command: CampaignCommand::Analytics(_),
        } => vec!["yc.campaign.record.read", "yc.whatsapp.analytics.read"],
        Command::Campaign {
            command: CampaignCommand::Export(_),
        } => vec!["yc.campaign.message.export", "yc.campaign.record.read"],
        Command::Analytics {
            command:
                AnalyticsCommand::Outline(_)
                | AnalyticsCommand::Overview(_)
                | AnalyticsCommand::Trend(_)
                | AnalyticsCommand::Templates { .. },
        } => vec!["yc.whatsapp.analytics.read"],
        Command::Analytics {
            command: AnalyticsCommand::Logs(_),
        } => vec!["yc.whatsapp.message.read"],
        Command::Analytics {
            command: AnalyticsCommand::CallingLogs(_),
        } => vec!["yc.calling.log.read"],
        Command::Whatsapp {
            command:
                WhatsappCommand::WabaAssignment {
                    command: WabaAssignmentCommand::List(_),
                },
        } => WABA_ASSIGNMENT_PERMISSIONS.to_vec(),
        Command::Inbox {
            command:
                InboxCommand::Conversations {
                    command: InboxConversationsCommand::Search(_),
                },
        } => vec!["yc.inbox.conversation.read"],
        Command::Inbox {
            command:
                InboxCommand::Conversations {
                    command: InboxConversationsCommand::Export(args),
                },
        } => {
            let mut permissions = vec!["yc.inbox.conversation.export", "yc.inbox.message.export"];
            if !args.no_contacts {
                permissions.push("yc.contact.record.export");
            }
            permissions
        }
        Command::Exports {
            command:
                ExportsCommand::Query(_) | ExportsCommand::Retry(_) | ExportsCommand::Download(_),
        } => vec![],
    }
}

fn login_command(cli: &Cli, permissions: &[String]) -> String {
    let mut command = "ycloud".to_string();
    if let Some(dashboard_url) = &cli.dashboard_url {
        command.push_str(" --dashboard-url ");
        command.push_str(&shell_arg(dashboard_url));
    }
    if let Some(config) = &cli.config {
        command.push_str(" --config ");
        command.push_str(&shell_arg(&config.to_string_lossy()));
    }
    command.push_str(" login --profile custom");
    let flags = permissions
        .iter()
        .map(|permission| format!(" --permission {}", shell_arg(permission)))
        .collect::<String>();
    format!("{command}{flags}")
}

pub fn oauth_login_command(
    cli: &Cli,
    config: &crate::config::Config,
    permissions: &[String],
) -> String {
    let mut saved = cli.clone();
    saved.dashboard_url = Some(config.dashboard.base_url.clone());
    let mut command = login_command(&saved, permissions);
    if let Some(session) = &config.auth.oauth {
        command.push_str(" --oauth-issuer ");
        command.push_str(&shell_arg(&session.issuer));
        command.push_str(" --oauth-client-id ");
        command.push_str(&shell_arg(&session.client_id));
        command.push_str(" --oauth-resource ");
        command.push_str(&shell_arg(&session.resource));
    }
    command
}

fn shell_arg(value: &str) -> String {
    if !value.is_empty()
        && value.bytes().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    b'-' | b'_' | b'.' | b'/' | b':' | b'@' | b'%' | b'+' | b'=' | b','
                )
        })
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    fn permission_error(required: &str) -> DashboardApiError {
        serde_json::from_value::<crate::http::ApiEnvelope<serde_json::Value>>(serde_json::json!({
            "code": 403,
            "msg": "Permission denied",
            "data": null,
            "error": {
                "code": "permission_denied",
                "message": "Permission denied",
                "retryable": false,
                "details": {"requiredPermission": required}
            }
        }))
        .unwrap()
        .error
        .map(|error| DashboardApiError::for_test(StatusCode::FORBIDDEN, error))
        .unwrap()
    }

    fn typed_error(status: StatusCode, code: &str) -> DashboardApiError {
        DashboardApiError::for_test(
            status,
            crate::http::ApiError {
                code: code.to_string(),
                message: "failure".to_string(),
                retryable: false,
                details: None,
            },
        )
    }

    #[test]
    fn combined_export_requests_both_permissions_and_preserves_existing_custom_permissions() {
        let cli = Cli::try_parse_from(["ycloud", "inbox", "conversations", "export", "--no-wait"])
            .unwrap();
        let plan = plan(
            &cli,
            &["yc.integration.status.read".to_string()],
            &permission_error("yc.inbox.conversation.export"),
        )
        .unwrap();

        assert_eq!(
            plan.requested_permissions,
            [
                "yc.contact.record.export",
                "yc.inbox.conversation.export",
                "yc.inbox.message.export",
                "yc.integration.status.read"
            ]
        );
        assert_eq!(
            plan.missing_permissions,
            [
                "yc.contact.record.export",
                "yc.inbox.conversation.export",
                "yc.inbox.message.export"
            ]
        );
    }

    #[test]
    fn export_task_permission_comes_only_from_server_detail() {
        let cli = Cli::try_parse_from(["ycloud", "exports", "query", "task-1"]).unwrap();
        let plan = plan(&cli, &[], &permission_error("yc.contact.record.export")).unwrap();

        assert_eq!(plan.requested_permissions, ["yc.contact.record.export"]);
        assert!(plan.login_command.contains("yc.contact.record.export"));
    }

    #[test]
    fn only_exact_permission_denied_403_is_reauthorizable() {
        assert!(is_reauthorizable(&typed_error(
            StatusCode::FORBIDDEN,
            "permission_denied"
        )));
        assert!(!is_reauthorizable(&typed_error(
            StatusCode::UNAUTHORIZED,
            "permission_denied"
        )));
        assert!(!is_reauthorizable(&typed_error(
            StatusCode::FORBIDDEN,
            "resource_forbidden"
        )));
        assert!(!is_reauthorizable(&typed_error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited"
        )));
    }

    #[test]
    fn remediation_command_preserves_dashboard_and_config_overrides() {
        let cli = Cli::try_parse_from([
            "ycloud",
            "--dashboard-url",
            "https://www-test-red.ycloud.com",
            "--config",
            "/tmp/ycloud profile.toml",
            "contacts",
            "list",
        ])
        .unwrap();
        let plan = plan(&cli, &[], &permission_error("yc.contact.record.read")).unwrap();

        assert_eq!(
            plan.login_command,
            "ycloud --dashboard-url https://www-test-red.ycloud.com --config '/tmp/ycloud profile.toml' login --profile custom --permission yc.contact.record.read"
        );
    }

    #[test]
    fn waba_assignment_requests_all_three_atomic_permissions() {
        let cli = Cli::try_parse_from(["ycloud", "whatsapp", "waba-assignment", "list"]).unwrap();
        let plan = plan(&cli, &[], &permission_error("yc.whatsapp.phone.read")).unwrap();

        assert_eq!(
            plan.requested_permissions,
            [
                "yc.inbox.assignment-rule.read",
                "yc.inbox.phone-assignment.read",
                "yc.whatsapp.phone.read"
            ]
        );
    }
}

#[cfg(test)]
mod oauth_recovery_tests {
    use super::*;
    use crate::config::{AuthConfig, Config, DashboardConfig, OAuthSession};
    use clap::Parser;
    #[test]
    fn recovery_uses_the_saved_environment_and_full_oauth_binding() {
        let cli =
            Cli::try_parse_from(["ycloud", "--config", "fixture.toml", "contacts", "metadata"])
                .unwrap();
        let config = Config {
            dashboard: DashboardConfig {
                base_url: "https://www-prepare.ycloud.com".into(),
            },
            auth: AuthConfig {
                oauth: Some(OAuthSession {
                    version: 2,
                    business_base_url: Some("https://www-prepare.ycloud.com".into()),
                    token_endpoint: None,
                    revocation_endpoint: None,
                    refresh_uncertain: false,
                    issuer: "https://open-platform-prepare.ycloud.com".into(),
                    client_id: "ycloud-console-cli".into(),
                    resource: "https://www-prepare.ycloud.com/api/cli".into(),
                    access_expires_at: 1,
                }),
                token_type: "Bearer".into(),
                access_token: String::new(),
                refresh_token: String::new(),
                record_id: String::new(),
                requested_permissions: vec![],
                tenant_id: None,
                user_id: None,
            },
        };
        let command = oauth_login_command(&cli, &config, &["yc.contact.metadata.read".into()]);
        let parsed = Cli::try_parse_from(command.split_whitespace()).unwrap();
        assert_eq!(
            parsed.dashboard_url.as_deref(),
            Some(config.dashboard.base_url.as_str())
        );
        match parsed.command {
            Command::Login(args) => {
                assert!(!args.oauth);
                assert_eq!(
                    args.oauth_issuer.as_deref(),
                    Some("https://open-platform-prepare.ycloud.com")
                );
                assert_eq!(
                    args.oauth_resource.as_deref(),
                    Some("https://www-prepare.ycloud.com/api/cli")
                );
                assert_eq!(args.oauth_client_id.as_deref(), Some("ycloud-console-cli"));
            }
            _ => panic!("expected login command"),
        }
        assert_eq!(shell_arg("client'with space"), "'client'\\''with space'");
    }
}
