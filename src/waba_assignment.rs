use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    cli::WabaAssignmentListArgs,
    config::Config,
    http::{ApiEnvelope, ApiPagination, DashboardClient},
    permissions::WABA_ASSIGNMENT_PERMISSIONS,
};

const PAGE_LIMIT: u32 = 100;
const BATCH_LIMIT: usize = 100;

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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WabaAssignmentReport {
    pub complete: bool,
    pub wabas: Vec<WabaReport>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WabaReport {
    pub waba_id: String,
    pub waba_name: Option<String>,
    pub waba_status: Option<String>,
    pub phone_numbers: Vec<PhoneReport>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
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
    ensure_effective_permissions(client, access_token).await?;
    let phones = fetch_phone_pages(client, access_token, waba_id).await?;
    let phone_ids: Vec<String> = phones
        .iter()
        .map(|phone| phone.phone_number_id.clone())
        .collect();
    let mut assignments = Vec::new();
    let mut rules = Vec::new();
    for batch in phone_ids.chunks(BATCH_LIMIT) {
        assignments.extend(fetch_assignment_pages(client, access_token, batch).await?);
        rules.extend(fetch_rule_pages(client, access_token, batch).await?);
    }
    Ok(assemble_report(phones, assignments, rules))
}

async fn ensure_effective_permissions(client: &DashboardClient, access_token: &str) -> Result<()> {
    let identity = client
        .whoami(access_token)
        .await?
        .require_data("whoami permission preflight")?;
    let effective: BTreeSet<&str> = identity
        .effective_permissions
        .iter()
        .map(String::as_str)
        .collect();
    let requested: BTreeSet<&str> = identity
        .requested_permissions
        .iter()
        .map(String::as_str)
        .collect();
    let missing: Vec<&str> = WABA_ASSIGNMENT_PERMISSIONS
        .iter()
        .copied()
        .filter(|permission| !effective.contains(permission))
        .collect();
    if !missing.is_empty() {
        let not_requested = missing
            .iter()
            .copied()
            .filter(|permission| !requested.contains(permission))
            .collect::<Vec<_>>();
        let ineffective = missing
            .iter()
            .copied()
            .filter(|permission| requested.contains(permission))
            .collect::<Vec<_>>();
        let not_requested_label = if not_requested.is_empty() {
            "-".to_string()
        } else {
            not_requested.join(",")
        };
        let ineffective_label = if ineffective.is_empty() {
            "-".to_string()
        } else {
            ineffective.join(",")
        };
        bail!(
            "WABA assignment requires all three effective permissions; not requested: {}; requested but ineffective: {}",
            not_requested_label,
            ineffective_label,
        );
    }
    Ok(())
}

async fn fetch_phone_pages(
    client: &DashboardClient,
    access_token: &str,
    waba_id: Option<&str>,
) -> Result<Vec<PhoneNumber>> {
    let mut records = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    loop {
        let envelope = client
            .whatsapp_phone_numbers(access_token, waba_id, cursor.as_deref(), PAGE_LIMIT)
            .await?;
        let (data, next) = page(envelope, "whatsapp/phone-numbers")?;
        records.extend(data);
        let Some(next_cursor) = next else {
            break;
        };
        if !seen.insert(next_cursor.clone()) {
            bail!("whatsapp/phone-numbers returned a repeated cursor");
        }
        cursor = Some(next_cursor);
    }
    Ok(records)
}

async fn fetch_assignment_pages(
    client: &DashboardClient,
    access_token: &str,
    phone_number_ids: &[String],
) -> Result<Vec<PhoneAssignment>> {
    let mut records = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    loop {
        let envelope = client
            .inbox_phone_assignments(
                access_token,
                &PhoneIdsSearchRequest {
                    phone_number_ids,
                    cursor: cursor.as_deref(),
                    limit: PAGE_LIMIT,
                },
            )
            .await?;
        let (data, next) = page(envelope, "inbox/phone-assignments/search")?;
        records.extend(data);
        let Some(next_cursor) = next else {
            break;
        };
        if !seen.insert(next_cursor.clone()) {
            bail!("inbox/phone-assignments/search returned a repeated cursor");
        }
        cursor = Some(next_cursor);
    }
    Ok(records)
}

async fn fetch_rule_pages(
    client: &DashboardClient,
    access_token: &str,
    phone_number_ids: &[String],
) -> Result<Vec<AssignmentRule>> {
    let mut records = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    loop {
        let envelope = client
            .inbox_assignment_rules(
                access_token,
                &PhoneIdsSearchRequest {
                    phone_number_ids,
                    cursor: cursor.as_deref(),
                    limit: PAGE_LIMIT,
                },
            )
            .await?;
        let (data, next) = page(envelope, "inbox/assignment-rules/search")?;
        records.extend(data);
        let Some(next_cursor) = next else {
            break;
        };
        if !seen.insert(next_cursor.clone()) {
            bail!("inbox/assignment-rules/search returned a repeated cursor");
        }
        cursor = Some(next_cursor);
    }
    Ok(records)
}

fn page<T>(envelope: ApiEnvelope<Vec<T>>, operation: &str) -> Result<(Vec<T>, Option<String>)> {
    let pagination = envelope
        .pagination
        .context(format!("{operation} response omitted pagination"))?;
    let data = envelope
        .data
        .with_context(|| format!("{operation} response missing data"))?;
    Ok((data, next_cursor(pagination, operation)?))
}

fn next_cursor(pagination: ApiPagination, operation: &str) -> Result<Option<String>> {
    if pagination.has_more {
        let cursor = pagination
            .next_cursor
            .filter(|value| !value.trim().is_empty())
            .context(format!(
                "{operation} response set hasMore=true without nextCursor"
            ))?;
        Ok(Some(cursor))
    } else {
        Ok(None)
    }
}

fn assemble_report(
    phones: Vec<PhoneNumber>,
    assignments: Vec<PhoneAssignment>,
    rules: Vec<AssignmentRule>,
) -> WabaAssignmentReport {
    let phones = phones
        .into_iter()
        .map(|phone| {
            (
                (phone.waba_id.clone(), phone.phone_number_id.clone()),
                phone,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let assignments = assignments
        .into_iter()
        .map(|assignment| (assignment_key(&assignment), assignment))
        .collect::<BTreeMap<_, _>>();
    let rules = rules
        .into_iter()
        .map(|rule| (rule_key(&rule), rule))
        .collect::<BTreeMap<_, _>>();

    let mut wabas: BTreeMap<String, WabaReport> = BTreeMap::new();
    for (_, phone) in phones {
        let phone_id = phone.phone_number_id.clone();
        let waba_id = phone.waba_id.clone();
        let phone_assignments = assignments
            .range(assignment_range_start(&phone_id)..=assignment_range_end(&phone_id))
            .map(|(_, value)| value.clone())
            .collect();
        let phone_rules = rules
            .range(rule_range_start(&phone_id)..=rule_range_end(&phone_id))
            .map(|(_, value)| value.clone())
            .collect();
        let entry = wabas.entry(waba_id.clone()).or_insert_with(|| WabaReport {
            waba_id,
            waba_name: phone.waba_name.clone(),
            waba_status: phone.waba_status.clone(),
            phone_numbers: Vec::new(),
        });
        entry.phone_numbers.push(PhoneReport {
            phone,
            assignments: phone_assignments,
            assignment_rules: phone_rules,
        });
    }
    WabaAssignmentReport {
        complete: true,
        wabas: wabas.into_values().collect(),
    }
}

type AssignmentKey = (String, String, String, String, String);
type RuleKey = (String, String, u32, String);

fn assignment_key(value: &PhoneAssignment) -> AssignmentKey {
    (
        value.phone_number_id.clone(),
        value.assignment_source.clone(),
        value.team_id.clone().unwrap_or_default(),
        value.agent_id.clone().unwrap_or_default(),
        value.relation_id.clone().unwrap_or_default(),
    )
}

fn assignment_range_start(phone_number_id: &str) -> AssignmentKey {
    (
        phone_number_id.to_string(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    )
}

fn assignment_range_end(phone_number_id: &str) -> AssignmentKey {
    (
        phone_number_id.to_string(),
        "\u{10ffff}".to_string(),
        "\u{10ffff}".to_string(),
        "\u{10ffff}".to_string(),
        "\u{10ffff}".to_string(),
    )
}

fn rule_key(value: &AssignmentRule) -> RuleKey {
    (
        value.phone_number_id.clone(),
        value.mode.clone(),
        value.sequence,
        value.node_id.clone(),
    )
}

fn rule_range_start(phone_number_id: &str) -> RuleKey {
    (phone_number_id.to_string(), String::new(), 0, String::new())
}

fn rule_range_end(phone_number_id: &str) -> RuleKey {
    (
        phone_number_id.to_string(),
        "\u{10ffff}".to_string(),
        u32::MAX,
        "\u{10ffff}".to_string(),
    )
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
    use crate::http::InvocationMode;
    use wiremock::{
        matchers::{body_json, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[test]
    fn report_is_grouped_sorted_and_deduplicated_without_hiding_unknowns() {
        let phone_a = phone("waba-2", "phone-2");
        let phone_b = phone("waba-1", "phone-1");
        let unknown = PhoneAssignment {
            phone_number_id: "phone-1".to_string(),
            relation_id: Some("relation-1".to_string()),
            assignment_source: "DIRECT".to_string(),
            team_id: None,
            team_name: None,
            team_status: None,
            agent_id: Some("missing-agent".to_string()),
            agent_name: None,
            agent_email: None,
            member_status: "UNKNOWN".to_string(),
            resolution_status: "UNKNOWN".to_string(),
        };

        let report = assemble_report(
            vec![phone_a, phone_b.clone(), phone_b],
            vec![unknown.clone(), unknown],
            Vec::new(),
        );

        assert!(report.complete);
        assert_eq!(report.wabas[0].waba_id, "waba-1");
        assert_eq!(report.wabas[1].waba_id, "waba-2");
        assert_eq!(report.wabas[0].phone_numbers.len(), 1);
        assert_eq!(report.wabas[0].phone_numbers[0].assignments.len(), 1);
        assert_eq!(
            report.wabas[0].phone_numbers[0].assignments[0]
                .agent_id
                .as_deref(),
            Some("missing-agent")
        );
        assert_eq!(
            report.wabas[0].phone_numbers[0].assignments[0].resolution_status,
            "UNKNOWN"
        );
    }

    #[test]
    fn table_identifies_assignment_source_team_email_and_status() {
        let mut assignment = PhoneAssignment {
            phone_number_id: "phone-1".to_string(),
            relation_id: Some("relation-1".to_string()),
            assignment_source: "TEAM".to_string(),
            team_id: Some("team-1".to_string()),
            team_name: Some("Support".to_string()),
            team_status: Some("ACTIVE".to_string()),
            agent_id: Some("agent-1".to_string()),
            agent_name: Some("Alice".to_string()),
            agent_email: Some("alice@example.com".to_string()),
            member_status: "ACTIVE".to_string(),
            resolution_status: "RESOLVED".to_string(),
        };
        let mut report = assemble_report(vec![phone("waba-1", "phone-1")], vec![], vec![]);
        report.wabas[0].phone_numbers[0]
            .assignments
            .push(assignment.clone());

        let table = render_table(&report);

        assert!(table.contains("TEAM Support: Alice <alice@example.com> [ACTIVE; RESOLVED]"));
        assignment.agent_email = None;
        assert!(!assignment_label(&assignment).contains('@'));
    }

    #[test]
    fn pagination_requires_cursor_when_more_data_exists() {
        let pagination = ApiPagination {
            next_cursor: None,
            has_more: true,
            total: Some(100),
        };

        assert!(next_cursor(pagination, "operation")
            .unwrap_err()
            .to_string()
            .contains("without nextCursor"));
    }

    #[test]
    fn table_keeps_unassigned_rule_action_visible() {
        let rule: AssignmentRule = serde_json::from_value(serde_json::json!({
            "phoneNumberId": "phone-1",
            "mode": "BASIC",
            "sequence": 0,
            "nodeId": "basic",
            "nodeType": "ACTION",
            "assignType": "UNASSIGNED",
            "conditions": [],
            "targets": []
        }))
        .unwrap();

        assert_eq!(rule_label(&rule), "BASIC:0:ACTION:UNASSIGNED");
    }

    #[tokio::test]
    async fn fetch_report_pages_then_joins_all_atomic_reads() {
        let server = MockServer::start().await;
        mount_permission_preflight(&server, WABA_ASSIGNMENT_PERMISSIONS.to_vec()).await;
        Mock::given(method("GET"))
            .and(path("/api/cli/v1/whatsapp/phone-numbers"))
            .respond_with(|request: &wiremock::Request| {
                let second_page = request
                    .url
                    .query_pairs()
                    .any(|(name, value)| name == "cursor" && value == "phone-next");
                let (phone_id, next_cursor, has_more) = if second_page {
                    ("phone-2", serde_json::Value::Null, false)
                } else {
                    ("phone-1", serde_json::json!("phone-next"), true)
                };
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "code": 0,
                    "data": [{
                        "wabaId": "waba-1",
                        "phoneNumberId": phone_id,
                        "displayPhoneNumber": format!("+{phone_id}"),
                        "status": "CONNECTED"
                    }],
                    "pagination": {
                        "nextCursor": next_cursor,
                        "hasMore": has_more,
                        "total": 2
                    }
                }))
            })
            .expect(2)
            .mount(&server)
            .await;
        let search_body = serde_json::json!({
            "phoneNumberIds": ["phone-1", "phone-2"],
            "limit": 100
        });
        Mock::given(method("POST"))
            .and(path("/api/cli/v1/inbox/phone-assignments/search"))
            .and(body_json(search_body.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "data": [{
                    "phoneNumberId": "phone-1",
                    "relationId": "relation-1",
                    "assignmentSource": "DIRECT",
                    "agentId": "missing-agent",
                    "memberStatus": "UNKNOWN",
                    "resolutionStatus": "UNKNOWN"
                }],
                "pagination": {"hasMore": false, "total": 1}
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/cli/v1/inbox/assignment-rules/search"))
            .and(body_json(search_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "data": [],
                "pagination": {"hasMore": false, "total": 0}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client =
            DashboardClient::new_with_mode(server.uri(), InvocationMode::Automation).unwrap();
        let report = fetch_report(&client, "YCLI.access", None).await.unwrap();

        assert!(report.complete);
        assert_eq!(report.wabas.len(), 1);
        assert_eq!(report.wabas[0].phone_numbers.len(), 2);
        assert_eq!(report.wabas[0].phone_numbers[0].assignments.len(), 1);
        assert_eq!(
            report.wabas[0].phone_numbers[0].assignments[0].resolution_status,
            "UNKNOWN"
        );
    }

    #[tokio::test]
    async fn fetch_report_fails_atomically_when_rule_read_fails() {
        let server = MockServer::start().await;
        mount_permission_preflight(&server, WABA_ASSIGNMENT_PERMISSIONS.to_vec()).await;
        Mock::given(method("GET"))
            .and(path("/api/cli/v1/whatsapp/phone-numbers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "data": [{
                    "wabaId": "waba-1",
                    "phoneNumberId": "phone-1",
                    "status": "CONNECTED"
                }],
                "pagination": {"hasMore": false, "total": 1}
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/cli/v1/inbox/phone-assignments/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "data": [],
                "pagination": {"hasMore": false, "total": 0}
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/cli/v1/inbox/assignment-rules/search"))
            .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
                "code": 503,
                "data": null,
                "error": {
                    "code": "downstream_unavailable",
                    "message": "retry later",
                    "retryable": true
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client =
            DashboardClient::new_with_mode(server.uri(), InvocationMode::Automation).unwrap();
        let error = fetch_report(&client, "YCLI.access", None)
            .await
            .expect_err("a partial report must not be returned");

        assert!(error.to_string().contains("downstream_unavailable"));
    }

    #[tokio::test]
    async fn fetch_report_requires_all_three_effective_permissions_before_inventory() {
        let server = MockServer::start().await;
        mount_permission_preflight(&server, vec!["yc.whatsapp.phone.read"]).await;

        let client =
            DashboardClient::new_with_mode(server.uri(), InvocationMode::Automation).unwrap();
        let error = fetch_report(&client, "YCLI.access", None)
            .await
            .expect_err("missing effective permissions must stop the scenario");

        assert!(error.to_string().contains("yc.inbox.phone-assignment.read"));
        assert!(error.to_string().contains("yc.inbox.assignment-rule.read"));
    }

    async fn mount_permission_preflight(server: &MockServer, permissions: Vec<&str>) {
        Mock::given(method("GET"))
            .and(path("/api/cli/v1/whoami"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "data": {
                    "userId": "user-1",
                    "tenantId": "tenant-1",
                    "requestedPermissions": permissions,
                    "effectivePermissions": permissions
                }
            })))
            .expect(1)
            .mount(server)
            .await;
    }

    fn phone(waba_id: &str, phone_number_id: &str) -> PhoneNumber {
        PhoneNumber {
            waba_id: waba_id.to_string(),
            waba_name: Some(format!("{waba_id}-name")),
            waba_status: Some("APPROVED".to_string()),
            phone_number_id: phone_number_id.to_string(),
            meta_phone_number_id: None,
            display_phone_number: Some(format!("+{phone_number_id}")),
            display_name: None,
            status: Some("CONNECTED".to_string()),
            quality_rating: None,
        }
    }
}
