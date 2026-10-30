use std::path::Path;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::{cli::WabaAssignmentListArgs, config::Config, http::DashboardClient};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PhoneNumber {
    pub waba_id: String,
    #[serde(default)]
    pub waba_name: Option<String>,
    #[serde(default)]
    pub waba_status: Option<String>,
    pub phone_number_id: String,
    #[serde(default)]
    pub meta_phone_number_id: Option<String>,
    #[serde(default)]
    pub display_phone_number: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub quality_rating: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PhoneAssignment {
    pub phone_number_id: String,
    #[serde(default)]
    pub relation_id: Option<String>,
    pub assignment_source: String,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub team_name: Option<String>,
    #[serde(default)]
    pub team_status: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_name: Option<String>,
    #[serde(default)]
    pub agent_email: Option<String>,
    pub member_status: String,
    pub resolution_status: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentCondition {
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    pub range: Vec<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub handle: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentTarget {
    pub r#type: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentRule {
    pub phone_number_id: String,
    pub mode: String,
    pub sequence: u32,
    pub node_id: String,
    pub node_type: String,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub default_next: Option<String>,
    #[serde(default)]
    pub basic_assign_type: Option<String>,
    #[serde(default)]
    pub assign_owner: Option<bool>,
    #[serde(default)]
    pub assign_last_assigned: Option<bool>,
    #[serde(default)]
    pub auto_assign_unassigned: Option<bool>,
    #[serde(default)]
    pub online_only: Option<bool>,
    #[serde(default)]
    pub assign_type: Option<String>,
    #[serde(default)]
    pub action_type: Option<String>,
    #[serde(default)]
    pub action_rule: Option<String>,
    #[serde(default)]
    pub conditions: Vec<AssignmentCondition>,
    #[serde(default)]
    pub targets: Vec<AssignmentTarget>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneIdsSearchRequest<'a> {
    pub phone_number_ids: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<&'a str>,
    pub limit: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WabaAssignmentReport {
    pub complete: bool,
    pub wabas: Vec<WabaReport>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WabaReport {
    pub waba_id: String,
    pub waba_name: Option<String>,
    pub waba_status: Option<String>,
    pub phone_numbers: Vec<PhoneReport>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PhoneReport {
    #[serde(flatten)]
    pub phone: PhoneNumber,
    pub assignments: Vec<PhoneAssignment>,
    pub assignment_rules: Vec<AssignmentRule>,
}

pub async fn list(
    client: &DashboardClient,
    config_path: &Path,
    args: WabaAssignmentListArgs,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let report = fetch_report(client, &config.auth.access_token, args.waba_id.as_deref()).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_table(&report));
    }
    Ok(())
}

async fn fetch_report(
    client: &DashboardClient,
    access_token: &str,
    waba_id: Option<&str>,
) -> Result<WabaAssignmentReport> {
    let report = client
        .waba_assignment_report(access_token, waba_id)
        .await?
        .require_data("WABA assignment report")?;
    if !report.complete {
        bail!("WABA assignment report is incomplete; no complete result can be displayed");
    }
    Ok(report)
}

fn render_table(report: &WabaAssignmentReport) -> String {
    let mut output =
        "WABA\tPHONE_NUMBER\tPHONE_STATUS\tASSIGNMENTS\tASSIGNMENT_RULES\n".to_string();
    for waba in &report.wabas {
        for phone in &waba.phone_numbers {
            let assignments = phone
                .assignments
                .iter()
                .map(assignment_label)
                .collect::<Vec<_>>()
                .join("; ");
            let rules = phone
                .assignment_rules
                .iter()
                .map(rule_label)
                .collect::<Vec<_>>()
                .join("; ");
            output.push_str(&format!(
                "{} ({})\t{}\t{}\t{}\t{}\n",
                text(waba.waba_name.as_deref()),
                waba.waba_id,
                text(phone.phone.display_phone_number.as_deref()),
                text(phone.phone.status.as_deref()),
                if assignments.is_empty() {
                    "-"
                } else {
                    &assignments
                },
                if rules.is_empty() { "-" } else { &rules },
            ));
        }
    }
    output
}

fn assignment_label(value: &PhoneAssignment) -> String {
    let subject = value
        .agent_name
        .as_deref()
        .or(value.agent_id.as_deref())
        .unwrap_or("UNKNOWN");
    let email = value
        .agent_email
        .as_deref()
        .map(|email| format!(" <{email}>"))
        .unwrap_or_default();
    let team = value
        .team_name
        .as_deref()
        .or(value.team_id.as_deref())
        .map(|team| format!(" {team}"))
        .unwrap_or_default();
    format!(
        "{}{}: {}{} [{}; {}]",
        value.assignment_source, team, subject, email, value.member_status, value.resolution_status
    )
}

fn rule_label(value: &AssignmentRule) -> String {
    let targets = value
        .targets
        .iter()
        .map(|target| {
            format!(
                "{}:{} [{}]",
                target.r#type,
                target.name.as_deref().unwrap_or(&target.id),
                target.status
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let assign_type = value
        .assign_type
        .as_deref()
        .map(|kind| format!(":{kind}"))
        .unwrap_or_default();
    if targets.is_empty() {
        format!(
            "{}:{}:{}{}",
            value.mode, value.sequence, value.node_type, assign_type
        )
    } else {
        format!(
            "{}:{}:{}{} -> {}",
            value.mode, value.sequence, value.node_type, assign_type, targets
        )
    }
}

fn text(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{body_json, method, path},
        Mock, MockServer, ResponseTemplate,
    };
    #[tokio::test]
    async fn fetch_report_uses_shared_endpoint_and_retains_unknown_members() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/cli/v1/whatsapp/waba-assignments/search"))
            .and(body_json(serde_json::json!({"wabaId":"waba-1"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"code":0,"data":{
                "complete":true,"wabas":[{"wabaId":"waba-1","wabaName":"Demo","wabaStatus":null,"phoneNumbers":[{
                    "wabaId":"waba-1","phoneNumberId":"p1","displayPhoneNumber":"+123",
                    "assignments":[{"phoneNumberId":"p1","assignmentSource":"DIRECT","memberStatus":"UNKNOWN","resolutionStatus":"UNKNOWN"}],"assignmentRules":[]
                }]}]}}))).expect(1).mount(&server).await;
        let client = DashboardClient::new(server.uri()).unwrap();
        let report = fetch_report(&client, "fixture", Some("waba-1"))
            .await
            .unwrap();
        assert!(render_table(&report).contains("UNKNOWN"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn incomplete_report_is_not_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"code":0,"data":{"complete":false,"wabas":[]}}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(fetch_report(
            &DashboardClient::new(server.uri()).unwrap(),
            "fixture",
            None
        )
        .await
        .is_err());
    }
}
